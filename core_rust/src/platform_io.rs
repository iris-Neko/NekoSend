use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io,
    sync::{Arc, Condvar, LazyLock, Mutex},
    time::Duration,
};

use serde::Deserialize;

use crate::{
    domain::{EventId, TransferEntryKind},
    storage::TransferEntryRecord,
    transfer::PreparedReceiveEntry,
};

const PLATFORM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRequestDto {
    pub request_id: String,
    pub operation: String,
    pub uri: String,
    pub final_name: Option<String>,
}

#[derive(Debug)]
struct PlatformResponse {
    fd: Option<i32>,
    value: Option<String>,
    error: Option<String>,
}

type ResponseSlot = Arc<(Mutex<Option<PlatformResponse>>, Condvar)>;

#[derive(Default)]
struct PlatformBroker {
    queue: VecDeque<PlatformRequestDto>,
    waiters: HashMap<String, ResponseSlot>,
}

static PLATFORM_BROKER: LazyLock<Mutex<PlatformBroker>> =
    LazyLock::new(|| Mutex::new(PlatformBroker::default()));

pub fn poll_platform_requests() -> Vec<PlatformRequestDto> {
    let mut broker = PLATFORM_BROKER
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    broker.queue.drain(..).collect()
}

pub(crate) fn enqueue_clipboard_write(text: &str, metadata_json: String) {
    let mut broker = PLATFORM_BROKER
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    broker.queue.push_back(PlatformRequestDto {
        request_id: EventId::generate().to_string(),
        operation: "write_clipboard_text".to_owned(),
        uri: text.to_owned(),
        final_name: Some(metadata_json),
    });
}

pub(crate) fn enqueue_clipboard_image_write(reference: &str, metadata_json: String) {
    let mut broker = PLATFORM_BROKER
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    broker.queue.push_back(PlatformRequestDto {
        request_id: EventId::generate().to_string(),
        operation: "write_clipboard_image".to_owned(),
        uri: reference.to_owned(),
        final_name: Some(metadata_json),
    });
}

pub fn complete_platform_request(
    request_id: String,
    fd: Option<i32>,
    value: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    let slot = PLATFORM_BROKER
        .lock()
        .map_err(|_| "platform request broker lock is poisoned".to_owned())?
        .waiters
        .remove(&request_id)
        .ok_or_else(|| "platform request is unknown or timed out".to_owned())?;
    let (response, ready) = &*slot;
    *response
        .lock()
        .map_err(|_| "platform response lock is poisoned".to_owned())? =
        Some(PlatformResponse { fd, value, error });
    ready.notify_one();
    Ok(())
}

pub(crate) fn open_document(uri: &str, writable: bool) -> io::Result<File> {
    let operation = if writable {
        "open_receive_file"
    } else {
        "open_source_file"
    };
    let response = request_platform(operation, uri, None)?;
    if let Some(error) = response.error {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, error));
    }
    let fd = response.fd.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "platform open request returned no file descriptor",
        )
    })?;
    file_from_raw_fd(fd)
}

pub(crate) fn prepare_document_receive(
    tree_uri: &str,
    entries: &[TransferEntryRecord],
) -> io::Result<(String, Vec<PreparedReceiveEntry>)> {
    let request_json = serde_json::to_string(
        &entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "entryId": entry.entry_id,
                    "entryKind": match entry.entry_kind {
                        TransferEntryKind::File => "file",
                        TransferEntryKind::Directory => "directory",
                    },
                    "relativePath": entry.relative_path,
                    "size": entry.size,
                    "persistedOffset": entry.persisted_offset,
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(io::Error::other)?;
    let response = request_platform("prepare_receive_tree", tree_uri, Some(request_json))?;
    if let Some(error) = response.error {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, error));
    }
    let value = response.value.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "platform receive preparation returned no value",
        )
    })?;
    let prepared: PlatformPreparedReceive =
        serde_json::from_str(&value).map_err(io::Error::other)?;
    let entries = prepared
        .prepared
        .into_iter()
        .map(|entry| {
            Ok(PreparedReceiveEntry {
                entry_id: entry.entry_id.parse().map_err(io::Error::other)?,
                destination_ref: entry.destination_ref,
                partial_ref: entry.partial_ref,
                persisted_offset: entry.persisted_offset,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((prepared.receive_base_ref, entries))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformPreparedReceive {
    receive_base_ref: String,
    prepared: Vec<PlatformPreparedEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformPreparedEntry {
    entry_id: String,
    destination_ref: String,
    partial_ref: Option<String>,
    persisted_offset: u64,
}

pub(crate) fn commit_document(uri: &str, final_name: &str) -> io::Result<String> {
    let response = request_platform("commit_receive_file", uri, Some(final_name.to_owned()))?;
    if let Some(error) = response.error {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, error));
    }
    response.value.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "platform commit request returned no document URI",
        )
    })
}

fn request_platform(
    operation: &str,
    uri: &str,
    final_name: Option<String>,
) -> io::Result<PlatformResponse> {
    let request_id = EventId::generate().to_string();
    let slot = Arc::new((Mutex::new(None), Condvar::new()));
    {
        let mut broker = PLATFORM_BROKER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        broker.waiters.insert(request_id.clone(), Arc::clone(&slot));
        broker.queue.push_back(PlatformRequestDto {
            request_id: request_id.clone(),
            operation: operation.to_owned(),
            uri: uri.to_owned(),
            final_name,
        });
    }

    let (response, ready) = &*slot;
    let response = response
        .lock()
        .map_err(|_| io::Error::other("platform response lock is poisoned"))?;
    let (mut response, timeout) = ready
        .wait_timeout_while(response, PLATFORM_REQUEST_TIMEOUT, |value| value.is_none())
        .map_err(|_| io::Error::other("platform response wait is poisoned"))?;
    if timeout.timed_out() && response.is_none() {
        PLATFORM_BROKER
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .waiters
            .remove(&request_id);
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "platform file request timed out",
        ));
    }
    response.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "platform file request completed without a response",
        )
    })
}

#[cfg(unix)]
fn file_from_raw_fd(fd: i32) -> io::Result<File> {
    use std::os::fd::FromRawFd;

    if fd < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "platform returned an invalid file descriptor",
        ));
    }
    // Ownership was detached by Android and is transferred exactly once here.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn file_from_raw_fd(_fd: i32) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "raw Android file descriptors are unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Instant};

    static TEST_BROKER_LOCK: Mutex<()> = Mutex::new(());

    fn reset_test_broker() {
        let mut broker = PLATFORM_BROKER
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        broker.queue.clear();
        broker.waiters.clear();
    }

    #[test]
    fn broker_delivers_response_to_waiting_worker() {
        let _guard = TEST_BROKER_LOCK.lock().unwrap();
        reset_test_broker();
        let worker = thread::spawn(|| request_platform("commit_receive_file", "content://x", None));
        let deadline = Instant::now() + Duration::from_secs(1);
        let request = loop {
            if let Some(request) = poll_platform_requests().pop() {
                break request;
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        };
        complete_platform_request(
            request.request_id,
            None,
            Some("content://renamed".to_owned()),
            None,
        )
        .unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap().value.as_deref(),
            Some("content://renamed")
        );
    }

    #[test]
    fn clipboard_write_is_a_nonblocking_one_way_request() {
        let _guard = TEST_BROKER_LOCK.lock().unwrap();
        reset_test_broker();
        enqueue_clipboard_write("hello", r#"{"origin":"phone","sequence":7}"#.to_owned());
        let request = poll_platform_requests().pop().unwrap();
        assert_eq!(request.operation, "write_clipboard_text");
        assert_eq!(request.uri, "hello");
        assert_eq!(
            request.final_name.as_deref(),
            Some(r#"{"origin":"phone","sequence":7}"#)
        );
    }

    #[test]
    fn clipboard_image_write_is_a_nonblocking_one_way_request() {
        let _guard = TEST_BROKER_LOCK.lock().unwrap();
        reset_test_broker();
        enqueue_clipboard_image_write(
            "content://received/image",
            r#"{"origin_device_id":"phone","clipboard_sequence":7,"content_fingerprint":"abc"}"#
                .to_owned(),
        );
        let request = poll_platform_requests().pop().unwrap();
        assert_eq!(request.operation, "write_clipboard_image");
        assert_eq!(request.uri, "content://received/image");
        assert!(request.final_name.unwrap().contains("clipboard_sequence"));
    }
}
