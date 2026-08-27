use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::frb_generated::StreamSink;

const EVENT_QUEUE_CAPACITY: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEventKind {
    CoreReady,
    AppSnapshotInvalidated,
    PeerPresenceChanged,
    ConversationChanged,
    MessageChanged,
    TransferProgress,
    IncomingOfferRequiresDecision,
    GroupInviteReceived,
    OwnDeviceBindingRequested,
    PlatformRequest,
    NotificationRequested,
    CoreErrorOccurred,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreEventDto {
    pub kind: CoreEventKind,
    pub entity_id: Option<String>,
    pub transfer_progress: Option<TransferProgressEventDto>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgressEventDto {
    pub transfer_id: String,
    pub state: String,
    pub persisted_bytes: u64,
    pub total_size: u64,
    pub bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
    pub active_entry_relative_path: Option<String>,
}

static EVENT_SENDER: OnceLock<Mutex<Option<mpsc::SyncSender<CoreEventDto>>>> = OnceLock::new();
static EVENT_QUEUE_DIRTY: AtomicBool = AtomicBool::new(false);

fn sender_slot() -> &'static Mutex<Option<mpsc::SyncSender<CoreEventDto>>> {
    EVENT_SENDER.get_or_init(|| Mutex::new(None))
}

pub fn subscribe(sink: StreamSink<CoreEventDto>) {
    disconnect();
    EVENT_QUEUE_DIRTY.store(false, Ordering::Release);
    let (sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
    if let Ok(mut slot) = sender_slot().lock() {
        *slot = Some(sender);
    }
    let _ = thread::Builder::new()
        .name("lan-chat-core-events".to_owned())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if sink.add(event).is_err() {
                    break;
                }
            }
        });
    publish(CoreEventKind::CoreReady, None);
}

pub fn disconnect() {
    if let Ok(mut slot) = sender_slot().lock() {
        *slot = None;
    }
}

pub fn publish(kind: CoreEventKind, entity_id: Option<String>) {
    publish_event(CoreEventDto {
        kind,
        entity_id,
        transfer_progress: None,
        occurred_at_ms: unix_time_ms(),
    });
}

pub fn publish_transfer_progress(progress: TransferProgressEventDto) {
    let transfer_id = progress.transfer_id.clone();
    publish_event(CoreEventDto {
        kind: CoreEventKind::TransferProgress,
        entity_id: Some(transfer_id),
        transfer_progress: Some(progress),
        occurred_at_ms: unix_time_ms(),
    });
}

fn publish_event(event: CoreEventDto) {
    let Ok(mut slot) = sender_slot().lock() else {
        return;
    };
    let Some(sender) = slot.as_ref() else {
        return;
    };
    if event.kind != CoreEventKind::AppSnapshotInvalidated
        && EVENT_QUEUE_DIRTY.swap(false, Ordering::AcqRel)
    {
        match sender.try_send(CoreEventDto {
            kind: CoreEventKind::AppSnapshotInvalidated,
            entity_id: None,
            transfer_progress: None,
            occurred_at_ms: unix_time_ms(),
        }) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                EVENT_QUEUE_DIRTY.store(true, Ordering::Release);
                return;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                *slot = None;
                return;
            }
        }
    }
    match sender.try_send(event) {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(_)) => {
            EVENT_QUEUE_DIRTY.store(true, Ordering::Release);
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            *slot = None;
        }
    }
}

pub fn invalidate() {
    publish(CoreEventKind::AppSnapshotInvalidated, None);
}

fn unix_time_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}
