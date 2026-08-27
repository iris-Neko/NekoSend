use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    net::{Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    PROTOCOL_VERSION,
    domain::{DeviceId, EntryId, EventId, TransferEntryKind, TransferId},
    events::{self, CoreEventKind, TransferProgressEventDto},
    platform_io::{commit_document, open_document},
    protocol::{FrameError, MAX_CONTROL_FRAME_BYTES, read_json_frame, write_json_frame},
    storage::{LocalProfile, Storage, StorageError, TransferEntryRecord},
};

use super::CONTROL_PORT;

const DATA_BUFFER_BYTES: usize = 1024 * 1024;
const DATA_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const OFFSET_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const OFFSET_PERSIST_BYTES: u64 = 64 * 1024 * 1024;
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(250);
const MAX_GLOBAL_DATA_CONNECTIONS: usize = 4;
const MAX_PEER_DATA_CONNECTIONS: usize = 2;

static DATA_SLOTS: LazyLock<Mutex<HashMap<DeviceId, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ACTIVE_DATA_STREAMS: LazyLock<Mutex<HashMap<TransferId, Arc<TcpStream>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Error)]
pub enum DataError {
    #[error("data connection I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("data frame failed: {0}")]
    Frame(#[from] FrameError),
    #[error("data JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("data database operation failed: {0}")]
    Storage(#[from] StorageError),
    #[error("data protocol validation failed: {0}")]
    Protocol(&'static str),
    #[error("source file changed after the offer")]
    SourceChanged,
    #[error("source or destination permission was lost")]
    PermissionLost,
    #[error("receive destination has insufficient free space")]
    NotEnoughSpace,
    #[error("receive destination path is no longer usable")]
    InvalidPath,
    #[error("data connection limit reached")]
    Busy,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DataHello {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    connection_id: EventId,
    sender_device_id: DeviceId,
    transfer_id: TransferId,
    protocol: u16,
}

#[derive(Debug, Serialize, Deserialize)]
struct DataHelloAck {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    connection_id: EventId,
    transfer_id: TransferId,
    accepted_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DataEntry {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    transfer_id: TransferId,
    entry_id: EntryId,
    offset: u64,
    data_length: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DataEnd {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    transfer_id: TransferId,
    entry_count: u32,
    total_bytes_sent: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DataComplete {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    transfer_id: TransferId,
    total_bytes_saved: u64,
    completed_at_ms: i64,
}

struct DataSlot {
    peer: DeviceId,
}

impl DataSlot {
    fn reserve(peer: DeviceId) -> Result<Self, DataError> {
        let mut slots = DATA_SLOTS.lock().unwrap_or_else(|error| error.into_inner());
        if !reserve_data_slot(&mut slots, peer) {
            return Err(DataError::Busy);
        }
        Ok(Self { peer })
    }
}

fn reserve_data_slot(slots: &mut HashMap<DeviceId, usize>, peer: DeviceId) -> bool {
    let total = slots.values().sum::<usize>();
    let peer_count = slots.get(&peer).copied().unwrap_or_default();
    if total >= MAX_GLOBAL_DATA_CONNECTIONS || peer_count >= MAX_PEER_DATA_CONNECTIONS {
        return false;
    }
    slots.insert(peer, peer_count + 1);
    true
}

impl Drop for DataSlot {
    fn drop(&mut self) {
        let mut slots = DATA_SLOTS.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = slots.get_mut(&self.peer) {
            *count -= 1;
            if *count == 0 {
                slots.remove(&self.peer);
            }
        }
    }
}

struct ActiveDataStream {
    transfer_id: TransferId,
    stream: Arc<TcpStream>,
}

impl ActiveDataStream {
    fn register(transfer_id: TransferId, stream: &TcpStream) -> io::Result<Self> {
        let stream = Arc::new(stream.try_clone()?);
        ACTIVE_DATA_STREAMS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(transfer_id, Arc::clone(&stream));
        Ok(Self {
            transfer_id,
            stream,
        })
    }
}

impl Drop for ActiveDataStream {
    fn drop(&mut self) {
        let mut active = ACTIVE_DATA_STREAMS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if active
            .get(&self.transfer_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.stream))
        {
            active.remove(&self.transfer_id);
        }
    }
}

pub(crate) fn stop_active_transfer(transfer_id: TransferId) {
    let stream = ACTIVE_DATA_STREAMS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&transfer_id)
        .cloned();
    if let Some(stream) = stream {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

pub(crate) fn stop_all_active_transfers() {
    let streams = ACTIVE_DATA_STREAMS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for stream in streams {
        let _ = stream.shutdown(Shutdown::Both);
    }
}

pub(crate) fn schedule_ready_sends(
    profile: &LocalProfile,
    database_path: &Path,
    stop: &Arc<AtomicBool>,
) {
    let Ok(storage) = Storage::open(database_path) else {
        return;
    };
    let Ok(candidates) = storage.schedulable_send_transfers(MAX_GLOBAL_DATA_CONNECTIONS as u32)
    else {
        return;
    };
    for (transfer_id, ip) in candidates {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let Ok((transfer, _)) = storage.get_transfer(transfer_id) else {
            continue;
        };
        let Ok(slot) = DataSlot::reserve(transfer.peer_device_id) else {
            continue;
        };
        let Ok(true) = storage.claim_send_transfer(transfer_id) else {
            continue;
        };
        let profile = profile.clone();
        let database_path = database_path.to_path_buf();
        let stop = Arc::clone(stop);
        let spawn = thread::Builder::new()
            .name(format!("lan-chat-data-send-{transfer_id}"))
            .spawn(move || {
                let _slot = slot;
                if let Err(error) = send_transfer(&profile, &database_path, transfer_id, &ip, &stop)
                {
                    if let Ok(mut storage) = Storage::open(&database_path) {
                        match error {
                            DataError::SourceChanged => {
                                let _ = storage
                                    .fail_send_transfer_source(transfer_id, "source_changed");
                            }
                            DataError::PermissionLost => {
                                let _ = storage
                                    .fail_send_transfer_source(transfer_id, "permission_lost");
                            }
                            _ => {
                                let _ = storage.interrupt_transfer(transfer_id);
                            }
                        }
                    }
                    record_data_error(&database_path, &format!("send {transfer_id}: {error}"));
                }
            });
        if spawn.is_err() {
            let _ = storage.interrupt_transfer(transfer_id);
        }
    }
}

pub(crate) fn handle_data_connection(
    first_frame: Value,
    stream: &mut TcpStream,
    profile: &LocalProfile,
    database_path: &Path,
    stop: &Arc<AtomicBool>,
) -> Result<(), DataError> {
    let hello: DataHello = serde_json::from_value(first_frame)?;
    if hello.version != PROTOCOL_VERSION
        || hello.frame_type != "data_hello"
        || hello.protocol != PROTOCOL_VERSION
        || hello.sender_device_id == profile.device_id
    {
        return Err(DataError::Protocol("invalid data_hello"));
    }
    let _slot = DataSlot::reserve(hello.sender_device_id)?;
    configure_data_stream(stream)?;
    let mut storage = Storage::open(database_path)?;
    if !storage.claim_receive_transfer(hello.transfer_id, hello.sender_device_id)? {
        return Err(DataError::Protocol("transfer is not ready for data"));
    }
    let _active_stream = ActiveDataStream::register(hello.transfer_id, stream)?;
    write_json_frame(
        stream,
        &DataHelloAck {
            version: PROTOCOL_VERSION,
            frame_type: "data_hello_ack".to_owned(),
            connection_id: hello.connection_id,
            transfer_id: hello.transfer_id,
            accepted_at_ms: unix_time_ms(),
        },
    )?;
    let result = receive_transfer(stream, &mut storage, profile, hello.transfer_id, stop);
    if let Err(error) = &result {
        match error {
            DataError::NotEnoughSpace => {
                let _ =
                    storage.fail_receive_transfer(profile, hello.transfer_id, "not_enough_space");
            }
            DataError::PermissionLost => {
                let _ =
                    storage.fail_receive_transfer(profile, hello.transfer_id, "permission_lost");
            }
            DataError::InvalidPath => {
                let _ = storage.fail_receive_transfer(profile, hello.transfer_id, "invalid_path");
            }
            _ => {
                let _ = storage.interrupt_transfer(hello.transfer_id);
            }
        }
    }
    result
}

fn send_transfer(
    profile: &LocalProfile,
    database_path: &Path,
    transfer_id: TransferId,
    ip: &str,
    stop: &Arc<AtomicBool>,
) -> Result<(), DataError> {
    let address = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::from_str(ip).map_err(|_| DataError::Protocol("invalid peer IPv4"))?,
        CONTROL_PORT,
    ));
    send_transfer_to(profile, database_path, transfer_id, address, stop)
}

fn send_transfer_to(
    profile: &LocalProfile,
    database_path: &Path,
    transfer_id: TransferId,
    address: SocketAddr,
    stop: &Arc<AtomicBool>,
) -> Result<(), DataError> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    configure_data_stream(&stream)?;
    let _active_stream = ActiveDataStream::register(transfer_id, &stream)?;
    let connection_id = EventId::generate();
    write_json_frame(
        &mut stream,
        &DataHello {
            version: PROTOCOL_VERSION,
            frame_type: "data_hello".to_owned(),
            connection_id,
            sender_device_id: profile.device_id,
            transfer_id,
            protocol: PROTOCOL_VERSION,
        },
    )?;
    let ack: DataHelloAck = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES)?;
    if ack.version != PROTOCOL_VERSION
        || ack.frame_type != "data_hello_ack"
        || ack.connection_id != connection_id
        || ack.transfer_id != transfer_id
    {
        return Err(DataError::Protocol("invalid data_hello_ack"));
    }

    let mut storage = Storage::open(database_path)?;
    let (transfer, entries) = storage.get_transfer(transfer_id)?;
    let mut buffer = vec![0_u8; DATA_BUFFER_BYTES];
    let mut sent_entries = 0_u32;
    let mut total_sent = 0_u64;
    let initial_persisted = entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.persisted_offset)
            .ok_or(DataError::Protocol("persisted byte count overflow"))
    })?;
    let transfer_started_at = Instant::now();
    let mut last_progress_event_at = Instant::now();
    for entry in entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
    {
        ensure_source_unchanged(entry)?;
        if entry.persisted_offset == entry.size {
            continue;
        }
        let data_length = entry.size - entry.persisted_offset;
        write_json_frame(
            &mut stream,
            &DataEntry {
                version: PROTOCOL_VERSION,
                frame_type: "data_entry".to_owned(),
                transfer_id,
                entry_id: entry.entry_id,
                offset: entry.persisted_offset,
                data_length,
            },
        )?;
        let mut file = open_source_file(entry)?;
        file.seek(SeekFrom::Start(entry.persisted_offset))
            .map_err(classify_source_io)?;
        let mut remaining = data_length;
        while remaining > 0 {
            if stop.load(Ordering::Acquire) {
                return Err(DataError::Io(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "core is stopping",
                )));
            }
            let chunk = usize::try_from(remaining.min(DATA_BUFFER_BYTES as u64))
                .map_err(|_| DataError::Protocol("data chunk is too large"))?;
            file.read_exact(&mut buffer[..chunk])
                .map_err(classify_source_io)?;
            stream.write_all(&buffer[..chunk])?;
            remaining -= chunk as u64;
            total_sent = total_sent
                .checked_add(chunk as u64)
                .ok_or(DataError::Protocol("sent byte count overflow"))?;
            if last_progress_event_at.elapsed() >= PROGRESS_EVENT_INTERVAL {
                publish_transfer_progress(
                    transfer_id,
                    "transferring",
                    initial_persisted.saturating_add(total_sent),
                    transfer.total_size,
                    total_sent,
                    transfer_started_at.elapsed(),
                    Some(entry.relative_path.clone()),
                );
                last_progress_event_at = Instant::now();
            }
        }
        sent_entries += 1;
    }
    for entry in entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
    {
        if let Err(error) = ensure_source_unchanged(entry) {
            let _ = write_json_frame(
                &mut stream,
                &serde_json::json!({
                    "version": PROTOCOL_VERSION,
                    "type": "transfer_source_changed",
                    "transfer_id": transfer_id,
                }),
            );
            return Err(error);
        }
    }
    write_json_frame(
        &mut stream,
        &DataEnd {
            version: PROTOCOL_VERSION,
            frame_type: "data_end".to_owned(),
            transfer_id,
            entry_count: sent_entries,
            total_bytes_sent: total_sent,
        },
    )?;
    stream.shutdown(Shutdown::Write)?;
    let complete: DataComplete = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES)?;
    if complete.version != PROTOCOL_VERSION
        || complete.frame_type != "data_complete"
        || complete.transfer_id != transfer_id
        || complete.total_bytes_saved != transfer.total_size
    {
        return Err(DataError::Protocol("invalid data_complete"));
    }
    storage.complete_send_transfer(transfer_id)?;
    publish_transfer_progress(
        transfer_id,
        "completed",
        transfer.total_size,
        transfer.total_size,
        total_sent,
        transfer_started_at.elapsed(),
        None,
    );
    Ok(())
}

fn receive_transfer(
    stream: &mut TcpStream,
    storage: &mut Storage,
    profile: &LocalProfile,
    transfer_id: TransferId,
    stop: &Arc<AtomicBool>,
) -> Result<(), DataError> {
    let (transfer, entries) = storage.get_transfer(transfer_id)?;
    let expected = entries
        .iter()
        .filter(|entry| {
            entry.entry_kind == TransferEntryKind::File && entry.persisted_offset < entry.size
        })
        .collect::<Vec<_>>();
    let expected_bytes = expected.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.size - entry.persisted_offset)
            .ok_or(DataError::Protocol("expected byte count overflow"))
    })?;
    let mut next_entry = 0_usize;
    let mut received_bytes = 0_u64;
    let mut buffer = vec![0_u8; DATA_BUFFER_BYTES];
    let initial_persisted = transfer.total_size.saturating_sub(expected_bytes);
    let transfer_started_at = Instant::now();
    let mut last_progress_event_at = Instant::now();
    loop {
        let value: Value = read_json_frame(stream, MAX_CONTROL_FRAME_BYTES)?;
        match value.get("type").and_then(Value::as_str) {
            Some("data_entry") => {
                let header: DataEntry = serde_json::from_value(value)?;
                let entry = expected
                    .get(next_entry)
                    .ok_or(DataError::Protocol("unexpected data_entry"))?;
                if header.version != PROTOCOL_VERSION
                    || header.frame_type != "data_entry"
                    || header.transfer_id != transfer_id
                    || header.entry_id != entry.entry_id
                    || header.offset != entry.persisted_offset
                    || header.data_length != entry.size - entry.persisted_offset
                {
                    return Err(DataError::Protocol("data_entry does not match manifest"));
                }
                receive_entry(
                    stream,
                    storage,
                    entry,
                    header.data_length,
                    &mut buffer,
                    stop,
                    ReceiveProgress {
                        last_event_at: &mut last_progress_event_at,
                        persisted_base: initial_persisted.saturating_add(received_bytes),
                        transferred_before_entry: received_bytes,
                        total_size: transfer.total_size,
                        started_at: transfer_started_at,
                    },
                )?;
                received_bytes = received_bytes
                    .checked_add(header.data_length)
                    .ok_or(DataError::Protocol("received byte count overflow"))?;
                next_entry += 1;
            }
            Some("data_end") => {
                let end: DataEnd = serde_json::from_value(value)?;
                if end.version != PROTOCOL_VERSION
                    || end.frame_type != "data_end"
                    || end.transfer_id != transfer_id
                    || end.entry_count as usize != expected.len()
                    || end.total_bytes_sent != expected_bytes
                    || next_entry != expected.len()
                    || received_bytes != expected_bytes
                {
                    return Err(DataError::Protocol("invalid data_end"));
                }
                publish_received_files(storage, &entries)?;
                storage.complete_receive_transfer(profile, transfer_id)?;
                publish_transfer_progress(
                    transfer_id,
                    "completed",
                    transfer.total_size,
                    transfer.total_size,
                    received_bytes,
                    transfer_started_at.elapsed(),
                    None,
                );
                if let Some(image) = storage.completed_clipboard_image(transfer_id)? {
                    crate::platform_io::enqueue_clipboard_image_write(
                        &image.reference,
                        serde_json::json!({
                            "origin_device_id": image.metadata.origin_device_id,
                            "clipboard_sequence": image.metadata.clipboard_sequence,
                            "content_fingerprint": image.metadata.content_fingerprint,
                        })
                        .to_string(),
                    );
                }
                write_json_frame(
                    stream,
                    &DataComplete {
                        version: PROTOCOL_VERSION,
                        frame_type: "data_complete".to_owned(),
                        transfer_id,
                        total_bytes_saved: transfer.total_size,
                        completed_at_ms: unix_time_ms(),
                    },
                )?;
                return Ok(());
            }
            Some("transfer_source_changed") => {
                return Err(DataError::SourceChanged);
            }
            _ => return Err(DataError::Protocol("unexpected data connection frame")),
        }
    }
}

struct ReceiveProgress<'a> {
    last_event_at: &'a mut Instant,
    persisted_base: u64,
    transferred_before_entry: u64,
    total_size: u64,
    started_at: Instant,
}

fn receive_entry(
    stream: &mut TcpStream,
    storage: &mut Storage,
    entry: &TransferEntryRecord,
    data_length: u64,
    buffer: &mut [u8],
    stop: &Arc<AtomicBool>,
    progress: ReceiveProgress<'_>,
) -> Result<(), DataError> {
    let partial = entry
        .partial_ref
        .as_deref()
        .ok_or(DataError::Protocol("receive entry has no partial path"))?;
    let mut file = open_receive_file(partial).map_err(classify_receive_io)?;
    prepare_receive_file_length(&file, partial, entry.persisted_offset)?;
    file.seek(SeekFrom::Start(entry.persisted_offset))
        .map_err(classify_receive_io)?;
    let mut offset = entry.persisted_offset;
    let mut remaining = data_length;
    let mut last_persisted_offset = offset;
    let mut last_persisted_at = Instant::now();
    while remaining > 0 {
        if stop.load(Ordering::Acquire) {
            persist_safe_offset(storage, entry, &file, offset)?;
            return Err(DataError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "core is stopping",
            )));
        }
        let chunk = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| DataError::Protocol("data chunk is too large"))?;
        let read = match stream.read(&mut buffer[..chunk]) {
            Ok(0) => {
                persist_safe_offset(storage, entry, &file, offset)?;
                return Err(DataError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "data stream ended before data_length",
                )));
            }
            Ok(read) => read,
            Err(error) => {
                persist_safe_offset(storage, entry, &file, offset)?;
                return Err(DataError::Io(error));
            }
        };
        let mut written = 0_usize;
        while written < read {
            match file.write(&buffer[written..read]) {
                Ok(0) => {
                    persist_safe_offset(storage, entry, &file, offset)?;
                    return Err(classify_receive_io(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "receive destination stopped accepting bytes",
                    )));
                }
                Ok(count) => {
                    written += count;
                    offset += count as u64;
                    remaining -= count as u64;
                }
                Err(error) => {
                    let failure = classify_receive_io(error);
                    persist_safe_offset(storage, entry, &file, offset)?;
                    return Err(failure);
                }
            }
        }
        if last_persisted_at.elapsed() >= OFFSET_PERSIST_INTERVAL
            || offset - last_persisted_offset >= OFFSET_PERSIST_BYTES
        {
            persist_safe_offset(storage, entry, &file, offset)?;
            last_persisted_at = Instant::now();
            last_persisted_offset = offset;
        }
        if progress.last_event_at.elapsed() >= PROGRESS_EVENT_INTERVAL {
            let entry_progress = offset.saturating_sub(entry.persisted_offset);
            publish_transfer_progress(
                entry.transfer_id,
                "transferring",
                progress.persisted_base.saturating_add(entry_progress),
                progress.total_size,
                progress
                    .transferred_before_entry
                    .saturating_add(entry_progress),
                progress.started_at.elapsed(),
                Some(entry.relative_path.clone()),
            );
            *progress.last_event_at = Instant::now();
        }
    }
    persist_safe_offset(storage, entry, &file, offset)?;
    Ok(())
}

fn publish_transfer_progress(
    transfer_id: TransferId,
    state: &str,
    persisted_bytes: u64,
    total_size: u64,
    transferred_since_start: u64,
    elapsed: Duration,
    active_entry_relative_path: Option<String>,
) {
    let elapsed_nanos = elapsed.as_nanos();
    let bytes_per_second = (u128::from(transferred_since_start) * 1_000_000_000)
        .checked_div(elapsed_nanos)
        .map(|rate| u64::try_from(rate).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let remaining = total_size.saturating_sub(persisted_bytes);
    let eta_seconds = (bytes_per_second > 0).then(|| remaining.div_ceil(bytes_per_second));
    events::publish_transfer_progress(TransferProgressEventDto {
        transfer_id: transfer_id.to_string(),
        state: state.to_owned(),
        persisted_bytes,
        total_size,
        bytes_per_second,
        eta_seconds,
        active_entry_relative_path,
    });
}

fn persist_safe_offset(
    storage: &mut Storage,
    entry: &TransferEntryRecord,
    mut file: &File,
    offset: u64,
) -> Result<(), DataError> {
    file.flush().map_err(classify_receive_io)?;
    storage.persist_transfer_offset(entry.transfer_id, entry.entry_id, offset)?;
    Ok(())
}

fn publish_received_files(
    storage: &Storage,
    entries: &[TransferEntryRecord],
) -> Result<(), DataError> {
    for entry in entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
    {
        let Some(partial_ref) = entry.partial_ref.as_deref() else {
            validate_published_receive_entry(entry)?;
            continue;
        };
        if is_platform_ref(partial_ref) {
            let final_name = entry
                .destination_ref
                .as_deref()
                .ok_or(DataError::Protocol("SAF receive entry has no final name"))?;
            if final_name.trim().is_empty() {
                return Err(DataError::InvalidPath);
            }
            let file = open_receive_file(partial_ref).map_err(classify_receive_io)?;
            if file.metadata().map_err(classify_receive_io)?.len() != entry.size {
                return Err(DataError::Protocol(
                    "received SAF document has the wrong length",
                ));
            }
        } else {
            let partial = Path::new(partial_ref);
            let destination = Path::new(
                entry
                    .destination_ref
                    .as_deref()
                    .ok_or(DataError::Protocol("receive entry has no destination path"))?,
            );
            let destination_complete = destination
                .metadata()
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == entry.size);
            if destination_complete && !partial.exists() {
                continue;
            }
            if partial.metadata().map_err(classify_receive_io)?.len() != entry.size
                || destination.exists()
            {
                return Err(DataError::InvalidPath);
            }
        }
    }

    for entry in entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
    {
        let Some(partial_ref) = entry.partial_ref.as_deref() else {
            continue;
        };
        let partial = Path::new(partial_ref);
        if is_platform_ref(partial.to_string_lossy().as_ref()) {
            let final_name = entry
                .destination_ref
                .as_deref()
                .ok_or(DataError::Protocol("SAF receive entry has no final name"))?;
            let file = open_receive_file(partial.to_string_lossy().as_ref())
                .map_err(classify_receive_io)?;
            file.sync_all().map_err(classify_receive_io)?;
            drop(file);
            let destination = commit_document(partial.to_string_lossy().as_ref(), final_name)
                .map_err(classify_receive_io)?;
            storage.persist_published_destination(
                entry.transfer_id,
                entry.entry_id,
                &destination,
            )?;
        } else {
            let destination = Path::new(
                entry
                    .destination_ref
                    .as_deref()
                    .ok_or(DataError::Protocol("receive entry has no destination path"))?,
            );
            if destination
                .metadata()
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == entry.size)
                && !partial.exists()
            {
                storage.persist_published_destination(
                    entry.transfer_id,
                    entry.entry_id,
                    destination.to_string_lossy().as_ref(),
                )?;
                continue;
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(partial)
                .map_err(classify_receive_io)?
                .sync_all()
                .map_err(classify_receive_io)?;
            fs::rename(partial, destination).map_err(classify_receive_io)?;
            storage.persist_published_destination(
                entry.transfer_id,
                entry.entry_id,
                destination.to_string_lossy().as_ref(),
            )?;
        }
    }
    Ok(())
}

fn validate_published_receive_entry(entry: &TransferEntryRecord) -> Result<(), DataError> {
    let destination = entry
        .destination_ref
        .as_deref()
        .ok_or(DataError::InvalidPath)?;
    let metadata = if is_platform_ref(destination) {
        open_receive_file(destination)
            .map_err(classify_receive_io)?
            .metadata()
            .map_err(classify_receive_io)?
    } else {
        fs::metadata(destination).map_err(classify_receive_io)?
    };
    if !metadata.is_file() || metadata.len() != entry.size {
        return Err(DataError::InvalidPath);
    }
    Ok(())
}

fn ensure_source_unchanged(entry: &TransferEntryRecord) -> Result<(), DataError> {
    let source = entry
        .source_ref
        .as_deref()
        .ok_or(DataError::SourceChanged)?;
    if is_platform_ref(source) {
        let file = open_document(source, false).map_err(classify_source_io)?;
        let metadata = file.metadata().map_err(classify_source_io)?;
        if metadata.len() != entry.size {
            return Err(DataError::SourceChanged);
        }
        return Ok(());
    }
    let metadata = fs::metadata(source).map_err(classify_source_io)?;
    if !metadata.is_file()
        || metadata.len() != entry.size
        || modified_at_ms(&metadata) != entry.modified_at_ms
    {
        return Err(DataError::SourceChanged);
    }
    Ok(())
}

fn open_source_file(entry: &TransferEntryRecord) -> Result<File, DataError> {
    let source = entry
        .source_ref
        .as_deref()
        .ok_or(DataError::SourceChanged)?;
    if is_platform_ref(source) {
        let file = open_document(source, false).map_err(classify_source_io)?;
        if file.metadata().map_err(classify_source_io)?.len() != entry.size {
            return Err(DataError::SourceChanged);
        }
        Ok(file)
    } else {
        File::open(source).map_err(classify_source_io)
    }
}

fn classify_source_io(error: io::Error) -> DataError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => DataError::PermissionLost,
        io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof => DataError::SourceChanged,
        _ => DataError::Io(error),
    }
}

fn classify_receive_io(error: io::Error) -> DataError {
    if error.kind() == io::ErrorKind::StorageFull
        || matches!(error.raw_os_error(), Some(28 | 39 | 112 | 122))
    {
        return DataError::NotEnoughSpace;
    }
    match error.kind() {
        io::ErrorKind::PermissionDenied => DataError::PermissionLost,
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => DataError::InvalidPath,
        _ => DataError::Io(error),
    }
}

fn open_receive_file(reference: &str) -> io::Result<File> {
    if is_platform_ref(reference) {
        open_document(reference, true)
    } else {
        OpenOptions::new().read(true).write(true).open(reference)
    }
}

fn prepare_receive_file_length(
    file: &File,
    reference: &str,
    persisted_offset: u64,
) -> Result<(), DataError> {
    // SAF prepared the document and reported its current size before handing
    // the descriptor to Rust. ProxyFileDescriptor does not support ftruncate.
    if !is_platform_ref(reference) {
        file.set_len(persisted_offset)
            .map_err(classify_receive_io)?;
    }
    Ok(())
}

fn is_platform_ref(reference: &str) -> bool {
    reference.starts_with("content://")
}

fn configure_data_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(DATA_IDLE_TIMEOUT))?;
    stream.set_write_timeout(Some(DATA_IDLE_TIMEOUT))?;
    Ok(())
}

fn modified_at_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn unix_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(i64::MAX)
}

fn record_data_error(database_path: &Path, message: &str) {
    events::publish(CoreEventKind::CoreErrorOccurred, None);
    let Some(directory) = database_path.parent() else {
        return;
    };
    let path: PathBuf = directory.join("lan_chat.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} data {message}", unix_time_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    use crate::{
        domain::{ClientOperationId, MessageKind, Platform},
        storage::AcceptedOffset,
        transfer::{enumerate_path, prepare_receive_paths},
    };

    #[test]
    fn io_failures_are_classified_by_the_side_that_owns_the_file() {
        assert!(matches!(
            classify_receive_io(io::Error::from(io::ErrorKind::StorageFull)),
            DataError::NotEnoughSpace
        ));
        assert!(matches!(
            classify_receive_io(io::Error::from(io::ErrorKind::PermissionDenied)),
            DataError::PermissionLost
        ));
        assert!(matches!(
            classify_receive_io(io::Error::from(io::ErrorKind::NotFound)),
            DataError::InvalidPath
        ));
        assert!(matches!(
            classify_source_io(io::Error::from(io::ErrorKind::PermissionDenied)),
            DataError::PermissionLost
        ));
        assert!(matches!(
            classify_source_io(io::Error::from(io::ErrorKind::NotFound)),
            DataError::SourceChanged
        ));
    }

    #[test]
    fn receive_length_is_only_truncated_for_native_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("partial.bin");
        fs::write(&path, [1_u8, 2, 3, 4]).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();

        prepare_receive_file_length(&file, "content://documents/partial", 1).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 4);

        prepare_receive_file_length(&file, path.to_str().unwrap(), 1).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 1);
    }

    #[test]
    fn receive_publish_restart_finishes_folder_after_one_entry_was_published() {
        let fixture = prepare_file_transfer(b"remaining bytes".to_vec());
        let published_destination = fixture.receive_dir.join("published.bin");
        fs::write(&published_destination, b"published bytes").unwrap();
        fs::write(&fixture.partial_ref, b"remaining bytes").unwrap();
        let storage = Storage::open(&fixture.receiver_path).unwrap();
        let (_, remaining_entries) = storage.get_transfer(fixture.transfer_id).unwrap();
        let remaining_entry = remaining_entries.into_iter().next().unwrap();
        let remaining_partial = remaining_entry.partial_ref.clone().unwrap();
        let remaining_destination = remaining_entry.destination_ref.clone().unwrap();
        let published_entry = TransferEntryRecord {
            entry_id: EntryId::generate(),
            transfer_id: fixture.transfer_id,
            entry_kind: TransferEntryKind::File,
            relative_path: "folder/published.bin".to_owned(),
            size: 15,
            modified_at_ms: 0,
            source_ref: None,
            destination_ref: Some(published_destination.to_string_lossy().into_owned()),
            partial_ref: None,
            persisted_offset: 15,
            state: "completed".to_owned(),
        };

        publish_received_files(&storage, &[published_entry.clone(), remaining_entry]).unwrap();
        assert_eq!(
            fs::read(&published_destination).unwrap(),
            b"published bytes"
        );
        assert_eq!(fs::read(remaining_destination).unwrap(), b"remaining bytes");
        assert!(!Path::new(&remaining_partial).exists());

        let mut wrong_size = published_entry;
        wrong_size.size += 1;
        assert!(matches!(
            publish_received_files(&storage, &[wrong_size]),
            Err(DataError::InvalidPath)
        ));
    }

    #[test]
    fn data_slots_enforce_global_and_per_peer_connection_limits() {
        let first = DeviceId::from_bytes([1; 16]);
        let second = DeviceId::from_bytes([2; 16]);
        let third = DeviceId::from_bytes([3; 16]);
        let mut slots = HashMap::new();

        assert!(reserve_data_slot(&mut slots, first));
        assert!(reserve_data_slot(&mut slots, first));
        assert!(!reserve_data_slot(&mut slots, first));
        assert!(reserve_data_slot(&mut slots, second));
        assert!(reserve_data_slot(&mut slots, second));
        assert!(!reserve_data_slot(&mut slots, third));
        assert_eq!(slots.values().sum::<usize>(), MAX_GLOBAL_DATA_CONNECTIONS);
        assert_eq!(slots[&first], MAX_PEER_DATA_CONNECTIONS);
        assert_eq!(slots[&second], MAX_PEER_DATA_CONNECTIONS);
    }

    struct PreparedFileTransfer {
        _temp: tempfile::TempDir,
        sender_path: PathBuf,
        receiver_path: PathBuf,
        source_bytes: Vec<u8>,
        sender: LocalProfile,
        receiver: LocalProfile,
        transfer_id: TransferId,
        entry_id: EntryId,
        receive_dir: PathBuf,
        partial_ref: String,
    }

    fn prepare_file_transfer(source_bytes: Vec<u8>) -> PreparedFileTransfer {
        let temp = tempfile::tempdir().unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
        let source_path = temp.path().join("source.bin");
        fs::write(&source_path, &source_bytes).unwrap();

        let mut sender_storage = Storage::open(&sender_path).unwrap();
        let sender = sender_storage
            .load_or_create_profile("电脑", Platform::Windows)
            .unwrap();
        let mut receiver_storage = Storage::open(&receiver_path).unwrap();
        let receiver = receiver_storage
            .load_or_create_profile("手机", Platform::Android)
            .unwrap();
        sender_storage
            .upsert_nearby_peer(
                receiver.device_id,
                &receiver.device_name,
                receiver.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        receiver_storage
            .upsert_nearby_peer(
                sender.device_id,
                &sender.device_name,
                sender.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        let conversation = sender_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                sender.device_id,
                receiver.device_id,
            )
            .unwrap();
        let manifest = enumerate_path(&source_path, MessageKind::File).unwrap();
        let outgoing = sender_storage
            .create_outgoing_offer(
                &sender,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        let offer_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver.device_id,
            "body": {
                "original_event_id": outgoing.event_id,
                "message_id": outgoing.message_id,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        receiver_storage
            .receive_file_offer(
                receiver.device_id,
                sender.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
                &conversation.conversation_id,
                1,
                None,
                &manifest,
                None,
                &offer_receipt,
            )
            .unwrap();
        sender_storage
            .mark_file_offer_stored(
                receiver.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
            )
            .unwrap();
        let (_, incoming_entries) = receiver_storage.get_transfer(outgoing.transfer_id).unwrap();
        let receive_dir = temp.path().join("downloads");
        let prepared = prepare_receive_paths(
            &receive_dir,
            outgoing.transfer_id,
            &manifest.display_name,
            &incoming_entries,
        )
        .unwrap();
        receiver_storage
            .accept_incoming_offer(
                &receiver,
                ClientOperationId::generate(),
                outgoing.transfer_id,
                receive_dir.to_str().unwrap(),
                &prepared,
            )
            .unwrap();
        let offsets = prepared
            .iter()
            .map(|entry| AcceptedOffset {
                entry_id: entry.entry_id,
                offset: entry.persisted_offset,
            })
            .collect::<Vec<_>>();
        sender_storage
            .receive_file_accept(
                receiver.device_id,
                EventId::generate(),
                outgoing.transfer_id,
                &offsets,
                "{}",
            )
            .unwrap();
        let entry_id = incoming_entries[0].entry_id;
        let partial_ref = prepared[0].partial_ref.clone().unwrap();
        drop(sender_storage);
        drop(receiver_storage);

        PreparedFileTransfer {
            _temp: temp,
            sender_path,
            receiver_path,
            source_bytes,
            sender,
            receiver,
            transfer_id: outgoing.transfer_id,
            entry_id,
            receive_dir,
            partial_ref,
        }
    }

    fn run_file_transfer(source_bytes: Vec<u8>) {
        let fixture = prepare_file_transfer(source_bytes);
        let sender_path = fixture.sender_path.clone();
        let receiver_path = fixture.receiver_path.clone();
        assert!(
            Storage::open(&sender_path)
                .unwrap()
                .claim_send_transfer(fixture.transfer_id)
                .unwrap()
        );

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = fixture.receiver.clone();
        let receiver_path_for_thread = receiver_path.clone();
        let receiver_stop = Arc::new(AtomicBool::new(false));
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let first: Value = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES).unwrap();
            handle_data_connection(
                first,
                &mut stream,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &receiver_stop,
            )
            .unwrap();
        });
        let sender_stop = Arc::new(AtomicBool::new(false));
        send_transfer_to(
            &fixture.sender,
            &sender_path,
            fixture.transfer_id,
            address,
            &sender_stop,
        )
        .unwrap();
        worker.join().unwrap();

        assert_eq!(
            fs::read(fixture.receive_dir.join("source.bin")).unwrap(),
            fixture.source_bytes
        );
        assert_eq!(
            Storage::open(&sender_path)
                .unwrap()
                .get_transfer(fixture.transfer_id)
                .unwrap()
                .0
                .state,
            "completed"
        );
        assert_eq!(
            Storage::open(&receiver_path)
                .unwrap()
                .get_transfer(fixture.transfer_id)
                .unwrap()
                .0
                .state,
            "completed"
        );
        let completion = Storage::open(&receiver_path)
            .unwrap()
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "delivery_receipt")
            .unwrap();
        assert!(!completion.requires_receipt);
        let completion_json: Value = serde_json::from_str(&completion.payload_json).unwrap();
        assert_eq!(completion_json["body"]["stage"], "completed");
    }

    #[test]
    fn data_connection_streams_file_and_completes_both_databases() {
        run_file_transfer(
            (0..(3 * 1024 * 1024 + 37))
                .map(|index| (index % 251) as u8)
                .collect(),
        );
    }

    #[test]
    fn data_connection_completes_zero_and_one_byte_files() {
        run_file_transfer(Vec::new());
        run_file_transfer(vec![0xa5]);
    }

    #[test]
    fn data_connection_eof_one_byte_short_is_recoverable_and_never_published() {
        let fixture = prepare_file_transfer(vec![0xa5]);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = fixture.receiver.clone();
        let receiver_path_for_thread = fixture.receiver_path.clone();
        let receiver_stop = Arc::new(AtomicBool::new(false));
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let first: Value = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES).unwrap();
            handle_data_connection(
                first,
                &mut stream,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &receiver_stop,
            )
        });

        let mut stream = TcpStream::connect(address).unwrap();
        configure_data_stream(&stream).unwrap();
        let connection_id = EventId::generate();
        write_json_frame(
            &mut stream,
            &DataHello {
                version: PROTOCOL_VERSION,
                frame_type: "data_hello".to_owned(),
                connection_id,
                sender_device_id: fixture.sender.device_id,
                transfer_id: fixture.transfer_id,
                protocol: PROTOCOL_VERSION,
            },
        )
        .unwrap();
        let ack: DataHelloAck = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES).unwrap();
        assert_eq!(ack.connection_id, connection_id);
        write_json_frame(
            &mut stream,
            &DataEntry {
                version: PROTOCOL_VERSION,
                frame_type: "data_entry".to_owned(),
                transfer_id: fixture.transfer_id,
                entry_id: fixture.entry_id,
                offset: 0,
                data_length: 1,
            },
        )
        .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();

        let error = worker.join().unwrap().unwrap_err();
        assert!(matches!(
            error,
            DataError::Io(ref error) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        let (transfer, entries) = Storage::open(&fixture.receiver_path)
            .unwrap()
            .get_transfer(fixture.transfer_id)
            .unwrap();
        assert_eq!(transfer.state, "queued");
        assert_eq!(transfer.failure_reason.as_deref(), Some("connection_error"));
        assert_eq!(transfer.persisted_bytes, 0);
        assert_eq!(entries[0].persisted_offset, 0);
        assert_eq!(fs::metadata(&fixture.partial_ref).unwrap().len(), 0);
        assert!(!fixture.receive_dir.join("source.bin").exists());
    }
}
