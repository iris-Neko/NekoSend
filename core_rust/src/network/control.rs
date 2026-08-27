use std::{
    collections::HashMap,
    fs::OpenOptions,
    io,
    io::Write as _,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    path::Path,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use socket2::{Domain, Protocol, Socket, Type};
use thiserror::Error;

use super::{DataError, handle_data_connection, schedule_ready_sends, stop_active_transfer};

use crate::{
    CORE_VERSION, PROTOCOL_VERSION,
    domain::{
        BindingId, ClientOperationId, DeviceId, EventId, GroupId, InviteId, MessageId, MessageKind,
        Platform, TransferId,
    },
    events::{self, CoreEventKind},
    protocol::{
        FrameError, MAX_CONTROL_FRAME_BYTES, read_json_frame, write_json_frame,
        write_json_frame_with_limit,
    },
    storage::{AcceptedOffset, GroupSnapshot, LocalProfile, PendingOutbox, Storage, StorageError},
    transfer::{MAX_MANIFEST_JSON_BYTES, ManifestEntry, TransferManifest, prepare_receive_paths},
};

pub const CONTROL_PORT: u16 = 53_318;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const OUTBOX_POLL_INTERVAL: Duration = Duration::from_millis(250);
const PING_INTERVAL: Duration = Duration::from_secs(10);
const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECTION_ACTOR_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("failed to bind TCP control listener: {0}")]
    Bind(#[source] io::Error),
    #[error("control connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("control frame failed: {0}")]
    Frame(#[from] FrameError),
    #[error("control JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("control protocol validation failed: {0}")]
    Protocol(&'static str),
    #[error("peer rejected control event with {code}: {message}")]
    RemoteProtocol { code: String, message: String },
    #[error("control database operation failed: {0}")]
    Storage(#[from] StorageError),
    #[error("data connection failed: {0}")]
    Data(#[from] DataError),
}

pub struct ControlService {
    stop: Arc<AtomicBool>,
    listener_worker: Option<JoinHandle<()>>,
    outbox_worker: Option<JoinHandle<()>>,
}

impl ControlService {
    pub fn start(profile: LocalProfile, database_path: &Path) -> Result<Self, ControlError> {
        let listener = create_listener().map_err(ControlError::Bind)?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(ConnectionRegistry::default());
        let listener_stop = Arc::clone(&stop);
        let listener_connections = Arc::clone(&connections);
        let listener_profile = profile.clone();
        let listener_path = database_path.to_path_buf();
        let listener_worker = thread::Builder::new()
            .name("lan-chat-control-listener".to_owned())
            .spawn(move || {
                run_listener(
                    listener,
                    listener_profile,
                    listener_path,
                    listener_stop,
                    listener_connections,
                );
            })
            .map_err(ControlError::Bind)?;

        let outbox_stop = Arc::clone(&stop);
        let outbox_connections = Arc::clone(&connections);
        let outbox_path = database_path.to_path_buf();
        let outbox_worker = thread::Builder::new()
            .name("lan-chat-outbox".to_owned())
            .spawn(move || {
                run_outbox(profile, outbox_path, outbox_stop, outbox_connections);
            })
            .map_err(ControlError::Bind)?;

        Ok(Self {
            stop,
            listener_worker: Some(listener_worker),
            outbox_worker: Some(outbox_worker),
        })
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.listener_worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.outbox_worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ConnectionKey {
    initiator_device_id: DeviceId,
    connection_id: EventId,
}

#[derive(Clone)]
struct ConnectionHandle {
    key: ConnectionKey,
    address: SocketAddr,
    commands: Sender<ConnectionCommand>,
}

enum ConnectionCommand {
    Deliver {
        entry: PendingOutbox,
        result: SyncSender<Result<(), DeliveryFailure>>,
    },
    Retire {
        kept_connection_id: EventId,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum DeliveryFailure {
    Connection(String),
    Protocol { code: String, message: String },
}

#[derive(Default)]
struct ConnectionRegistry {
    inner: Mutex<HashMap<DeviceId, ConnectionHandle>>,
}

enum RegisterConnection {
    Accepted,
    Rejected { kept_connection_id: EventId },
}

impl ConnectionRegistry {
    fn register(
        &self,
        peer_device_id: DeviceId,
        candidate: ConnectionHandle,
    ) -> RegisterConnection {
        let mut connections = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(current) = connections.get(&peer_device_id) {
            if current.key <= candidate.key {
                return RegisterConnection::Rejected {
                    kept_connection_id: current.key.connection_id,
                };
            }
            let _ = current.commands.send(ConnectionCommand::Retire {
                kept_connection_id: candidate.key.connection_id,
            });
        }
        connections.insert(peer_device_id, candidate);
        RegisterConnection::Accepted
    }

    fn current(&self, peer_device_id: DeviceId) -> Option<ConnectionHandle> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&peer_device_id)
            .cloned()
    }

    fn remove_if_current(&self, peer_device_id: DeviceId, key: ConnectionKey) {
        let mut connections = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if connections
            .get(&peer_device_id)
            .is_some_and(|connection| connection.key == key)
        {
            connections.remove(&peer_device_id);
        }
    }

    fn retire_if_address_changed(&self, peer_device_id: DeviceId, address: SocketAddr) {
        let mut connections = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if connections
            .get(&peer_device_id)
            .is_some_and(|connection| connection.address != address)
            && let Some(connection) = connections.remove(&peer_device_id)
        {
            let _ = connection.commands.send(ConnectionCommand::Retire {
                kept_connection_id: connection.key.connection_id,
            });
        }
    }
}

impl Drop for ControlService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    connection_id: EventId,
    initiator_device_id: DeviceId,
    device_id: DeviceId,
    device_name: String,
    platform: Platform,
    app_version: String,
    protocol_min: u16,
    protocol_max: u16,
    sent_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelloAck {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    connection_id: EventId,
    device_id: DeviceId,
    device_name: String,
    platform: Platform,
    selected_protocol: u16,
    received_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProtocolErrorFrame {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    code: String,
    related_event_id: Option<EventId>,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DuplicateConnectionFrame {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    kept_connection_id: EventId,
}

#[derive(Debug, Serialize, Deserialize)]
struct PingFrame {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    ping_id: EventId,
    sent_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct TextEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: TextBody,
}

#[derive(Debug, Deserialize)]
struct TextBody {
    message_id: MessageId,
    conversation_id: String,
    message_kind: String,
    text: String,
    created_at_ms: i64,
    group_revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GroupInviteEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupInviteBody,
}

#[derive(Debug, Deserialize)]
struct GroupInviteBody {
    invite_id: InviteId,
    group: GroupSnapshot,
}

#[derive(Debug, Deserialize)]
struct GroupInviteReplyEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupInviteReplyBody,
}

#[derive(Debug, Deserialize)]
struct GroupInviteReplyBody {
    invite_id: InviteId,
    group_id: GroupId,
    decision: String,
}

#[derive(Debug, Deserialize)]
struct GroupUpdateEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupUpdateBody,
}

#[derive(Debug, Deserialize)]
struct GroupUpdateBody {
    previous_revision: u64,
    group: GroupSnapshot,
    change_reason: String,
}

#[derive(Debug, Deserialize)]
struct GroupSyncRequestEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupSyncRequestBody,
}

#[derive(Debug, Deserialize)]
struct GroupSyncRequestBody {
    group_id: GroupId,
    known_revision: u64,
}

#[derive(Debug, Deserialize)]
struct GroupSyncResponseEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupSyncResponseBody,
}

#[derive(Debug, Deserialize)]
struct GroupSyncResponseBody {
    group: GroupSnapshot,
    requested_revision: u64,
}

#[derive(Debug, Deserialize)]
struct GroupLeaveEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: GroupLeaveBody,
}

#[derive(Debug, Deserialize)]
struct GroupLeaveBody {
    group_id: GroupId,
    known_revision: u64,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceBindEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: OwnDeviceBindBody,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceBindBody {
    binding_id: BindingId,
    requester_name: String,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceBindReplyEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: OwnDeviceBindReplyBody,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceBindReplyBody {
    binding_id: BindingId,
    decision: String,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceUnbindEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: OwnDeviceUnbindBody,
}

#[derive(Debug, Deserialize)]
struct OwnDeviceUnbindBody {
    binding_id: BindingId,
}

#[derive(Debug, Deserialize)]
struct ClipboardUpdateEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: ClipboardUpdateBody,
}

#[derive(Debug, Deserialize)]
struct ClipboardUpdateBody {
    message_id: MessageId,
    conversation_id: String,
    origin_device_id: DeviceId,
    clipboard_sequence: u64,
    content_type: String,
    text: String,
    automatic: bool,
}

#[derive(Debug, Deserialize)]
struct ReceiptEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: ReceiptBody,
}

#[derive(Debug, Deserialize)]
struct ReceiptBody {
    original_event_id: EventId,
    message_id: Option<MessageId>,
    transfer_id: Option<TransferId>,
    stage: String,
}

#[derive(Debug, Deserialize)]
struct FileOfferEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: FileOfferBody,
}

#[derive(Debug, Deserialize)]
struct FileOfferBody {
    message_id: MessageId,
    transfer_id: TransferId,
    conversation_id: String,
    message_kind: MessageKind,
    display_name: String,
    total_size: u64,
    entry_count: u32,
    created_at_ms: i64,
    group_revision: Option<u64>,
    origin_device_id: Option<DeviceId>,
    clipboard_sequence: Option<u64>,
    content_fingerprint: Option<String>,
    automatic: Option<bool>,
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct FileAcceptEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: FileAcceptBody,
}

#[derive(Debug, Deserialize)]
struct FileAcceptBody {
    transfer_id: TransferId,
    entries: Vec<AcceptedOffset>,
}

#[derive(Debug, Deserialize)]
struct FileRejectEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: FileRejectBody,
}

#[derive(Debug, Deserialize)]
struct FileRejectBody {
    transfer_id: TransferId,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct TransferPauseEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: TransferPauseBody,
}

#[derive(Debug, Deserialize)]
struct TransferPauseBody {
    transfer_id: TransferId,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct TransferResumeEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: TransferResumeBody,
}

#[derive(Debug, Deserialize)]
struct TransferResumeBody {
    transfer_id: TransferId,
    entries: Vec<AcceptedOffset>,
}

#[derive(Debug, Deserialize)]
struct TransferCancelEnvelope {
    version: u16,
    #[serde(rename = "type")]
    frame_type: String,
    event_id: EventId,
    sender_device_id: DeviceId,
    body: TransferCancelBody,
}

#[derive(Debug, Deserialize)]
struct TransferCancelBody {
    transfer_id: TransferId,
    cancelled_by: DeviceId,
}

fn create_listener() -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, CONTROL_PORT).into())?;
    socket.listen(32)?;
    let listener: TcpListener = socket.into();
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn run_listener(
    listener: TcpListener,
    profile: LocalProfile,
    database_path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    connections: Arc<ConnectionRegistry>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, source)) => {
                let connection_profile = profile.clone();
                let connection_path = database_path.clone();
                let connection_stop = Arc::clone(&stop);
                let connection_registry = Arc::clone(&connections);
                let _ = thread::Builder::new()
                    .name("lan-chat-incoming".to_owned())
                    .spawn(move || {
                        if let Err(error) = configure_stream(&stream) {
                            record_control_error(
                                &connection_path,
                                &format!(
                                    "incoming from {source}: configure stream failed: {error}"
                                ),
                            );
                            return;
                        }
                        if let Err(error) = handle_incoming(
                            &mut stream,
                            source,
                            &connection_profile,
                            &connection_path,
                            &connection_stop,
                            &connection_registry,
                        ) {
                            record_control_error(
                                &connection_path,
                                &format!("incoming from {source}: {error}"),
                            );
                        }
                    });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn run_outbox(
    profile: LocalProfile,
    database_path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    connections: Arc<ConnectionRegistry>,
) {
    while !stop.load(Ordering::Acquire) {
        if let Ok(mut storage) = Storage::open(&database_path)
            && let Ok(entries) = storage.pending_outbox()
        {
            for entry in entries {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Err(error) =
                    deliver_outbox_managed(&profile, &database_path, &entry, &connections, &stop)
                {
                    record_control_error(
                        &database_path,
                        &format!(
                            "outbox peer={} event={}: {error}",
                            entry.peer_device_id, entry.event_id
                        ),
                    );
                    match &error {
                        ControlError::RemoteProtocol { code, .. }
                            if !remote_protocol_should_retry(code) =>
                        {
                            let _ = storage.mark_outbox_protocol_failure(
                                entry.peer_device_id,
                                entry.event_id,
                                &code.to_ascii_lowercase(),
                            );
                            events::publish(CoreEventKind::CoreErrorOccurred, None);
                        }
                        _ => {
                            let _ = storage.mark_outbox_retry(entry.peer_device_id, entry.event_id);
                        }
                    }
                }
            }
        }
        schedule_ready_sends(&profile, &database_path, &stop);
        thread::sleep(OUTBOX_POLL_INTERVAL);
    }
}

fn handle_incoming(
    stream: &mut TcpStream,
    source: SocketAddr,
    profile: &LocalProfile,
    database_path: &Path,
    stop: &Arc<AtomicBool>,
    connections: &Arc<ConnectionRegistry>,
) -> Result<(), ControlError> {
    let first_frame: Value = read_json_frame(stream, MAX_CONTROL_FRAME_BYTES)?;
    if first_frame.get("type").and_then(Value::as_str) == Some("data_hello") {
        return handle_data_connection(first_frame, stream, profile, database_path, stop)
            .map_err(ControlError::from);
    }
    let hello: Hello = serde_json::from_value(first_frame)?;
    if hello.protocol_min > PROTOCOL_VERSION || hello.protocol_max < PROTOCOL_VERSION {
        let _ = write_protocol_error(
            stream,
            "UNSUPPORTED_PROTOCOL",
            Some(hello.connection_id),
            "no mutually supported protocol version",
        );
        return Err(ControlError::Protocol("unsupported protocol version"));
    }
    validate_hello(&hello, profile.device_id)?;
    let source_ip = match source.ip() {
        IpAddr::V4(ip) => ip,
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .ok_or(ControlError::Protocol("IPv6 is not supported"))?,
    };
    let mut storage = Storage::open(database_path)?;
    storage.upsert_nearby_peer(
        hello.device_id,
        &hello.device_name,
        hello.platform,
        &source_ip.to_string(),
        unix_time_ms(),
    )?;
    write_json_frame(
        stream,
        &HelloAck {
            version: PROTOCOL_VERSION,
            frame_type: "hello_ack".to_owned(),
            connection_id: hello.connection_id,
            device_id: profile.device_id,
            device_name: profile.device_name.clone(),
            platform: profile.platform,
            selected_protocol: PROTOCOL_VERSION,
            received_at_ms: unix_time_ms(),
        },
    )?;

    let key = ConnectionKey {
        initiator_device_id: hello.initiator_device_id,
        connection_id: hello.connection_id,
    };
    let address = SocketAddr::V4(SocketAddrV4::new(source_ip, CONTROL_PORT));
    let (commands, receiver) = mpsc::channel();
    let registration = connections.register(
        hello.device_id,
        ConnectionHandle {
            key,
            address,
            commands,
        },
    );
    if let RegisterConnection::Rejected { kept_connection_id } = registration {
        write_duplicate_connection(stream, kept_connection_id)?;
        return Ok(());
    }

    stream.set_read_timeout(Some(CONNECTION_ACTOR_POLL_INTERVAL))?;
    let result = run_connection_actor(stream, &hello, &mut storage, profile, stop, receiver);
    connections.remove_if_current(hello.device_id, key);
    result
}

fn handle_control_event_value(
    stream: &mut TcpStream,
    envelope_value: Value,
    hello: &Hello,
    storage: &mut Storage,
    profile: &LocalProfile,
) -> Result<(), ControlError> {
    let frame_type = envelope_value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ControlError::Protocol("control envelope is missing type"))?
        .to_owned();
    let event_entity_id = control_event_entity_id(&envelope_value);
    if frame_type == "ping" {
        let ping: PingFrame = serde_json::from_value(envelope_value)?;
        if ping.version != PROTOCOL_VERSION || ping.frame_type != "ping" {
            return Err(ControlError::Protocol("invalid ping"));
        }
        write_json_frame(
            stream,
            &PingFrame {
                version: PROTOCOL_VERSION,
                frame_type: "pong".to_owned(),
                ping_id: ping.ping_id,
                sent_at_ms: ping.sent_at_ms,
            },
        )?;
        return Ok(());
    }
    if frame_type == "pong" {
        let pong: PingFrame = serde_json::from_value(envelope_value)?;
        if pong.version != PROTOCOL_VERSION || pong.frame_type != "pong" {
            return Err(ControlError::Protocol("invalid pong"));
        }
        return Ok(());
    }
    if frame_type == "delivery_receipt" {
        let envelope: ReceiptEnvelope = serde_json::from_value(envelope_value)?;
        return handle_completed_receipt(storage, hello, &envelope);
    }
    let saved_receipt = match frame_type.as_str() {
        "text_message" => {
            let envelope: TextEnvelope = serde_json::from_value(envelope_value)?;
            validate_text_envelope(&envelope, hello.device_id)?;
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                Some(envelope.body.message_id),
                None,
                "stored",
            );
            storage.receive_text_message(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.message_id,
                &envelope.body.conversation_id,
                &envelope.body.message_kind,
                &envelope.body.text,
                envelope.body.created_at_ms,
                envelope.body.group_revision,
                &receipt_json,
            )?
        }
        "group_invite" => {
            let envelope: GroupInviteEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_invite"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid group_invite"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_invite(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.invite_id,
                &envelope.body.group,
                &receipt_json,
            )?
        }
        "group_invite_reply" => {
            let envelope: GroupInviteReplyEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_invite_reply"
                || envelope.sender_device_id != hello.device_id
                || !matches!(envelope.body.decision.as_str(), "accepted" | "rejected")
            {
                return Err(ControlError::Protocol("invalid group_invite_reply"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_invite_reply(
                profile,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.invite_id,
                envelope.body.group_id,
                envelope.body.decision == "accepted",
                &receipt_json,
            )?
        }
        "group_update" => {
            let envelope: GroupUpdateEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_update"
                || envelope.sender_device_id != hello.device_id
                || envelope.body.change_reason.is_empty()
                || envelope.body.change_reason.len() > 64
            {
                return Err(ControlError::Protocol("invalid group_update"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_update(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.previous_revision,
                &envelope.body.group,
                &receipt_json,
            )?
        }
        "group_sync_request" => {
            let envelope: GroupSyncRequestEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_sync_request"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid group_sync_request"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_sync_request(
                profile,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.group_id,
                envelope.body.known_revision,
                &receipt_json,
            )?
        }
        "group_sync_response" => {
            let envelope: GroupSyncResponseEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_sync_response"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid group_sync_response"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_sync_response(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.requested_revision,
                &envelope.body.group,
                &receipt_json,
            )?
        }
        "group_leave_request" => {
            let envelope: GroupLeaveEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "group_leave_request"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid group_leave_request"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_group_leave_request(
                profile,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.group_id,
                envelope.body.known_revision,
                &receipt_json,
            )?
        }
        "own_device_bind" => {
            let envelope: OwnDeviceBindEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "own_device_bind"
                || envelope.sender_device_id != hello.device_id
                || envelope.body.requester_name != hello.device_name
            {
                return Err(ControlError::Protocol("invalid own_device_bind"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_own_device_bind(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.binding_id,
                &receipt_json,
            )?
        }
        "own_device_bind_reply" => {
            let envelope: OwnDeviceBindReplyEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "own_device_bind_reply"
                || envelope.sender_device_id != hello.device_id
                || !matches!(envelope.body.decision.as_str(), "accepted" | "rejected")
            {
                return Err(ControlError::Protocol("invalid own_device_bind_reply"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_own_device_bind_reply(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.binding_id,
                envelope.body.decision == "accepted",
                &receipt_json,
            )?
        }
        "own_device_unbind" => {
            let envelope: OwnDeviceUnbindEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "own_device_unbind"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid own_device_unbind"));
            }
            let receipt_json =
                delivery_receipt_json(profile.device_id, envelope.event_id, None, None, "stored");
            storage.receive_own_device_unbind(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.binding_id,
                &receipt_json,
            )?
        }
        "clipboard_update" => {
            let envelope: ClipboardUpdateEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "clipboard_update"
                || envelope.sender_device_id != hello.device_id
                || envelope.body.content_type != "text"
            {
                return Err(ControlError::Protocol("invalid clipboard_update"));
            }
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                Some(envelope.body.message_id),
                None,
                "stored",
            );
            let received = storage.receive_clipboard_text(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.message_id,
                &envelope.body.conversation_id,
                envelope.body.origin_device_id,
                envelope.body.clipboard_sequence,
                &envelope.body.text,
                envelope.body.automatic,
                &receipt_json,
            )?;
            if received.write_to_system && !received.duplicate {
                crate::platform_io::enqueue_clipboard_write(
                    &envelope.body.text,
                    serde_json::json!({
                        "origin_device_id": envelope.body.origin_device_id,
                        "clipboard_sequence": envelope.body.clipboard_sequence
                    })
                    .to_string(),
                );
            }
            received.receipt_json
        }
        "file_offer" => {
            let envelope: FileOfferEnvelope = serde_json::from_value(envelope_value)?;
            validate_file_offer(&envelope, hello.device_id)?;
            let clipboard_metadata = envelope.body.origin_device_id.map(|origin_device_id| {
                crate::storage::ClipboardImageMetadata {
                    origin_device_id,
                    clipboard_sequence: envelope.body.clipboard_sequence.unwrap_or_default(),
                    content_fingerprint: envelope
                        .body
                        .content_fingerprint
                        .clone()
                        .unwrap_or_default(),
                    automatic: envelope.body.automatic.unwrap_or(false),
                }
            });
            let manifest = TransferManifest {
                message_kind: envelope.body.message_kind,
                display_name: envelope.body.display_name,
                total_size: envelope.body.total_size,
                entry_count: envelope.body.entry_count,
                entries: envelope.body.entries,
            };
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                Some(envelope.body.message_id),
                Some(envelope.body.transfer_id),
                "stored",
            );
            let received = storage.receive_file_offer(
                profile.device_id,
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.message_id,
                envelope.body.transfer_id,
                &envelope.body.conversation_id,
                envelope.body.created_at_ms,
                envelope.body.group_revision,
                &manifest,
                clipboard_metadata.as_ref(),
                &receipt_json,
            )?;
            if let Some(receive_base_ref) = received.auto_receive_base_ref.as_deref() {
                let (transfer, entries) = storage.get_transfer(envelope.body.transfer_id)?;
                let (accepted_base_ref, prepared) = if receive_base_ref.starts_with("content://") {
                    crate::platform_io::prepare_document_receive(receive_base_ref, &entries)?
                } else {
                    let prepared = prepare_receive_paths(
                        receive_base_ref,
                        envelope.body.transfer_id,
                        &transfer.display_name,
                        &entries,
                    )
                    .map_err(io::Error::other)?;
                    (receive_base_ref.to_owned(), prepared)
                };
                storage.accept_incoming_offer(
                    profile,
                    envelope
                        .event_id
                        .to_string()
                        .parse::<ClientOperationId>()
                        .map_err(|_| StorageError::InvalidStoredId)?,
                    envelope.body.transfer_id,
                    &accepted_base_ref,
                    &prepared,
                )?;
            }
            received.receipt_json
        }
        "file_accept" => {
            let envelope: FileAcceptEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "file_accept"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid file_accept"));
            }
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                None,
                Some(envelope.body.transfer_id),
                "stored",
            );
            storage.receive_file_accept(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.transfer_id,
                &envelope.body.entries,
                &receipt_json,
            )?
        }
        "file_reject" => {
            let envelope: FileRejectEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "file_reject"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid file_reject"));
            }
            let failure_reason = match envelope.body.reason.as_str() {
                "user_rejected" => "rejected",
                "not_enough_space" => "not_enough_space",
                "permission_lost" => "permission_lost",
                "invalid_path" => "invalid_path",
                "unsupported" => "unsupported",
                _ => return Err(ControlError::Protocol("invalid file_reject reason")),
            };
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                None,
                Some(envelope.body.transfer_id),
                "stored",
            );
            let receipt = storage.receive_file_reject(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.transfer_id,
                failure_reason,
                &receipt_json,
            )?;
            stop_active_transfer(envelope.body.transfer_id);
            receipt
        }
        "transfer_pause" => {
            let envelope: TransferPauseEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "transfer_pause"
                || envelope.sender_device_id != hello.device_id
                || !matches!(envelope.body.reason.as_str(), "user" | "system")
            {
                return Err(ControlError::Protocol("invalid transfer_pause"));
            }
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                None,
                Some(envelope.body.transfer_id),
                "stored",
            );
            let saved = storage.receive_transfer_pause(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.transfer_id,
                &receipt_json,
            )?;
            stop_active_transfer(envelope.body.transfer_id);
            saved
        }
        "transfer_resume" => {
            let envelope: TransferResumeEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "transfer_resume"
                || envelope.sender_device_id != hello.device_id
            {
                return Err(ControlError::Protocol("invalid transfer_resume"));
            }
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                None,
                Some(envelope.body.transfer_id),
                "stored",
            );
            storage
                .receive_transfer_resume(
                    profile,
                    envelope.sender_device_id,
                    envelope.event_id,
                    envelope.body.transfer_id,
                    &envelope.body.entries,
                    &receipt_json,
                )?
                .receipt_json
        }
        "transfer_cancel" => {
            let envelope: TransferCancelEnvelope = serde_json::from_value(envelope_value)?;
            if envelope.version != PROTOCOL_VERSION
                || envelope.frame_type != "transfer_cancel"
                || envelope.sender_device_id != hello.device_id
                || envelope.body.cancelled_by != envelope.sender_device_id
            {
                return Err(ControlError::Protocol("invalid transfer_cancel"));
            }
            let receipt_json = delivery_receipt_json(
                profile.device_id,
                envelope.event_id,
                None,
                Some(envelope.body.transfer_id),
                "stored",
            );
            let saved = storage.receive_transfer_cancel(
                envelope.sender_device_id,
                envelope.event_id,
                envelope.body.transfer_id,
                &receipt_json,
            )?;
            stop_active_transfer(envelope.body.transfer_id);
            saved
        }
        _ => return Err(ControlError::Protocol("unsupported control event type")),
    };
    let saved_value: Value = serde_json::from_str(&saved_receipt)?;
    write_json_frame(stream, &saved_value)?;
    if let Some(kind) = core_event_kind_for_control(&frame_type) {
        events::publish(kind, event_entity_id);
    }
    Ok(())
}

fn handle_completed_receipt(
    storage: &mut Storage,
    hello: &Hello,
    envelope: &ReceiptEnvelope,
) -> Result<(), ControlError> {
    if envelope.version != PROTOCOL_VERSION
        || envelope.frame_type != "delivery_receipt"
        || envelope.sender_device_id != hello.device_id
        || envelope.body.stage != "completed"
    {
        return Err(ControlError::Protocol("invalid completed delivery_receipt"));
    }
    let transfer_id = envelope.body.transfer_id.ok_or(ControlError::Protocol(
        "completed receipt is missing transfer_id",
    ))?;
    storage.receive_transfer_completed(
        envelope.sender_device_id,
        envelope.event_id,
        transfer_id,
    )?;
    events::publish(
        CoreEventKind::TransferProgress,
        Some(transfer_id.to_string()),
    );
    Ok(())
}

fn run_connection_actor(
    stream: &mut TcpStream,
    hello: &Hello,
    storage: &mut Storage,
    profile: &LocalProfile,
    stop: &Arc<AtomicBool>,
    commands: Receiver<ConnectionCommand>,
) -> Result<(), ControlError> {
    let mut last_activity = Instant::now();
    let mut pending_ping = None;
    while !stop.load(Ordering::Acquire) {
        match commands.try_recv() {
            Ok(ConnectionCommand::Deliver { entry, result }) => {
                match deliver_outbox_bidirectional(
                    storage,
                    &entry,
                    stream,
                    hello,
                    profile,
                    &mut pending_ping,
                ) {
                    Ok(()) => {
                        let _ = result.send(Ok(()));
                        last_activity = Instant::now();
                    }
                    Err(ControlError::RemoteProtocol { code, message }) => {
                        let _ = result.send(Err(DeliveryFailure::Protocol { code, message }));
                        last_activity = Instant::now();
                    }
                    Err(error) => {
                        let _ = result.send(Err(DeliveryFailure::Connection(error.to_string())));
                        return Err(error);
                    }
                }
            }
            Ok(ConnectionCommand::Retire { kept_connection_id }) => {
                write_duplicate_connection(stream, kept_connection_id)?;
                return Ok(());
            }
            Err(TryRecvError::Disconnected | TryRecvError::Empty) => {}
        }

        if pending_ping.is_none() && last_activity.elapsed() >= PING_INTERVAL {
            let ping_id = EventId::generate();
            write_json_frame(
                &mut *stream,
                &PingFrame {
                    version: PROTOCOL_VERSION,
                    frame_type: "ping".to_owned(),
                    ping_id,
                    sent_at_ms: unix_time_ms(),
                },
            )?;
            pending_ping = Some(ping_id);
        }

        match read_json_frame::<_, Value>(&mut *stream, MAX_MANIFEST_JSON_BYTES) {
            Ok(value) => {
                last_activity = Instant::now();
                if value.get("type").and_then(Value::as_str) == Some("duplicate_connection") {
                    let duplicate: DuplicateConnectionFrame = serde_json::from_value(value)?;
                    if duplicate.version != PROTOCOL_VERSION
                        || duplicate.frame_type != "duplicate_connection"
                    {
                        return Err(ControlError::Protocol("invalid duplicate_connection"));
                    }
                    return Ok(());
                }
                handle_actor_frame(stream, value, hello, storage, profile, &mut pending_ping)?;
            }
            Err(error) => {
                let error = ControlError::Frame(error);
                if !is_read_timeout(&error) {
                    if is_closed_connection(&error) {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        }

        if last_activity.elapsed() >= IDLE_CONNECTION_TIMEOUT {
            return Ok(());
        }
    }
    Ok(())
}

fn handle_actor_frame(
    stream: &mut TcpStream,
    value: Value,
    hello: &Hello,
    storage: &mut Storage,
    profile: &LocalProfile,
    pending_ping: &mut Option<EventId>,
) -> Result<(), ControlError> {
    match value.get("type").and_then(Value::as_str) {
        Some("pong") => {
            let pong: PingFrame = serde_json::from_value(value)?;
            if pong.version != PROTOCOL_VERSION
                || pong.frame_type != "pong"
                || pending_ping.take() != Some(pong.ping_id)
            {
                return Err(ControlError::Protocol("invalid pong"));
            }
            Ok(())
        }
        Some("protocol_error") => {
            let error: ProtocolErrorFrame = serde_json::from_value(value)?;
            if error.version != PROTOCOL_VERSION
                || error.frame_type != "protocol_error"
                || !is_known_protocol_error_code(&error.code)
            {
                return Err(ControlError::Protocol("invalid protocol_error"));
            }
            Err(ControlError::RemoteProtocol {
                code: error.code,
                message: error.message,
            })
        }
        _ => {
            let related_event_id = value
                .get("event_id")
                .and_then(Value::as_str)
                .and_then(|value| EventId::from_str(value).ok());
            match handle_control_event_value(stream, value, hello, storage, profile) {
                Ok(()) => Ok(()),
                Err(error) => {
                    let Some(code) = protocol_error_code(&error) else {
                        return Err(error);
                    };
                    write_protocol_error(stream, code, related_event_id, &error.to_string())?;
                    if related_event_id.is_some() {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
            }
        }
    }
}

fn protocol_error_code(error: &ControlError) -> Option<&'static str> {
    match error {
        ControlError::Protocol(_) | ControlError::Json(_) => Some("PROTOCOL_INVALID_FRAME"),
        ControlError::Storage(error) => match error {
            StorageError::DirectoryUnwritable(_) => Some("CONFIG_DIRECTORY_UNWRITABLE"),
            StorageError::Open(_) => Some("STORAGE_OPEN_FAILED"),
            StorageError::Migration(_) => Some("STORAGE_MIGRATION_FAILED"),
            StorageError::Write(_) | StorageError::Serialization(_) => Some("STORAGE_WRITE_FAILED"),
            StorageError::InvalidText => Some("MESSAGE_TOO_LARGE"),
            StorageError::Manifest(_) | StorageError::TransferSourceMismatch => {
                Some("FILE_INVALID_PATH")
            }
            StorageError::PeerNotFound => Some("PEER_UNKNOWN"),
            StorageError::ConversationNotFound
            | StorageError::InviteNotFound
            | StorageError::BindingNotFound
            | StorageError::TransferNotFound => Some("NOT_FOUND"),
            StorageError::GroupNotFound => Some("GROUP_NOT_FOUND"),
            StorageError::NotGroupOwner => Some("NOT_GROUP_OWNER"),
            StorageError::GroupRevisionConflict => Some("GROUP_REVISION_CONFLICT"),
            StorageError::GroupFull => Some("GROUP_FULL"),
            StorageError::GroupOwnerMustTransfer => Some("GROUP_OWNER_MUST_TRANSFER"),
            StorageError::ClipboardBindingRequired => Some("CLIPBOARD_BINDING_REQUIRED"),
            StorageError::ClipboardTooLarge => Some("CLIPBOARD_TOO_LARGE"),
            StorageError::InvalidStoredDeviceId
            | StorageError::InvalidDeviceName
            | StorageError::InvalidGroupName
            | StorageError::InvalidGroupMembers
            | StorageError::InvalidGroupUpdate
            | StorageError::InvalidClipboardMode
            | StorageError::InvalidClipboardImageMetadata
            | StorageError::InvalidReceivePolicy
            | StorageError::InvalidLogLevel
            | StorageError::ClipboardSequenceExhausted
            | StorageError::GroupSequenceExhausted
            | StorageError::InvalidStoredId
            | StorageError::UnsupportedSchema(_) => Some("INVALID_ARGUMENT"),
        },
        ControlError::Bind(_)
        | ControlError::Io(_)
        | ControlError::Frame(_)
        | ControlError::Data(_)
        | ControlError::RemoteProtocol { .. } => None,
    }
}

fn remote_protocol_should_retry(code: &str) -> bool {
    matches!(
        code,
        "NOT_READY"
            | "STORAGE_OPEN_FAILED"
            | "STORAGE_MIGRATION_FAILED"
            | "STORAGE_WRITE_FAILED"
            | "NETWORK_CONNECTION_LOST"
            | "GROUP_REVISION_CONFLICT"
    )
}

fn is_known_protocol_error_code(code: &str) -> bool {
    matches!(
        code,
        "INVALID_ARGUMENT"
            | "NOT_READY"
            | "ALREADY_EXISTS"
            | "NOT_FOUND"
            | "CONFIG_DIRECTORY_UNWRITABLE"
            | "STORAGE_OPEN_FAILED"
            | "STORAGE_MIGRATION_FAILED"
            | "STORAGE_WRITE_FAILED"
            | "NETWORK_UDP_BIND_FAILED"
            | "NETWORK_TCP_BIND_FAILED"
            | "NETWORK_CONNECT_FAILED"
            | "NETWORK_CONNECTION_LOST"
            | "UNSUPPORTED_PROTOCOL"
            | "PROTOCOL_INVALID_FRAME"
            | "PROTOCOL_LIMIT_EXCEEDED"
            | "PEER_OFFLINE"
            | "PEER_UNKNOWN"
            | "MESSAGE_TOO_LARGE"
            | "GROUP_FULL"
            | "GROUP_NOT_FOUND"
            | "NOT_GROUP_OWNER"
            | "GROUP_OWNER_MUST_TRANSFER"
            | "GROUP_REVISION_CONFLICT"
            | "TRANSFER_REJECTED"
            | "TRANSFER_SOURCE_CHANGED"
            | "TRANSFER_CANCELLED"
            | "FILE_NOT_FOUND"
            | "FILE_OPEN_FAILED"
            | "FILE_WRITE_FAILED"
            | "FILE_NOT_ENOUGH_SPACE"
            | "FILE_INVALID_PATH"
            | "FILE_PERMISSION_LOST"
            | "PLATFORM_REQUEST_TIMEOUT"
            | "PLATFORM_UNSUPPORTED"
            | "NOTIFICATION_PERMISSION_REQUIRED"
            | "CLIPBOARD_EMPTY"
            | "CLIPBOARD_TOO_LARGE"
            | "CLIPBOARD_BINDING_REQUIRED"
            | "CLIPBOARD_BACKGROUND_READ_BLOCKED"
            | "INTERNAL_ERROR"
    )
}

fn deliver_outbox_bidirectional(
    storage: &mut Storage,
    entry: &PendingOutbox,
    stream: &mut TcpStream,
    hello: &Hello,
    profile: &LocalProfile,
    pending_ping: &mut Option<EventId>,
) -> Result<(), ControlError> {
    let payload: Value = serde_json::from_str(&entry.payload_json)?;
    let event_entity_id = control_event_entity_id(&payload);
    write_outbox_frame(&mut *stream, &payload, &entry.event_type)?;
    if !entry.requires_receipt {
        storage.mark_control_event_stored(entry.peer_device_id, entry.event_id)?;
        if let Some(kind) = outbox_event_kind(&entry.event_type) {
            events::publish(kind, event_entity_id);
        }
        return Ok(());
    }

    let deadline = Instant::now() + RECEIPT_TIMEOUT;
    loop {
        match read_json_frame::<_, Value>(&mut *stream, MAX_MANIFEST_JSON_BYTES) {
            Ok(value) => match value.get("type").and_then(Value::as_str) {
                Some("delivery_receipt") => {
                    let receipt: ReceiptEnvelope = serde_json::from_value(value)?;
                    if receipt.body.stage == "stored"
                        && receipt.body.original_event_id == entry.event_id
                    {
                        apply_outbox_receipt(storage, entry, &receipt, event_entity_id)?;
                        return Ok(());
                    }
                    handle_completed_receipt(storage, hello, &receipt)?;
                }
                Some("protocol_error") => {
                    let error: ProtocolErrorFrame = serde_json::from_value(value)?;
                    if error.version != PROTOCOL_VERSION
                        || error.frame_type != "protocol_error"
                        || error.related_event_id != Some(entry.event_id)
                        || !is_known_protocol_error_code(&error.code)
                    {
                        return Err(ControlError::Protocol("invalid protocol_error"));
                    }
                    return Err(ControlError::RemoteProtocol {
                        code: error.code,
                        message: error.message,
                    });
                }
                Some("duplicate_connection") => {
                    return Err(ControlError::Protocol("connection retired by peer"));
                }
                _ => handle_actor_frame(stream, value, hello, storage, profile, pending_ping)?,
            },
            Err(error) => {
                let error = ControlError::Frame(error);
                if !is_read_timeout(&error) {
                    return Err(error);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(ControlError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "stored receipt timed out",
            )));
        }
    }
}

fn is_read_timeout(error: &ControlError) -> bool {
    matches!(
        error,
        ControlError::Frame(FrameError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            )
    )
}

fn write_outbox_frame(
    stream: &mut TcpStream,
    payload: &Value,
    event_type: &str,
) -> Result<(), ControlError> {
    if matches!(event_type, "file_offer" | "file_accept" | "transfer_resume") {
        write_json_frame_with_limit(stream, payload, MAX_MANIFEST_JSON_BYTES)?;
    } else {
        write_json_frame(stream, payload)?;
    }
    Ok(())
}

fn is_closed_connection(error: &ControlError) -> bool {
    matches!(
        error,
        ControlError::Frame(FrameError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
            )
    )
}

fn core_event_kind_for_control(frame_type: &str) -> Option<CoreEventKind> {
    match frame_type {
        "text_message" | "clipboard_update" => Some(CoreEventKind::MessageChanged),
        "file_offer" => Some(CoreEventKind::IncomingOfferRequiresDecision),
        "group_invite" => Some(CoreEventKind::GroupInviteReceived),
        "own_device_bind" => Some(CoreEventKind::OwnDeviceBindingRequested),
        "file_accept" | "file_reject" | "transfer_pause" | "transfer_resume"
        | "transfer_cancel" => Some(CoreEventKind::TransferProgress),
        "group_invite_reply"
        | "group_update"
        | "group_sync_request"
        | "group_sync_response"
        | "group_leave_request"
        | "own_device_bind_reply"
        | "own_device_unbind" => Some(CoreEventKind::ConversationChanged),
        _ => None,
    }
}

fn control_event_entity_id(value: &Value) -> Option<String> {
    [
        "/body/message_id",
        "/body/transfer_id",
        "/body/invite_id",
        "/body/binding_id",
        "/body/group_id",
        "/body/group/group_id",
    ]
    .into_iter()
    .find_map(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn connect_outgoing_stream(
    profile: &LocalProfile,
    peer_device_id: DeviceId,
    address: SocketAddr,
) -> Result<(TcpStream, Hello, ConnectionKey), ControlError> {
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
    configure_stream(&stream)?;
    let connection_id = EventId::generate();
    write_json_frame(
        &mut stream,
        &Hello {
            version: PROTOCOL_VERSION,
            frame_type: "hello".to_owned(),
            connection_id,
            initiator_device_id: profile.device_id,
            device_id: profile.device_id,
            device_name: profile.device_name.clone(),
            platform: profile.platform,
            app_version: CORE_VERSION.to_owned(),
            protocol_min: PROTOCOL_VERSION,
            protocol_max: PROTOCOL_VERSION,
            sent_at_ms: unix_time_ms(),
        },
    )?;
    let ack: HelloAck = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES)?;
    if ack.version != PROTOCOL_VERSION
        || ack.frame_type != "hello_ack"
        || ack.connection_id != connection_id
        || ack.device_id != peer_device_id
        || ack.selected_protocol != PROTOCOL_VERSION
    {
        return Err(ControlError::Protocol("hello_ack does not match the peer"));
    }
    let remote = Hello {
        version: PROTOCOL_VERSION,
        frame_type: "hello".to_owned(),
        connection_id,
        initiator_device_id: profile.device_id,
        device_id: ack.device_id,
        device_name: ack.device_name,
        platform: ack.platform,
        app_version: String::new(),
        protocol_min: PROTOCOL_VERSION,
        protocol_max: PROTOCOL_VERSION,
        sent_at_ms: ack.received_at_ms,
    };
    Ok((
        stream,
        remote,
        ConnectionKey {
            initiator_device_id: profile.device_id,
            connection_id,
        },
    ))
}

fn start_outgoing_actor(
    profile: &LocalProfile,
    peer_device_id: DeviceId,
    address: SocketAddr,
    database_path: &Path,
    connections: &Arc<ConnectionRegistry>,
    stop: &Arc<AtomicBool>,
) -> Result<(), ControlError> {
    let (mut stream, hello, key) = connect_outgoing_stream(profile, peer_device_id, address)?;
    let (commands, receiver) = mpsc::channel();
    match connections.register(
        peer_device_id,
        ConnectionHandle {
            key,
            address,
            commands,
        },
    ) {
        RegisterConnection::Rejected { kept_connection_id } => {
            write_duplicate_connection(&mut stream, kept_connection_id)?;
            return Ok(());
        }
        RegisterConnection::Accepted => {}
    }

    stream.set_read_timeout(Some(CONNECTION_ACTOR_POLL_INTERVAL))?;
    let actor_profile = profile.clone();
    let actor_path = database_path.to_path_buf();
    let actor_connections = Arc::clone(connections);
    let actor_stop = Arc::clone(stop);
    thread::Builder::new()
        .name("lan-chat-control-actor".to_owned())
        .spawn(move || {
            let result = Storage::open(&actor_path)
                .map_err(ControlError::from)
                .and_then(|mut storage| {
                    run_connection_actor(
                        &mut stream,
                        &hello,
                        &mut storage,
                        &actor_profile,
                        &actor_stop,
                        receiver,
                    )
                });
            actor_connections.remove_if_current(peer_device_id, key);
            if let Err(error) = result {
                record_control_error(
                    &actor_path,
                    &format!("connection actor peer={peer_device_id}: {error}"),
                );
            }
        })
        .map_err(ControlError::Io)?;
    Ok(())
}

fn deliver_outbox_managed(
    profile: &LocalProfile,
    database_path: &Path,
    entry: &PendingOutbox,
    connections: &Arc<ConnectionRegistry>,
    stop: &Arc<AtomicBool>,
) -> Result<(), ControlError> {
    let ip = Ipv4Addr::from_str(&entry.last_ip)
        .map_err(|_| ControlError::Protocol("peer address is not valid IPv4"))?;
    let address = SocketAddr::V4(SocketAddrV4::new(ip, CONTROL_PORT));
    connections.retire_if_address_changed(entry.peer_device_id, address);
    if connections.current(entry.peer_device_id).is_none() {
        start_outgoing_actor(
            profile,
            entry.peer_device_id,
            address,
            database_path,
            connections,
            stop,
        )?;
    }
    let connection = connections
        .current(entry.peer_device_id)
        .ok_or(ControlError::Protocol(
            "control connection was not registered",
        ))?;
    let (result_sender, result_receiver) = mpsc::sync_channel(1);
    connection
        .commands
        .send(ConnectionCommand::Deliver {
            entry: entry.clone(),
            result: result_sender,
        })
        .map_err(|_| ControlError::Protocol("control connection actor stopped"))?;
    match result_receiver.recv_timeout(RECEIPT_TIMEOUT + FRAME_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(DeliveryFailure::Connection(message))) => {
            Err(ControlError::Io(io::Error::other(message)))
        }
        Ok(Err(DeliveryFailure::Protocol { code, message })) => {
            Err(ControlError::RemoteProtocol { code, message })
        }
        Err(_) => {
            connections.remove_if_current(entry.peer_device_id, connection.key);
            Err(ControlError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "control connection actor timed out",
            )))
        }
    }
}

#[cfg(test)]
struct OutgoingControlConnection {
    stream: TcpStream,
    last_activity: Instant,
}

#[cfg(test)]
impl OutgoingControlConnection {
    fn connect(
        profile: &LocalProfile,
        peer_device_id: DeviceId,
        address: SocketAddr,
    ) -> Result<Self, ControlError> {
        let (stream, _, _) = connect_outgoing_stream(profile, peer_device_id, address)?;
        Ok(Self {
            stream,
            last_activity: Instant::now(),
        })
    }

    fn deliver(
        &mut self,
        storage: &mut Storage,
        entry: &PendingOutbox,
    ) -> Result<(), ControlError> {
        deliver_outbox_on_stream(storage, entry, &mut self.stream)?;
        self.last_activity = Instant::now();
        Ok(())
    }

    fn ping(&mut self) -> Result<(), ControlError> {
        let ping_id = EventId::generate();
        write_json_frame(
            &mut self.stream,
            &PingFrame {
                version: PROTOCOL_VERSION,
                frame_type: "ping".to_owned(),
                ping_id,
                sent_at_ms: unix_time_ms(),
            },
        )?;
        let pong: PingFrame = read_json_frame(&mut self.stream, MAX_CONTROL_FRAME_BYTES)?;
        if pong.version != PROTOCOL_VERSION || pong.frame_type != "pong" || pong.ping_id != ping_id
        {
            return Err(ControlError::Protocol("invalid pong"));
        }
        self.last_activity = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
fn deliver_outbox_on_stream(
    storage: &mut Storage,
    entry: &PendingOutbox,
    stream: &mut TcpStream,
) -> Result<(), ControlError> {
    let payload: Value = serde_json::from_str(&entry.payload_json)?;
    let event_entity_id = control_event_entity_id(&payload);
    write_outbox_frame(stream, &payload, &entry.event_type)?;
    if !entry.requires_receipt {
        storage.mark_control_event_stored(entry.peer_device_id, entry.event_id)?;
        if let Some(kind) = outbox_event_kind(&entry.event_type) {
            events::publish(kind, event_entity_id);
        }
        return Ok(());
    }
    let receipt: ReceiptEnvelope = read_json_frame(stream, MAX_CONTROL_FRAME_BYTES)?;
    apply_outbox_receipt(storage, entry, &receipt, event_entity_id)
}

fn apply_outbox_receipt(
    storage: &mut Storage,
    entry: &PendingOutbox,
    receipt: &ReceiptEnvelope,
    event_entity_id: Option<String>,
) -> Result<(), ControlError> {
    if receipt.version != PROTOCOL_VERSION
        || receipt.frame_type != "delivery_receipt"
        || receipt.sender_device_id != entry.peer_device_id
        || receipt.body.original_event_id != entry.event_id
        || receipt.body.stage != "stored"
    {
        return Err(ControlError::Protocol("invalid delivery receipt"));
    }
    match entry.event_type.as_str() {
        "text_message" | "clipboard_update" => storage.mark_text_delivered(
            entry.peer_device_id,
            receipt.body.original_event_id,
            receipt
                .body
                .message_id
                .ok_or(ControlError::Protocol("text receipt is missing message_id"))?,
        )?,
        "file_offer" => storage.mark_file_offer_stored(
            entry.peer_device_id,
            receipt.body.original_event_id,
            receipt.body.message_id.ok_or(ControlError::Protocol(
                "offer receipt is missing message_id",
            ))?,
            receipt.body.transfer_id.ok_or(ControlError::Protocol(
                "offer receipt is missing transfer_id",
            ))?,
        )?,
        "file_accept"
        | "file_reject"
        | "transfer_pause"
        | "transfer_resume"
        | "transfer_cancel"
        | "group_invite"
        | "group_invite_reply"
        | "group_update"
        | "group_sync_request"
        | "group_sync_response"
        | "group_leave_request"
        | "own_device_bind"
        | "own_device_bind_reply"
        | "own_device_unbind" => storage
            .mark_control_event_stored(entry.peer_device_id, receipt.body.original_event_id)?,
        _ => return Err(ControlError::Protocol("unsupported outbox event type")),
    }
    if let Some(kind) = outbox_event_kind(&entry.event_type) {
        events::publish(kind, event_entity_id);
    }
    Ok(())
}

fn outbox_event_kind(event_type: &str) -> Option<CoreEventKind> {
    match event_type {
        "text_message" | "clipboard_update" => Some(CoreEventKind::MessageChanged),
        "file_offer" | "file_accept" | "file_reject" | "transfer_pause" | "transfer_resume"
        | "transfer_cancel" | "delivery_receipt" => Some(CoreEventKind::TransferProgress),
        "group_invite"
        | "group_invite_reply"
        | "group_update"
        | "group_sync_request"
        | "group_sync_response"
        | "group_leave_request"
        | "own_device_bind"
        | "own_device_bind_reply"
        | "own_device_unbind" => Some(CoreEventKind::ConversationChanged),
        _ => None,
    }
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    // Accepted sockets inherit the listener's nonblocking mode on Windows.
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(FRAME_TIMEOUT))?;
    stream.set_write_timeout(Some(FRAME_TIMEOUT))?;
    Ok(())
}

fn write_protocol_error(
    stream: &mut TcpStream,
    code: &str,
    related_event_id: Option<EventId>,
    message: &str,
) -> Result<(), ControlError> {
    write_json_frame(
        stream,
        &ProtocolErrorFrame {
            version: PROTOCOL_VERSION,
            frame_type: "protocol_error".to_owned(),
            code: code.to_owned(),
            related_event_id,
            message: truncate_utf8(message, 256),
        },
    )?;
    Ok(())
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn write_duplicate_connection(
    stream: &mut TcpStream,
    kept_connection_id: EventId,
) -> Result<(), ControlError> {
    write_json_frame(
        stream,
        &DuplicateConnectionFrame {
            version: PROTOCOL_VERSION,
            frame_type: "duplicate_connection".to_owned(),
            kept_connection_id,
        },
    )?;
    Ok(())
}

fn validate_hello(hello: &Hello, local_device_id: DeviceId) -> Result<(), ControlError> {
    if hello.version != PROTOCOL_VERSION
        || hello.frame_type != "hello"
        || hello.initiator_device_id != hello.device_id
        || hello.device_id == local_device_id
        || hello.protocol_min > PROTOCOL_VERSION
        || hello.protocol_max < PROTOCOL_VERSION
        || hello.device_name.is_empty()
        || hello.device_name.chars().count() > 32
        || hello.device_name.len() > 128
    {
        return Err(ControlError::Protocol("invalid hello"));
    }
    Ok(())
}

fn validate_text_envelope(
    envelope: &TextEnvelope,
    hello_device_id: DeviceId,
) -> Result<(), ControlError> {
    if envelope.version != PROTOCOL_VERSION
        || envelope.frame_type != "text_message"
        || envelope.sender_device_id != hello_device_id
        || !match envelope.body.message_kind.as_str() {
            "text" => (1..=20_000).contains(&envelope.body.text.chars().count()),
            "clipboard_text" => {
                !envelope.body.text.is_empty() && envelope.body.text.len() <= 1024 * 1024
            }
            _ => false,
        }
    {
        return Err(ControlError::Protocol("invalid text_message"));
    }
    Ok(())
}

fn validate_file_offer(
    envelope: &FileOfferEnvelope,
    hello_device_id: DeviceId,
) -> Result<(), ControlError> {
    let metadata_complete = envelope.body.origin_device_id.is_some()
        && envelope.body.clipboard_sequence.is_some()
        && envelope.body.content_fingerprint.is_some()
        && envelope.body.automatic.is_some();
    let metadata_empty = envelope.body.origin_device_id.is_none()
        && envelope.body.clipboard_sequence.is_none()
        && envelope.body.content_fingerprint.is_none()
        && envelope.body.automatic.is_none();
    let valid_clipboard_metadata = envelope.body.message_kind == MessageKind::ClipboardImage
        && metadata_complete
        && envelope.body.origin_device_id == Some(envelope.sender_device_id)
        && envelope
            .body
            .clipboard_sequence
            .is_some_and(|value| value > 0)
        && envelope
            .body
            .content_fingerprint
            .as_deref()
            .is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
    let valid_regular_metadata =
        envelope.body.message_kind != MessageKind::ClipboardImage && metadata_empty;
    if envelope.version != PROTOCOL_VERSION
        || envelope.frame_type != "file_offer"
        || envelope.sender_device_id != hello_device_id
        || (!valid_clipboard_metadata && !valid_regular_metadata)
    {
        return Err(ControlError::Protocol("invalid file_offer"));
    }
    Ok(())
}

fn delivery_receipt_json(
    sender_device_id: DeviceId,
    original_event_id: EventId,
    message_id: Option<MessageId>,
    transfer_id: Option<TransferId>,
    stage: &str,
) -> String {
    let mut body = serde_json::json!({
        "original_event_id": original_event_id,
        "stage": stage,
        "at_ms": unix_time_ms(),
    });
    if let Some(message_id) = message_id {
        body["message_id"] = serde_json::json!(message_id);
    }
    if let Some(transfer_id) = transfer_id {
        body["transfer_id"] = serde_json::json!(transfer_id);
    }
    serde_json::json!({
        "version": PROTOCOL_VERSION,
        "type": "delivery_receipt",
        "event_id": EventId::generate(),
        "sender_device_id": sender_device_id,
        "sent_at_ms": unix_time_ms(),
        "body": body,
    })
    .to_string()
}

fn unix_time_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn record_control_error(database_path: &Path, message: &str) {
    events::publish(CoreEventKind::CoreErrorOccurred, None);
    let Some(directory) = database_path.parent() else {
        return;
    };
    let path = directory.join("lan_chat.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} control {message}", unix_time_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_test_peers(
        left: &mut Storage,
        left_profile: &LocalProfile,
        right: &mut Storage,
        right_profile: &LocalProfile,
    ) {
        left.upsert_nearby_peer(
            right_profile.device_id,
            &right_profile.device_name,
            right_profile.platform,
            "127.0.0.1",
            1,
        )
        .unwrap();
        right
            .upsert_nearby_peer(
                left_profile.device_id,
                &left_profile.device_name,
                left_profile.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
    }

    fn deliver_test_entry(
        sender_storage: &mut Storage,
        sender: &LocalProfile,
        receiver: &LocalProfile,
        receiver_path: &Path,
        entry: &PendingOutbox,
    ) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_device_id = receiver.device_id;
        let receiver = receiver.clone();
        let receiver_path = receiver_path.to_path_buf();
        let worker = thread::spawn(move || {
            let (mut stream, source) = listener.accept().unwrap();
            configure_stream(&stream).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            handle_incoming(
                &mut stream,
                source,
                &receiver,
                &receiver_path,
                &stop,
                &Arc::new(ConnectionRegistry::default()),
            )
            .unwrap();
        });
        let mut connection =
            OutgoingControlConnection::connect(sender, receiver_device_id, address).unwrap();
        connection.deliver(sender_storage, entry).unwrap();
        drop(connection);
        worker.join().unwrap();
    }

    fn pending_test_entry(storage: &Storage, event_type: &str, peer: DeviceId) -> PendingOutbox {
        storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == event_type && entry.peer_device_id == peer)
            .unwrap_or_else(|| panic!("missing {event_type} for {peer}"))
    }

    fn hello_for_remote(
        local: &LocalProfile,
        remote: &LocalProfile,
        connection_id: EventId,
    ) -> Hello {
        Hello {
            version: PROTOCOL_VERSION,
            frame_type: "hello".to_owned(),
            connection_id,
            initiator_device_id: local.device_id,
            device_id: remote.device_id,
            device_name: remote.device_name.clone(),
            platform: remote.platform,
            app_version: CORE_VERSION.to_owned(),
            protocol_min: PROTOCOL_VERSION,
            protocol_max: PROTOCOL_VERSION,
            sent_at_ms: unix_time_ms(),
        }
    }

    #[test]
    fn connection_registry_keeps_the_smallest_deterministic_key() {
        let registry = ConnectionRegistry::default();
        let peer = DeviceId::from_bytes([9; 16]);
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, CONTROL_PORT));
        let (high_sender, high_receiver) = mpsc::channel();
        let high_key = ConnectionKey {
            initiator_device_id: DeviceId::from_bytes([2; 16]),
            connection_id: EventId::generate(),
        };
        assert!(matches!(
            registry.register(
                peer,
                ConnectionHandle {
                    key: high_key,
                    address,
                    commands: high_sender,
                }
            ),
            RegisterConnection::Accepted
        ));

        let (low_sender, _low_receiver) = mpsc::channel();
        let low_key = ConnectionKey {
            initiator_device_id: DeviceId::from_bytes([1; 16]),
            connection_id: EventId::generate(),
        };
        assert!(matches!(
            registry.register(
                peer,
                ConnectionHandle {
                    key: low_key,
                    address,
                    commands: low_sender,
                }
            ),
            RegisterConnection::Accepted
        ));
        assert!(matches!(
            high_receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            ConnectionCommand::Retire { kept_connection_id }
                if kept_connection_id == low_key.connection_id
        ));

        let (worse_sender, _worse_receiver) = mpsc::channel();
        assert!(matches!(
            registry.register(
                peer,
                ConnectionHandle {
                    key: high_key,
                    address,
                    commands: worse_sender,
                }
            ),
            RegisterConnection::Rejected { kept_connection_id }
                if kept_connection_id == low_key.connection_id
        ));
        assert_eq!(registry.current(peer).unwrap().key, low_key);
    }

    #[test]
    fn one_control_socket_delivers_simultaneous_bidirectional_messages() {
        let temp = tempfile::tempdir().unwrap();
        let left_path = temp.path().join("left.db");
        let right_path = temp.path().join("right.db");
        let mut left_storage = Storage::open(&left_path).unwrap();
        let mut right_storage = Storage::open(&right_path).unwrap();
        let left = left_storage
            .load_or_create_profile("Left", Platform::Windows)
            .unwrap();
        let right = right_storage
            .load_or_create_profile("Right", Platform::Android)
            .unwrap();
        connect_test_peers(&mut left_storage, &left, &mut right_storage, &right);
        let left_conversation = left_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                left.device_id,
                right.device_id,
            )
            .unwrap();
        let right_conversation = right_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                right.device_id,
                left.device_id,
            )
            .unwrap();
        let left_message = left_storage
            .create_outgoing_text(&left, &left_conversation.conversation_id, "left to right")
            .unwrap();
        let right_message = right_storage
            .create_outgoing_text(&right, &right_conversation.conversation_id, "right to left")
            .unwrap();
        let left_entry = pending_test_entry(&left_storage, "text_message", right.device_id);
        let right_entry = pending_test_entry(&right_storage, "text_message", left.device_id);

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut left_stream = TcpStream::connect(address).unwrap();
        let (mut right_stream, _) = listener.accept().unwrap();
        configure_stream(&left_stream).unwrap();
        configure_stream(&right_stream).unwrap();
        left_stream
            .set_read_timeout(Some(CONNECTION_ACTOR_POLL_INTERVAL))
            .unwrap();
        right_stream
            .set_read_timeout(Some(CONNECTION_ACTOR_POLL_INTERVAL))
            .unwrap();

        let connection_id = EventId::generate();
        let left_hello = hello_for_remote(&left, &right, connection_id);
        let right_hello = hello_for_remote(&right, &left, connection_id);
        let stop = Arc::new(AtomicBool::new(false));
        let (left_commands, left_receiver) = mpsc::channel();
        let (right_commands, right_receiver) = mpsc::channel();
        let (left_result_sender, left_result_receiver) = mpsc::sync_channel(1);
        let (right_result_sender, right_result_receiver) = mpsc::sync_channel(1);
        left_commands
            .send(ConnectionCommand::Deliver {
                entry: left_entry,
                result: left_result_sender,
            })
            .unwrap();
        right_commands
            .send(ConnectionCommand::Deliver {
                entry: right_entry,
                result: right_result_sender,
            })
            .unwrap();

        let left_stop = Arc::clone(&stop);
        let left_worker = thread::spawn(move || {
            run_connection_actor(
                &mut left_stream,
                &left_hello,
                &mut left_storage,
                &left,
                &left_stop,
                left_receiver,
            )
        });
        let right_stop = Arc::clone(&stop);
        let right_worker = thread::spawn(move || {
            run_connection_actor(
                &mut right_stream,
                &right_hello,
                &mut right_storage,
                &right,
                &right_stop,
                right_receiver,
            )
        });

        assert_eq!(
            left_result_receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            Ok(())
        );
        assert_eq!(
            right_result_receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap(),
            Ok(())
        );
        stop.store(true, Ordering::Release);
        left_worker.join().unwrap().unwrap();
        right_worker.join().unwrap().unwrap();

        let left_storage = Storage::open(&left_path).unwrap();
        let right_storage = Storage::open(&right_path).unwrap();
        assert!(left_storage.pending_outbox().unwrap().is_empty());
        assert!(right_storage.pending_outbox().unwrap().is_empty());
        assert_eq!(
            left_storage
                .list_messages(&left_conversation.conversation_id, 100)
                .unwrap()
                .iter()
                .filter(|message| message.message_id == right_message.message_id)
                .count(),
            1
        );
        assert_eq!(
            right_storage
                .list_messages(&right_conversation.conversation_id, 100)
                .unwrap()
                .iter()
                .filter(|message| message.message_id == left_message.message_id)
                .count(),
            1
        );
    }

    #[test]
    fn business_error_returns_authoritative_code_and_keeps_connection_alive() {
        let temp = tempfile::tempdir().unwrap();
        let receiver_path = temp.path().join("receiver.db");
        let mut receiver_storage = Storage::open(&receiver_path).unwrap();
        let receiver = receiver_storage
            .load_or_create_profile("Receiver", Platform::Windows)
            .unwrap();
        let sender = LocalProfile {
            device_id: DeviceId::from_bytes([7; 16]),
            device_name: "Sender".to_owned(),
            platform: Platform::Android,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        receiver_storage
            .upsert_nearby_peer(
                sender.device_id,
                &sender.device_name,
                sender.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(address).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        configure_stream(&client).unwrap();
        configure_stream(&server).unwrap();
        server
            .set_read_timeout(Some(CONNECTION_ACTOR_POLL_INTERVAL))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let actor_stop = Arc::clone(&stop);
        let hello = hello_for_remote(&receiver, &sender, EventId::generate());
        let (_commands, receiver_commands) = mpsc::channel();
        let worker = thread::spawn(move || {
            run_connection_actor(
                &mut server,
                &hello,
                &mut receiver_storage,
                &receiver,
                &actor_stop,
                receiver_commands,
            )
        });

        let event_id = EventId::generate();
        let missing_group = GroupId::new(sender.device_id, 1).unwrap();
        write_json_frame(
            &mut client,
            &serde_json::json!({
                "version": PROTOCOL_VERSION,
                "type": "text_message",
                "event_id": event_id,
                "sender_device_id": sender.device_id,
                "sent_at_ms": unix_time_ms(),
                "body": {
                    "message_id": MessageId::generate(),
                    "conversation_id": missing_group,
                    "message_kind": "text",
                    "text": "missing group",
                    "created_at_ms": unix_time_ms(),
                    "group_revision": 1
                }
            }),
        )
        .unwrap();
        let error: ProtocolErrorFrame =
            read_json_frame(&mut client, MAX_CONTROL_FRAME_BYTES).unwrap();
        assert_eq!(error.code, "GROUP_NOT_FOUND");
        assert_eq!(error.related_event_id, Some(event_id));

        let ping_id = EventId::generate();
        write_json_frame(
            &mut client,
            &PingFrame {
                version: PROTOCOL_VERSION,
                frame_type: "ping".to_owned(),
                ping_id,
                sent_at_ms: unix_time_ms(),
            },
        )
        .unwrap();
        let pong: PingFrame = read_json_frame(&mut client, MAX_CONTROL_FRAME_BYTES).unwrap();
        assert_eq!(pong.frame_type, "pong");
        assert_eq!(pong.ping_id, ping_id);
        drop(client);
        stop.store(true, Ordering::Release);
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn terminal_protocol_error_clears_outbox_and_marks_delivery_failed() {
        let temp = tempfile::tempdir().unwrap();
        let mut sender_storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let mut receiver_storage = Storage::open(temp.path().join("receiver.db")).unwrap();
        let sender = sender_storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let receiver = receiver_storage
            .load_or_create_profile("Receiver", Platform::Android)
            .unwrap();
        connect_test_peers(
            &mut sender_storage,
            &sender,
            &mut receiver_storage,
            &receiver,
        );
        let conversation = sender_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                sender.device_id,
                receiver.device_id,
            )
            .unwrap();
        let message = sender_storage
            .create_outgoing_text(&sender, &conversation.conversation_id, "will fail")
            .unwrap();
        let entry = pending_test_entry(&sender_storage, "text_message", receiver.device_id);
        sender_storage
            .mark_outbox_protocol_failure(receiver.device_id, entry.event_id, "group_not_found")
            .unwrap();

        assert!(sender_storage.pending_outbox().unwrap().is_empty());
        let deliveries = sender_storage
            .list_message_deliveries(message.message_id)
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].state, "failed");
        assert_eq!(
            deliveries[0].failure_reason.as_deref(),
            Some("group_not_found")
        );
        let stored = sender_storage
            .list_messages(&conversation.conversation_id, 10)
            .unwrap();
        assert_eq!(stored[0].state, "failed");
    }

    #[test]
    fn protocol_error_message_limit_is_utf8_bytes() {
        let truncated = truncate_utf8(&"你".repeat(100), 256);
        assert!(truncated.len() <= 256);
        assert_eq!(truncated.chars().count(), 85);
    }

    #[test]
    fn control_frames_map_to_typed_core_events_and_entity_ids() {
        let message_id = MessageId::generate();
        let value = serde_json::json!({
            "type": "text_message",
            "body": { "message_id": message_id }
        });
        assert_eq!(
            core_event_kind_for_control("text_message"),
            Some(CoreEventKind::MessageChanged)
        );
        assert_eq!(
            control_event_entity_id(&value),
            Some(message_id.to_string())
        );
        assert_eq!(
            core_event_kind_for_control("file_offer"),
            Some(CoreEventKind::IncomingOfferRequiresDecision)
        );
        assert_eq!(core_event_kind_for_control("unknown"), None);
    }

    #[test]
    fn text_envelope_accepts_group_revision_for_group_routing() {
        let sender = DeviceId::from_bytes([1; 16]);
        let value = serde_json::json!({
            "version": 1,
            "type": "text_message",
            "event_id": EventId::generate(),
            "sender_device_id": sender,
            "sent_at_ms": 1,
            "body": {
                "message_id": MessageId::generate(),
                "conversation_id": format!("p:{}:{}", sender, DeviceId::from_bytes([2; 16])),
                "message_kind": "text",
                "text": "hello",
                "created_at_ms": 1,
                "group_revision": 3
            }
        });
        let envelope: TextEnvelope = serde_json::from_value(value).unwrap();
        assert!(validate_text_envelope(&envelope, sender).is_ok());
    }

    #[test]
    fn text_envelope_accepts_clipboard_text_and_enforces_byte_limit() {
        let sender = DeviceId::from_bytes([1; 16]);
        let value = serde_json::json!({
            "version": 1,
            "type": "text_message",
            "event_id": EventId::generate(),
            "sender_device_id": sender,
            "sent_at_ms": 1,
            "body": {
                "message_id": MessageId::generate(),
                "conversation_id": format!("p:{}:{}", sender, DeviceId::from_bytes([2; 16])),
                "message_kind": "clipboard_text",
                "text": "clipboard",
                "created_at_ms": 1
            }
        });
        let envelope: TextEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert!(validate_text_envelope(&envelope, sender).is_ok());

        let mut oversized = value;
        oversized["body"]["text"] = serde_json::Value::String("x".repeat(1024 * 1024 + 1));
        let envelope: TextEnvelope = serde_json::from_value(oversized).unwrap();
        assert!(validate_text_envelope(&envelope, sender).is_err());
    }

    #[test]
    fn clipboard_image_offer_requires_complete_origin_metadata() {
        let sender = DeviceId::from_bytes([1; 16]);
        let mut value = serde_json::json!({
            "version": 1,
            "type": "file_offer",
            "event_id": EventId::generate(),
            "sender_device_id": sender,
            "body": {
                "message_id": MessageId::generate(),
                "transfer_id": TransferId::generate(),
                "conversation_id": format!("p:{}:{}", sender, DeviceId::from_bytes([2; 16])),
                "message_kind": "clipboard_image",
                "display_name": "clipboard.png",
                "total_size": 4,
                "entry_count": 1,
                "created_at_ms": 1,
                "entries": [{
                    "entry_id": crate::domain::EntryId::generate(),
                    "entry_kind": "file",
                    "relative_path": "clipboard.png",
                    "size": 4,
                    "modified_at_ms": 1
                }]
            }
        });
        let envelope: FileOfferEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert!(validate_file_offer(&envelope, sender).is_err());

        value["body"]["origin_device_id"] = serde_json::json!(sender);
        value["body"]["clipboard_sequence"] = serde_json::json!(1);
        value["body"]["content_fingerprint"] = serde_json::json!("a".repeat(64));
        value["body"]["automatic"] = serde_json::json!(true);
        let envelope: FileOfferEnvelope = serde_json::from_value(value).unwrap();
        assert!(validate_file_offer(&envelope, sender).is_ok());
    }

    #[test]
    fn accepted_stream_waits_for_a_delayed_first_frame() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            thread::sleep(Duration::from_millis(100));
            write_json_frame(&mut stream, &serde_json::json!({"type": "delayed"})).unwrap();
        });

        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        configure_stream(&stream).unwrap();
        let frame: Value = read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES).unwrap();

        sender.join().unwrap();
        assert_eq!(frame["type"], "delayed");
    }

    #[test]
    fn unsupported_hello_returns_authoritative_protocol_error() {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("receiver.db");
        let mut storage = Storage::open(&database_path).unwrap();
        let receiver = storage
            .load_or_create_profile("Receiver", Platform::Android)
            .unwrap();
        drop(storage);
        let sender = LocalProfile {
            device_id: DeviceId::generate(),
            device_name: "Future Sender".to_owned(),
            platform: Platform::Windows,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = receiver.clone();
        let database_for_thread = database_path.clone();
        let worker = thread::spawn(move || {
            let (mut stream, source) = listener.accept().unwrap();
            configure_stream(&stream).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            assert!(matches!(
                handle_incoming(
                    &mut stream,
                    source,
                    &receiver_for_thread,
                    &database_for_thread,
                    &stop,
                    &Arc::new(ConnectionRegistry::default()),
                ),
                Err(ControlError::Protocol("unsupported protocol version"))
            ));
        });

        let mut stream = TcpStream::connect(address).unwrap();
        configure_stream(&stream).unwrap();
        let connection_id = EventId::generate();
        write_json_frame(
            &mut stream,
            &Hello {
                version: 2,
                frame_type: "hello".to_owned(),
                connection_id,
                initiator_device_id: sender.device_id,
                device_id: sender.device_id,
                device_name: sender.device_name,
                platform: sender.platform,
                app_version: "2.0.0".to_owned(),
                protocol_min: 2,
                protocol_max: 2,
                sent_at_ms: 1,
            },
        )
        .unwrap();
        let error: ProtocolErrorFrame =
            read_json_frame(&mut stream, MAX_CONTROL_FRAME_BYTES).unwrap();
        assert_eq!(error.frame_type, "protocol_error");
        assert_eq!(error.code, "UNSUPPORTED_PROTOCOL");
        assert_eq!(error.related_event_id, Some(connection_id));
        worker.join().unwrap();
    }

    #[test]
    fn persistent_loopback_delivers_multiple_messages_and_ping_on_one_handshake() {
        let temp = tempfile::tempdir().unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
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
        sender_storage
            .create_outgoing_text(&sender, &conversation.conversation_id, "真机前先回环")
            .unwrap();
        sender_storage
            .create_outgoing_text(&sender, &conversation.conversation_id, "同一连接第二条")
            .unwrap();
        let entries = sender_storage.pending_outbox().unwrap();

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = receiver.clone();
        let receiver_path_for_thread = receiver_path.clone();
        let worker = thread::spawn(move || {
            let (mut stream, source) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            configure_stream(&stream).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            handle_incoming(
                &mut stream,
                source,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &stop,
                &Arc::new(ConnectionRegistry::default()),
            )
            .unwrap();
        });
        let mut connection =
            OutgoingControlConnection::connect(&sender, receiver.device_id, address).unwrap();
        for entry in &entries {
            connection.deliver(&mut sender_storage, entry).unwrap();
        }
        connection.ping().unwrap();
        drop(connection);
        worker.join().unwrap();

        assert!(sender_storage.pending_outbox().unwrap().is_empty());
        assert_eq!(
            sender_storage
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()[0]
                .state,
            "delivered"
        );
        let received = Storage::open(&receiver_path)
            .unwrap()
            .list_messages(&conversation.conversation_id, 100)
            .unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].text, "真机前先回环");
        assert_eq!(received[1].text, "同一连接第二条");
    }

    #[test]
    fn lost_receipt_and_sender_restart_redeliver_without_duplicate_message() {
        let temp = tempfile::tempdir().unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
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
        let outgoing = sender_storage
            .create_outgoing_text(&sender, &conversation.conversation_id, "回执会丢失")
            .unwrap();
        let entry = sender_storage.pending_outbox().unwrap().remove(0);
        drop(receiver_storage);

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = receiver.clone();
        let receiver_path_for_thread = receiver_path.clone();
        let first_receiver = thread::spawn(move || {
            let (mut stream, source) = listener.accept().unwrap();
            configure_stream(&stream).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _ = handle_incoming(
                &mut stream,
                source,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &stop,
                &Arc::new(ConnectionRegistry::default()),
            );
        });
        let mut first_connection =
            OutgoingControlConnection::connect(&sender, receiver.device_id, address).unwrap();
        let payload: Value = serde_json::from_str(&entry.payload_json).unwrap();
        write_json_frame(&mut first_connection.stream, &payload).unwrap();
        drop(first_connection);
        first_receiver.join().unwrap();

        assert_eq!(
            Storage::open(&receiver_path)
                .unwrap()
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(sender_storage.pending_outbox().unwrap().len(), 1);
        drop(sender_storage);

        let second_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let second_address = second_listener.local_addr().unwrap();
        let receiver_for_thread = receiver.clone();
        let receiver_path_for_thread = receiver_path.clone();
        let second_receiver = thread::spawn(move || {
            let (mut stream, source) = second_listener.accept().unwrap();
            configure_stream(&stream).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            handle_incoming(
                &mut stream,
                source,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &stop,
                &Arc::new(ConnectionRegistry::default()),
            )
            .unwrap();
        });
        let mut restarted_storage = Storage::open(&sender_path).unwrap();
        let restarted_entry = restarted_storage.pending_outbox().unwrap().remove(0);
        assert_eq!(restarted_entry.event_id, entry.event_id);
        let mut second_connection =
            OutgoingControlConnection::connect(&sender, receiver.device_id, second_address)
                .unwrap();
        second_connection
            .deliver(&mut restarted_storage, &restarted_entry)
            .unwrap();
        drop(second_connection);
        second_receiver.join().unwrap();

        assert!(restarted_storage.pending_outbox().unwrap().is_empty());
        assert_eq!(
            restarted_storage
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()[0]
                .state,
            "delivered"
        );
        let received = Storage::open(&receiver_path)
            .unwrap()
            .list_messages(&conversation.conversation_id, 100)
            .unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].message_id, outgoing.message_id);
        assert_eq!(received[0].text, "回执会丢失");
    }

    #[test]
    fn three_node_group_protocol_survives_owner_offline_and_sender_restart() {
        let temp = tempfile::tempdir().unwrap();
        let owner_path = temp.path().join("owner.db");
        let member_b_path = temp.path().join("member-b.db");
        let member_c_path = temp.path().join("member-c.db");
        let mut owner_storage = Storage::open(&owner_path).unwrap();
        let owner = owner_storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let mut member_b_storage = Storage::open(&member_b_path).unwrap();
        let member_b = member_b_storage
            .load_or_create_profile("Member B", Platform::Windows)
            .unwrap();
        let mut member_c_storage = Storage::open(&member_c_path).unwrap();
        let member_c = member_c_storage
            .load_or_create_profile("Member C", Platform::Android)
            .unwrap();
        connect_test_peers(&mut owner_storage, &owner, &mut member_b_storage, &member_b);
        connect_test_peers(&mut owner_storage, &owner, &mut member_c_storage, &member_c);
        connect_test_peers(
            &mut member_b_storage,
            &member_b,
            &mut member_c_storage,
            &member_c,
        );

        let created = owner_storage
            .create_group(
                &owner,
                crate::domain::ClientOperationId::generate(),
                "Family",
                &[member_b.device_id, member_c.device_id],
            )
            .unwrap();
        for (receiver, path) in [
            (&member_b, member_b_path.as_path()),
            (&member_c, member_c_path.as_path()),
        ] {
            let invite = pending_test_entry(&owner_storage, "group_invite", receiver.device_id);
            deliver_test_entry(&mut owner_storage, &owner, receiver, path, &invite);
        }

        let invite_b = member_b_storage.list_group_invitations().unwrap()[0].invite_id;
        member_b_storage
            .decide_group_invite(&member_b, ClientOperationId::generate(), invite_b, true)
            .unwrap();
        let reply_b = pending_test_entry(&member_b_storage, "group_invite_reply", owner.device_id);
        deliver_test_entry(
            &mut member_b_storage,
            &member_b,
            &owner,
            &owner_path,
            &reply_b,
        );
        let update_b = pending_test_entry(&owner_storage, "group_update", member_b.device_id);
        deliver_test_entry(
            &mut owner_storage,
            &owner,
            &member_b,
            &member_b_path,
            &update_b,
        );

        let invite_c = member_c_storage.list_group_invitations().unwrap()[0].invite_id;
        member_c_storage
            .decide_group_invite(&member_c, ClientOperationId::generate(), invite_c, true)
            .unwrap();
        let reply_c = pending_test_entry(&member_c_storage, "group_invite_reply", owner.device_id);
        deliver_test_entry(
            &mut member_c_storage,
            &member_c,
            &owner,
            &owner_path,
            &reply_c,
        );
        for (receiver, path) in [
            (&member_b, member_b_path.as_path()),
            (&member_c, member_c_path.as_path()),
        ] {
            let update = pending_test_entry(&owner_storage, "group_update", receiver.device_id);
            deliver_test_entry(&mut owner_storage, &owner, receiver, path, &update);
        }

        let sync_request =
            pending_test_entry(&member_c_storage, "group_sync_request", owner.device_id);
        deliver_test_entry(
            &mut member_c_storage,
            &member_c,
            &owner,
            &owner_path,
            &sync_request,
        );
        let sync_response =
            pending_test_entry(&owner_storage, "group_sync_response", member_c.device_id);
        deliver_test_entry(
            &mut owner_storage,
            &owner,
            &member_c,
            &member_c_path,
            &sync_response,
        );
        assert_eq!(
            member_b_storage
                .get_group(member_b.device_id, created.group_id.clone())
                .unwrap()
                .snapshot
                .revision,
            3
        );
        assert_eq!(
            member_c_storage
                .get_group(member_c.device_id, created.group_id.clone())
                .unwrap()
                .snapshot
                .revision,
            3
        );

        let outgoing = member_b_storage
            .create_outgoing_text(&member_b, &created.conversation_id, "owner is offline")
            .unwrap();
        let to_member_c = pending_test_entry(&member_b_storage, "text_message", member_c.device_id);
        deliver_test_entry(
            &mut member_b_storage,
            &member_b,
            &member_c,
            &member_c_path,
            &to_member_c,
        );
        let partial = member_b_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(partial.len(), 2);
        assert_eq!(
            partial.iter().filter(|delivery| delivery.delivered).count(),
            1
        );
        assert_eq!(
            member_c_storage
                .list_messages(&created.conversation_id, 100)
                .unwrap()
                .iter()
                .filter(|message| message.message_id == outgoing.message_id)
                .count(),
            1
        );

        drop(member_b_storage);
        let mut member_b_storage = Storage::open(&member_b_path).unwrap();
        let to_owner = pending_test_entry(&member_b_storage, "text_message", owner.device_id);
        deliver_test_entry(
            &mut member_b_storage,
            &member_b,
            &owner,
            &owner_path,
            &to_owner,
        );
        let completed = member_b_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(
            completed
                .iter()
                .filter(|delivery| delivery.delivered)
                .count(),
            2
        );
        assert!(member_b_storage.pending_outbox().unwrap().is_empty());
        assert_eq!(
            owner_storage
                .list_messages(&created.conversation_id, 100)
                .unwrap()
                .iter()
                .filter(|message| message.message_id == outgoing.message_id)
                .count(),
            1
        );
    }

    #[test]
    fn malformed_connection_is_closed_and_next_valid_outbox_reconnects() {
        let temp = tempfile::tempdir().unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
        let mut sender_storage = Storage::open(&sender_path).unwrap();
        let sender = sender_storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let mut receiver_storage = Storage::open(&receiver_path).unwrap();
        let receiver = receiver_storage
            .load_or_create_profile("Receiver", Platform::Android)
            .unwrap();
        connect_test_peers(
            &mut sender_storage,
            &sender,
            &mut receiver_storage,
            &receiver,
        );
        let conversation = sender_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                sender.device_id,
                receiver.device_id,
            )
            .unwrap();
        sender_storage
            .create_outgoing_text(
                &sender,
                &conversation.conversation_id,
                "valid after malformed",
            )
            .unwrap();
        let entry = sender_storage.pending_outbox().unwrap().remove(0);
        drop(receiver_storage);

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver_for_thread = receiver.clone();
        let receiver_path_for_thread = receiver_path.clone();
        let worker = thread::spawn(move || {
            let (mut malformed, source) = listener.accept().unwrap();
            configure_stream(&malformed).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            assert!(
                handle_incoming(
                    &mut malformed,
                    source,
                    &receiver_for_thread,
                    &receiver_path_for_thread,
                    &stop,
                    &Arc::new(ConnectionRegistry::default()),
                )
                .is_err()
            );

            let (mut valid, source) = listener.accept().unwrap();
            configure_stream(&valid).unwrap();
            handle_incoming(
                &mut valid,
                source,
                &receiver_for_thread,
                &receiver_path_for_thread,
                &stop,
                &Arc::new(ConnectionRegistry::default()),
            )
            .unwrap();
        });

        let mut malformed = TcpStream::connect(address).unwrap();
        malformed.write_all(&1_u32.to_be_bytes()).unwrap();
        malformed.write_all(b"{").unwrap();
        malformed.flush().unwrap();
        drop(malformed);

        let mut valid =
            OutgoingControlConnection::connect(&sender, receiver.device_id, address).unwrap();
        valid.deliver(&mut sender_storage, &entry).unwrap();
        drop(valid);
        worker.join().unwrap();

        assert!(sender_storage.pending_outbox().unwrap().is_empty());
        let received = Storage::open(&receiver_path)
            .unwrap()
            .list_messages(&conversation.conversation_id, 100)
            .unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text, "valid after malformed");
    }
}
