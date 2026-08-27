use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    domain::{
        ClientOperationId, DeviceId, EntryId, EventId, GroupId, MessageId, MessageKind, Platform,
        PrivateConversationId, TransferEntryKind, TransferId,
    },
    transfer::{
        ManifestEntry, ManifestError, PreparedReceiveEntry, SourceItem, TransferManifest,
        validate_manifest,
    },
};

mod bindings;
mod groups;

pub use bindings::*;
pub use groups::*;

const SCHEMA_VERSION: i64 = 3;
const MIGRATION_V1: &str = include_str!("migration_v1.sql");
const MIGRATION_V2: &str = include_str!("migration_v2.sql");
const MIGRATION_V3: &str = include_str!("migration_v3.sql");
const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalProfile {
    pub device_id: DeviceId,
    pub device_name: String,
    pub platform: Platform,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettingsRecord {
    pub default_receive_policy: String,
    pub default_receive_ref: Option<String>,
    pub notifications_enabled: bool,
    pub close_to_tray: bool,
    pub start_on_boot: bool,
    pub android_keep_online: bool,
    pub auto_open_receive_directory: bool,
    pub log_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerReceivePolicyRecord {
    pub device_id: DeviceId,
    pub device_name: String,
    pub relation: String,
    pub receive_policy_override: Option<String>,
    pub effective_receive_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRecord {
    pub conversation_id: String,
    pub title: String,
    pub peer_device_id: Option<DeviceId>,
    pub group_id: Option<GroupId>,
    pub is_group: bool,
    pub member_count: u32,
    pub last_message_preview: Option<String>,
    pub last_activity_at_ms: i64,
    pub unread_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub message_id: MessageId,
    pub conversation_id: String,
    pub sender_device_id: DeviceId,
    pub kind: String,
    pub state: String,
    pub text: String,
    pub total_size: Option<u64>,
    pub entry_count: Option<u32>,
    pub transfer_id: Option<TransferId>,
    pub transfer_state: Option<String>,
    pub persisted_bytes: Option<u64>,
    pub local_file_ref: Option<String>,
    pub created_at_ms: i64,
    pub local_sort_order: i64,
    pub delivered_count: u32,
    pub delivery_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDeliveryRecord {
    pub recipient_device_id: DeviceId,
    pub recipient_name: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub updated_at_ms: i64,
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOutbox {
    pub peer_device_id: DeviceId,
    pub event_id: EventId,
    pub event_type: String,
    pub payload_json: String,
    pub last_ip: String,
    pub requires_receipt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRecord {
    pub transfer_id: TransferId,
    pub message_id: MessageId,
    pub conversation_id: String,
    pub peer_device_id: DeviceId,
    pub direction: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub display_name: String,
    pub total_size: u64,
    pub entry_count: u32,
    pub persisted_bytes: u64,
    pub receive_base_ref: Option<String>,
    pub local_file_ref: Option<String>,
    pub paused_by_user: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEntryRecord {
    pub entry_id: EntryId,
    pub transfer_id: TransferId,
    pub entry_kind: TransferEntryKind,
    pub relative_path: String,
    pub size: u64,
    pub modified_at_ms: i64,
    pub source_ref: Option<String>,
    pub destination_ref: Option<String>,
    pub partial_ref: Option<String>,
    pub persisted_offset: u64,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingOfferRecord {
    pub message_id: MessageId,
    pub transfer_id: TransferId,
    pub event_id: EventId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImageMetadata {
    pub origin_device_id: DeviceId,
    pub clipboard_sequence: u64,
    pub content_fingerprint: String,
    pub automatic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedClipboardImage {
    pub reference: String,
    pub metadata: ClipboardImageMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOfferResult {
    pub transfer_id: TransferId,
    pub receipt_json: String,
    pub duplicate: bool,
    pub auto_receive_base_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveTransferControlResult {
    pub receipt_json: String,
    pub queued_resume_reply: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteConversationResult {
    pub cancelled_transfer_ids: Vec<TransferId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedOffset {
    pub entry_id: EntryId,
    pub offset: u64,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("application data directory is not writable: {0}")]
    DirectoryUnwritable(String),
    #[error("failed to open database: {0}")]
    Open(#[source] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[source] rusqlite::Error),
    #[error("database write failed: {0}")]
    Write(#[source] rusqlite::Error),
    #[error("database contains an invalid device id")]
    InvalidStoredDeviceId,
    #[error("device name must contain 1-32 characters and at most 128 UTF-8 bytes")]
    InvalidDeviceName,
    #[error("text message must contain 1-20,000 Unicode characters")]
    InvalidText,
    #[error("invalid transfer manifest: {0}")]
    Manifest(#[from] ManifestError),
    #[error("peer or private conversation does not exist")]
    PeerNotFound,
    #[error("conversation does not exist")]
    ConversationNotFound,
    #[error("group name must contain 1-50 characters and at most 200 UTF-8 bytes")]
    InvalidGroupName,
    #[error("group members must be unique and contain 2-32 devices with exactly one owner")]
    InvalidGroupMembers,
    #[error("group does not exist or is not active for this device")]
    GroupNotFound,
    #[error("only the current group owner can perform this operation")]
    NotGroupOwner,
    #[error("group revision does not follow the locally stored revision")]
    GroupRevisionConflict,
    #[error("group update must contain exactly one management action")]
    InvalidGroupUpdate,
    #[error("group already contains 32 active or invited devices")]
    GroupFull,
    #[error("the group owner must transfer ownership or disband before leaving")]
    GroupOwnerMustTransfer,
    #[error("group invitation does not exist or is no longer pending")]
    InviteNotFound,
    #[error("own-device binding does not exist or is in the wrong state")]
    BindingNotFound,
    #[error("clipboard mode must be off, send_only, receive_only, or bidirectional")]
    InvalidClipboardMode,
    #[error("automatic clipboard sync requires an active own-device binding")]
    ClipboardBindingRequired,
    #[error("clipboard text must contain 1 byte to 1 MiB of UTF-8")]
    ClipboardTooLarge,
    #[error("clipboard image metadata must contain a lowercase SHA-256 fingerprint")]
    InvalidClipboardImageMetadata,
    #[error("receive policy must be auto_accept or ask_every_time")]
    InvalidReceivePolicy,
    #[error("log level must be normal or debug")]
    InvalidLogLevel,
    #[error("local clipboard sequence is exhausted")]
    ClipboardSequenceExhausted,
    #[error("replacement sources do not match the original transfer metadata")]
    TransferSourceMismatch,
    #[error("local group sequence is exhausted")]
    GroupSequenceExhausted,
    #[error("failed to serialize persisted application data: {0}")]
    Serialization(String),
    #[error("transfer does not exist or is in the wrong state")]
    TransferNotFound,
    #[error("database contains an invalid identifier")]
    InvalidStoredId,
    #[error("database schema version {0} is newer than this application supports")]
    UnsupportedSchema(i64),
}

pub struct Storage {
    connection: Connection,
    path: PathBuf,
}

impl Storage {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| StorageError::DirectoryUnwritable(error.to_string()))?;
        }

        let connection = Connection::open(path).map_err(StorageError::Open)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(StorageError::Open)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;\n\
                 PRAGMA journal_mode = WAL;\n\
                 PRAGMA synchronous = NORMAL;\n\
                 PRAGMA temp_store = MEMORY;",
            )
            .map_err(StorageError::Open)?;

        let mut storage = Self {
            connection,
            path: path.to_path_buf(),
        };
        storage.migrate()?;
        Ok(storage)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create_profile(
        &mut self,
        device_name: &str,
        platform: Platform,
    ) -> Result<LocalProfile, StorageError> {
        validate_device_name(device_name)?;
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let stored = transaction
            .query_row(
                "SELECT device_id, device_name, created_at_ms
                 FROM local_profile WHERE singleton_id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?;

        let (device_id, stored_device_name, created_at_ms) = match stored {
            Some((raw_id, stored_device_name, created_at_ms)) => {
                let device_id =
                    DeviceId::from_str(&raw_id).map_err(|_| StorageError::InvalidStoredDeviceId)?;
                transaction
                    .execute(
                        "UPDATE local_profile
                         SET platform = ?1, updated_at_ms = ?2
                         WHERE singleton_id = 1",
                        params![platform_name(platform), now],
                    )
                    .map_err(StorageError::Write)?;
                (device_id, stored_device_name, created_at_ms)
            }
            None => {
                let device_id = DeviceId::generate();
                transaction
                    .execute(
                        "INSERT INTO local_profile(
                            singleton_id, device_id, device_name, platform,
                            group_sequence, clipboard_sequence, created_at_ms, updated_at_ms
                         ) VALUES (1, ?1, ?2, ?3, 0, 0, ?4, ?4)",
                        params![
                            device_id.to_string(),
                            device_name,
                            platform_name(platform),
                            now
                        ],
                    )
                    .map_err(StorageError::Write)?;
                transaction
                    .execute(
                        "INSERT INTO app_settings(singleton_id, updated_at_ms) VALUES (1, ?1)",
                        [now],
                    )
                    .map_err(StorageError::Write)?;
                (device_id, device_name.to_owned(), now)
            }
        };

        transaction.commit().map_err(StorageError::Write)?;
        Ok(LocalProfile {
            device_id,
            device_name: stored_device_name,
            platform,
            created_at_ms,
            updated_at_ms: now,
        })
    }

    pub fn upsert_nearby_peer(
        &self,
        device_id: DeviceId,
        device_name: &str,
        platform: Platform,
        source_ip: &str,
        seen_at_ms: i64,
    ) -> Result<(), StorageError> {
        validate_device_name(device_name)?;
        self.connection
            .execute(
                "INSERT INTO peers(
                    device_id, device_name, platform, relation, last_ip,
                    last_seen_at_ms, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'nearby', ?4, ?5, ?5, ?5)
                 ON CONFLICT(device_id) DO UPDATE SET
                    device_name = excluded.device_name,
                    platform = excluded.platform,
                    last_ip = excluded.last_ip,
                    last_seen_at_ms = excluded.last_seen_at_ms,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    device_id.to_string(),
                    device_name,
                    platform_name(platform),
                    source_ip,
                    seen_at_ms
                ],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn open_private_conversation(
        &mut self,
        client_operation_id: ClientOperationId,
        local_device_id: DeviceId,
        peer_device_id: DeviceId,
    ) -> Result<ConversationRecord, StorageError> {
        let conversation_id = PrivateConversationId::new(local_device_id, peer_device_id)
            .map_err(|_| StorageError::PeerNotFound)?
            .to_string();
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved_conversation_id) = saved_client_operation::<String>(
            &transaction,
            client_operation_id,
            "open_private_conversation",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return self
                .get_conversation(&saved_conversation_id)?
                .ok_or(StorageError::PeerNotFound);
        }
        let peer_name = transaction
            .query_row(
                "SELECT device_name FROM peers WHERE device_id = ?1",
                [peer_device_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::PeerNotFound)?;
        transaction
            .execute(
                "UPDATE peers
                 SET relation = CASE WHEN relation = 'nearby' THEN 'known' ELSE relation END,
                     known_at_ms = COALESCE(known_at_ms, ?2), updated_at_ms = ?2
                 WHERE device_id = ?1",
                params![peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO conversations(
                    conversation_id, kind, state, peer_device_id, title_cache,
                    last_activity_at_ms, unread_count, created_at_ms
                 ) VALUES (?1, 'private', 'active', ?2, ?3, ?4, 0, ?4)
                 ON CONFLICT(conversation_id) DO UPDATE SET
                    title_cache = excluded.title_cache, deleted_at_ms = NULL",
                params![conversation_id, peer_device_id.to_string(), peer_name, now],
            )
            .map_err(StorageError::Write)?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "open_private_conversation",
            &conversation_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        self.get_conversation(&conversation_id)?
            .ok_or(StorageError::PeerNotFound)
    }

    pub fn set_default_receive_ref(
        &mut self,
        client_operation_id: ClientOperationId,
        receive_ref: Option<&str>,
    ) -> Result<(), StorageError> {
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "set_default_receive_ref",
            |transaction, now| {
                transaction
                    .execute(
                        "UPDATE app_settings SET default_receive_ref = ?1, updated_at_ms = ?2
                 WHERE singleton_id = 1",
                        params![receive_ref, now],
                    )
                    .map_err(StorageError::Write)?;
                Ok(())
            },
        )
    }

    pub fn app_settings(&self) -> Result<AppSettingsRecord, StorageError> {
        self.connection
            .query_row(
                "SELECT default_receive_policy, default_receive_ref,
                        notifications_enabled, close_to_tray, start_on_boot,
                        android_keep_online, auto_open_receive_directory, log_level
                 FROM app_settings WHERE singleton_id = 1",
                [],
                |row| {
                    Ok(AppSettingsRecord {
                        default_receive_policy: row.get(0)?,
                        default_receive_ref: row.get(1)?,
                        notifications_enabled: row.get(2)?,
                        close_to_tray: row.get(3)?,
                        start_on_boot: row.get(4)?,
                        android_keep_online: row.get(5)?,
                        auto_open_receive_directory: row.get(6)?,
                        log_level: row.get(7)?,
                    })
                },
            )
            .map_err(StorageError::Write)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_app_settings(
        &mut self,
        client_operation_id: ClientOperationId,
        default_receive_policy: Option<&str>,
        default_receive_ref: Option<&str>,
        clear_default_receive_ref: bool,
        notifications_enabled: Option<bool>,
        close_to_tray: Option<bool>,
        start_on_boot: Option<bool>,
        android_keep_online: Option<bool>,
        auto_open_receive_directory: Option<bool>,
        log_level: Option<&str>,
    ) -> Result<AppSettingsRecord, StorageError> {
        if default_receive_policy.is_some_and(|value| !valid_receive_policy(value)) {
            return Err(StorageError::InvalidReceivePolicy);
        }
        if log_level.is_some_and(|value| !matches!(value, "normal" | "debug")) {
            return Err(StorageError::InvalidLogLevel);
        }
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "update_app_settings",
            |transaction, now| {
                transaction
                    .execute(
                        "UPDATE app_settings SET
                    default_receive_policy = COALESCE(?1, default_receive_policy),
                    default_receive_ref = CASE
                        WHEN ?3 THEN NULL
                        WHEN ?2 IS NOT NULL THEN ?2
                        ELSE default_receive_ref
                    END,
                    notifications_enabled = COALESCE(?4, notifications_enabled),
                    close_to_tray = COALESCE(?5, close_to_tray),
                    start_on_boot = COALESCE(?6, start_on_boot),
                    android_keep_online = COALESCE(?7, android_keep_online),
                    auto_open_receive_directory = COALESCE(?8, auto_open_receive_directory),
                    log_level = COALESCE(?9, log_level),
                    updated_at_ms = ?10
                 WHERE singleton_id = 1",
                        params![
                            default_receive_policy,
                            default_receive_ref,
                            clear_default_receive_ref,
                            notifications_enabled,
                            close_to_tray,
                            start_on_boot,
                            android_keep_online,
                            auto_open_receive_directory,
                            log_level,
                            now,
                        ],
                    )
                    .map_err(StorageError::Write)?;
                transaction
                    .query_row(
                        "SELECT default_receive_policy, default_receive_ref,
                                notifications_enabled, close_to_tray, start_on_boot,
                                android_keep_online, auto_open_receive_directory, log_level
                         FROM app_settings WHERE singleton_id = 1",
                        [],
                        |row| {
                            Ok(AppSettingsRecord {
                                default_receive_policy: row.get(0)?,
                                default_receive_ref: row.get(1)?,
                                notifications_enabled: row.get(2)?,
                                close_to_tray: row.get(3)?,
                                start_on_boot: row.get(4)?,
                                android_keep_online: row.get(5)?,
                                auto_open_receive_directory: row.get(6)?,
                                log_level: row.get(7)?,
                            })
                        },
                    )
                    .map_err(StorageError::Write)
            },
        )
    }

    pub fn update_device_name(
        &mut self,
        client_operation_id: ClientOperationId,
        device_name: &str,
    ) -> Result<LocalProfile, StorageError> {
        validate_device_name(device_name)?;
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "update_device_name",
            |transaction, now| {
                transaction
                    .execute(
                        "UPDATE local_profile SET device_name = ?1, updated_at_ms = ?2
                 WHERE singleton_id = 1",
                        params![device_name, now],
                    )
                    .map_err(StorageError::Write)?;
                transaction
                    .query_row(
                        "SELECT device_id, device_name, platform, created_at_ms, updated_at_ms
                 FROM local_profile WHERE singleton_id = 1",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, i64>(4)?,
                            ))
                        },
                    )
                    .map_err(StorageError::Write)
                    .and_then(
                        |(device_id, name, platform, created_at_ms, updated_at_ms)| {
                            Ok(LocalProfile {
                                device_id: device_id
                                    .parse()
                                    .map_err(|_| StorageError::InvalidStoredDeviceId)?,
                                device_name: name,
                                platform: match platform.as_str() {
                                    "windows" => Platform::Windows,
                                    "android" => Platform::Android,
                                    _ => return Err(StorageError::InvalidStoredId),
                                },
                                created_at_ms,
                                updated_at_ms,
                            })
                        },
                    )
            },
        )
    }

    pub fn list_peer_receive_policies(&self) -> Result<Vec<PeerReceivePolicyRecord>, StorageError> {
        let default_policy = self.app_settings()?.default_receive_policy;
        let mut statement = self
            .connection
            .prepare(
                "SELECT device_id, device_name, relation, receive_policy_override
                 FROM peers WHERE relation != 'nearby'
                 ORDER BY device_name COLLATE NOCASE, device_id",
            )
            .map_err(StorageError::Write)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(StorageError::Write)?;
        rows.map(|row| {
            let (device_id, device_name, relation, receive_policy_override) =
                row.map_err(StorageError::Write)?;
            Ok(PeerReceivePolicyRecord {
                device_id: device_id
                    .parse()
                    .map_err(|_| StorageError::InvalidStoredDeviceId)?,
                device_name,
                relation,
                effective_receive_policy: receive_policy_override
                    .clone()
                    .unwrap_or_else(|| default_policy.clone()),
                receive_policy_override,
            })
        })
        .collect()
    }

    pub fn set_peer_receive_policy(
        &mut self,
        client_operation_id: ClientOperationId,
        peer_device_id: DeviceId,
        receive_policy_override: Option<&str>,
    ) -> Result<(), StorageError> {
        if receive_policy_override.is_some_and(|value| !valid_receive_policy(value)) {
            return Err(StorageError::InvalidReceivePolicy);
        }
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "set_peer_receive_policy",
            |transaction, now| {
                let changed = transaction
                    .execute(
                        "UPDATE peers SET receive_policy_override = ?2, updated_at_ms = ?3
                 WHERE device_id = ?1 AND relation != 'nearby'",
                        params![peer_device_id.to_string(), receive_policy_override, now],
                    )
                    .map_err(StorageError::Write)?;
                if changed == 0 {
                    return Err(StorageError::PeerNotFound);
                }
                Ok(())
            },
        )
    }

    pub fn create_outgoing_text(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        text: &str,
    ) -> Result<MessageRecord, StorageError> {
        self.create_outgoing_text_message(profile, conversation_id, text, "text", None)
    }

    pub fn create_outgoing_text_idempotent(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        conversation_id: &str,
        text: &str,
    ) -> Result<MessageRecord, StorageError> {
        self.create_outgoing_text_message(
            profile,
            conversation_id,
            text,
            "text",
            Some(client_operation_id),
        )
    }

    pub fn create_outgoing_clipboard_text(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        text: &str,
    ) -> Result<MessageRecord, StorageError> {
        self.create_outgoing_text_message(profile, conversation_id, text, "clipboard_text", None)
    }

    pub fn create_outgoing_clipboard_text_idempotent(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        conversation_id: &str,
        text: &str,
    ) -> Result<MessageRecord, StorageError> {
        self.create_outgoing_text_message(
            profile,
            conversation_id,
            text,
            "clipboard_text",
            Some(client_operation_id),
        )
    }

    fn create_outgoing_text_message(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        text: &str,
        message_kind: &str,
        client_operation_id: Option<ClientOperationId>,
    ) -> Result<MessageRecord, StorageError> {
        validate_text_message(message_kind, text)?;
        if conversation_id.starts_with("g:") {
            return self.create_outgoing_group_text(
                profile,
                conversation_id,
                text,
                message_kind,
                client_operation_id,
            );
        }
        let canonical: PrivateConversationId = conversation_id
            .parse()
            .map_err(|_| StorageError::PeerNotFound)?;
        let (left, right) = canonical.devices();
        let peer_device_id = if left == profile.device_id {
            right
        } else if right == profile.device_id {
            left
        } else {
            return Err(StorageError::PeerNotFound);
        };
        let message_id = MessageId::generate();
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "text_message",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "message_id": message_id,
                "conversation_id": conversation_id,
                "message_kind": message_kind,
                "text": text,
                "created_at_ms": now
            }
        })
        .to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let operation_type = if message_kind == "clipboard_text" {
            "send_clipboard_text_message"
        } else {
            "send_text_message"
        };
        if let Some(operation_id) = client_operation_id
            && let Some(saved) = saved_client_operation(&transaction, operation_id, operation_type)?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let exists = transaction
            .query_row(
                "SELECT 1 FROM conversations
                 WHERE conversation_id = ?1 AND peer_device_id = ?2 AND kind = 'private'",
                params![conversation_id, peer_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !exists {
            return Err(StorageError::PeerNotFound);
        }
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(
                    message_id, conversation_id, sender_device_id, kind, state,
                    text_content, created_at_ms, local_sort_order
                 ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    profile.device_id.to_string(),
                    message_kind,
                    text,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO message_deliveries(
                    message_id, recipient_device_id, state, updated_at_ms
                 ) VALUES (?1, ?2, 'queued', ?3)",
                params![message_id.to_string(), peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO outbox(
                    peer_device_id, event_id, event_type, payload_json,
                    requires_receipt, state, attempts, next_attempt_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'text_message', ?3, 1, 'pending', 0, ?4, ?4, ?4)",
                params![
                    peer_device_id.to_string(),
                    event_id.to_string(),
                    payload_json,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations
                 SET last_message_id = ?2, last_activity_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let result = MessageRecord {
            message_id,
            conversation_id: conversation_id.to_owned(),
            sender_device_id: profile.device_id,
            kind: message_kind.to_owned(),
            state: "queued".to_owned(),
            text: text.to_owned(),
            total_size: None,
            entry_count: None,
            transfer_id: None,
            transfer_state: None,
            persisted_bytes: None,
            local_file_ref: None,
            created_at_ms: now,
            local_sort_order: sort_order,
            delivered_count: 0,
            delivery_count: 1,
        };
        if let Some(operation_id) = client_operation_id {
            save_client_operation(&transaction, operation_id, operation_type, &result, now)?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    pub fn create_outgoing_offer(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        manifest: &TransferManifest,
        clipboard_fingerprint: Option<&str>,
        automatic: bool,
    ) -> Result<OutgoingOfferRecord, StorageError> {
        self.create_outgoing_offer_inner(
            profile,
            conversation_id,
            manifest,
            clipboard_fingerprint,
            automatic,
            None,
        )
    }

    pub fn create_outgoing_offer_idempotent(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        conversation_id: &str,
        manifest: &TransferManifest,
        clipboard_fingerprint: Option<&str>,
        automatic: bool,
    ) -> Result<OutgoingOfferRecord, StorageError> {
        self.create_outgoing_offer_inner(
            profile,
            conversation_id,
            manifest,
            clipboard_fingerprint,
            automatic,
            Some(client_operation_id),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_outgoing_offer_inner(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        manifest: &TransferManifest,
        clipboard_fingerprint: Option<&str>,
        automatic: bool,
        client_operation_id: Option<ClientOperationId>,
    ) -> Result<OutgoingOfferRecord, StorageError> {
        validate_manifest(manifest)?;
        validate_clipboard_image_offer_metadata(
            manifest.message_kind,
            clipboard_fingerprint,
            automatic,
        )?;
        if conversation_id.starts_with("g:") {
            return self.create_outgoing_group_offer(
                profile,
                conversation_id,
                manifest,
                clipboard_fingerprint,
                automatic,
                client_operation_id,
            );
        }
        let canonical: PrivateConversationId = conversation_id
            .parse()
            .map_err(|_| StorageError::PeerNotFound)?;
        let (left, right) = canonical.devices();
        let peer_device_id = if left == profile.device_id {
            right
        } else if right == profile.device_id {
            left
        } else {
            return Err(StorageError::PeerNotFound);
        };
        let message_id = MessageId::generate();
        let transfer_id = TransferId::generate();
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let total_size = to_sql_u64(manifest.total_size)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(operation_id) = client_operation_id
            && let Some(saved) = saved_client_operation(&transaction, operation_id, "send_source")?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let exists = transaction
            .query_row(
                "SELECT 1 FROM conversations
                 WHERE conversation_id = ?1 AND peer_device_id = ?2 AND kind = 'private'",
                params![conversation_id, peer_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !exists {
            return Err(StorageError::PeerNotFound);
        }
        if automatic {
            require_clipboard_send_binding(&transaction, peer_device_id)?;
        }
        let clipboard_metadata = clipboard_fingerprint
            .map(|fingerprint| {
                allocate_clipboard_image_metadata(
                    &transaction,
                    profile.device_id,
                    fingerprint,
                    automatic,
                    now,
                )
            })
            .transpose()?;
        let entries = manifest
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "entry_id": entry.entry_id,
                    "entry_kind": entry.entry_kind,
                    "relative_path": entry.relative_path,
                    "size": entry.size,
                    "modified_at_ms": entry.modified_at_ms,
                })
            })
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({
            "message_id": message_id,
            "transfer_id": transfer_id,
            "conversation_id": conversation_id,
            "message_kind": manifest.message_kind,
            "display_name": manifest.display_name,
            "total_size": manifest.total_size,
            "entry_count": manifest.entry_count,
            "created_at_ms": now,
            "entries": entries,
        });
        if let Some(metadata) = &clipboard_metadata {
            body["origin_device_id"] = serde_json::json!(metadata.origin_device_id);
            body["clipboard_sequence"] = serde_json::json!(metadata.clipboard_sequence);
            body["content_fingerprint"] = serde_json::json!(metadata.content_fingerprint);
            body["automatic"] = serde_json::json!(metadata.automatic);
        }
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "file_offer",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": body,
        })
        .to_string();
        if payload_json.len() > crate::transfer::MAX_MANIFEST_JSON_BYTES {
            return Err(StorageError::Manifest(ManifestError::JsonLimit));
        }
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(
                    message_id, conversation_id, sender_device_id, kind, state,
                    display_name, total_size, entry_count, created_at_ms, local_sort_order
                 ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    profile.device_id.to_string(),
                    message_kind_name(manifest.message_kind),
                    manifest.display_name,
                    total_size,
                    i64::from(manifest.entry_count),
                    now,
                    sort_order,
                ],
            )
            .map_err(StorageError::Write)?;
        if let Some(metadata) = &clipboard_metadata {
            insert_clipboard_image_dedup(&transaction, message_id, metadata, now)?;
        }
        transaction
            .execute(
                "INSERT INTO message_deliveries(
                    message_id, recipient_device_id, state, last_event_id, updated_at_ms
                 ) VALUES (?1, ?2, 'queued', ?3, ?4)",
                params![
                    message_id.to_string(),
                    peer_device_id.to_string(),
                    event_id.to_string(),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO transfers(
                    transfer_id, message_id, peer_device_id, direction, state,
                    display_name, total_size, entry_count, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'send', 'queued', ?4, ?5, ?6, ?7, ?7)",
                params![
                    transfer_id.to_string(),
                    message_id.to_string(),
                    peer_device_id.to_string(),
                    manifest.display_name,
                    total_size,
                    i64::from(manifest.entry_count),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        insert_transfer_entries(&transaction, transfer_id, &manifest.entries, true, now)?;
        transaction
            .execute(
                "INSERT INTO outbox(
                    peer_device_id, event_id, event_type, payload_json,
                    requires_receipt, state, attempts, next_attempt_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'file_offer', ?3, 1, 'pending', 0, ?4, ?4, ?4)",
                params![
                    peer_device_id.to_string(),
                    event_id.to_string(),
                    payload_json,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let result = OutgoingOfferRecord {
            message_id,
            transfer_id,
            event_id,
        };
        if let Some(operation_id) = client_operation_id {
            save_client_operation(&transaction, operation_id, "send_source", &result, now)?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    fn create_outgoing_group_offer(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        manifest: &TransferManifest,
        clipboard_fingerprint: Option<&str>,
        automatic: bool,
        client_operation_id: Option<ClientOperationId>,
    ) -> Result<OutgoingOfferRecord, StorageError> {
        if automatic {
            return Err(StorageError::ClipboardBindingRequired);
        }
        let group_id: GroupId = conversation_id
            .parse()
            .map_err(|_| StorageError::GroupNotFound)?;
        let now = unix_time_ms();
        let total_size = to_sql_u64(manifest.total_size)?;
        let message_id = MessageId::generate();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(operation_id) = client_operation_id
            && let Some(saved) = saved_client_operation(&transaction, operation_id, "send_source")?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let (revision, state, local_membership) = transaction
            .query_row(
                "SELECT g.revision, g.state, gm.membership
                 FROM groups g JOIN group_members gm ON gm.group_id = g.group_id
                 WHERE g.group_id = ?1 AND gm.device_id = ?2",
                params![group_id.to_string(), profile.device_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        if state != "active" || local_membership != "joined" {
            return Err(StorageError::GroupNotFound);
        }
        let mut statement = transaction
            .prepare(
                "SELECT device_id FROM group_members
                 WHERE group_id = ?1 AND membership = 'joined' AND device_id != ?2
                 ORDER BY device_id",
            )
            .map_err(StorageError::Write)?;
        let recipients = statement
            .query_map(
                params![group_id.to_string(), profile.device_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .map_err(StorageError::Write)?
            .map(|row| {
                row.map_err(StorageError::Write)?
                    .parse()
                    .map_err(|_| StorageError::InvalidStoredId)
            })
            .collect::<Result<Vec<DeviceId>, StorageError>>()?;
        drop(statement);
        if recipients.is_empty() {
            return Err(StorageError::GroupNotFound);
        }
        let clipboard_metadata = clipboard_fingerprint
            .map(|fingerprint| {
                allocate_clipboard_image_metadata(
                    &transaction,
                    profile.device_id,
                    fingerprint,
                    false,
                    now,
                )
            })
            .transpose()?;
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(
                    message_id, conversation_id, sender_device_id, kind, state,
                    display_name, total_size, entry_count, group_revision,
                    created_at_ms, local_sort_order
                 ) VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    profile.device_id.to_string(),
                    message_kind_name(manifest.message_kind),
                    manifest.display_name,
                    total_size,
                    i64::from(manifest.entry_count),
                    revision,
                    now,
                    sort_order,
                ],
            )
            .map_err(StorageError::Write)?;
        if let Some(metadata) = &clipboard_metadata {
            insert_clipboard_image_dedup(&transaction, message_id, metadata, now)?;
        }
        let mut first_offer = None;
        for recipient in recipients {
            let transfer_id = TransferId::generate();
            let event_id = EventId::generate();
            let recipient_entries = manifest
                .entries
                .iter()
                .map(|entry| ManifestEntry {
                    entry_id: EntryId::generate(),
                    entry_kind: entry.entry_kind,
                    relative_path: entry.relative_path.clone(),
                    size: entry.size,
                    modified_at_ms: entry.modified_at_ms,
                    source_ref: entry.source_ref.clone(),
                })
                .collect::<Vec<_>>();
            let mut body = serde_json::json!({
                "message_id": message_id,
                "transfer_id": transfer_id,
                "conversation_id": group_id,
                "message_kind": manifest.message_kind,
                "display_name": manifest.display_name,
                "total_size": manifest.total_size,
                "entry_count": manifest.entry_count,
                "created_at_ms": now,
                "group_revision": revision,
                "entries": recipient_entries,
            });
            if let Some(metadata) = &clipboard_metadata {
                body["origin_device_id"] = serde_json::json!(metadata.origin_device_id);
                body["clipboard_sequence"] = serde_json::json!(metadata.clipboard_sequence);
                body["content_fingerprint"] = serde_json::json!(metadata.content_fingerprint);
                body["automatic"] = serde_json::json!(metadata.automatic);
            }
            let payload_json = serde_json::json!({
                "version": 1,
                "type": "file_offer",
                "event_id": event_id,
                "sender_device_id": profile.device_id,
                "sent_at_ms": now,
                "body": body,
            })
            .to_string();
            if payload_json.len() > crate::transfer::MAX_MANIFEST_JSON_BYTES {
                return Err(StorageError::Manifest(ManifestError::JsonLimit));
            }
            transaction
                .execute(
                    "INSERT INTO message_deliveries(
                        message_id, recipient_device_id, state, last_event_id, updated_at_ms
                     ) VALUES (?1, ?2, 'queued', ?3, ?4)",
                    params![
                        message_id.to_string(),
                        recipient.to_string(),
                        event_id.to_string(),
                        now,
                    ],
                )
                .map_err(StorageError::Write)?;
            transaction
                .execute(
                    "INSERT INTO transfers(
                        transfer_id, message_id, peer_device_id, direction, state,
                        display_name, total_size, entry_count, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, 'send', 'queued', ?4, ?5, ?6, ?7, ?7)",
                    params![
                        transfer_id.to_string(),
                        message_id.to_string(),
                        recipient.to_string(),
                        manifest.display_name,
                        total_size,
                        i64::from(manifest.entry_count),
                        now,
                    ],
                )
                .map_err(StorageError::Write)?;
            insert_transfer_entries(&transaction, transfer_id, &recipient_entries, true, now)?;
            transaction
                .execute(
                    "INSERT INTO outbox(
                        peer_device_id, event_id, event_type, payload_json,
                        requires_receipt, state, attempts, next_attempt_at_ms,
                        created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, 'file_offer', ?3, 1, 'pending', 0, ?4, ?4, ?4)",
                    params![
                        recipient.to_string(),
                        event_id.to_string(),
                        payload_json,
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
            first_offer.get_or_insert(OutgoingOfferRecord {
                message_id,
                transfer_id,
                event_id,
            });
        }
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let result = first_offer.ok_or(StorageError::GroupNotFound)?;
        if let Some(operation_id) = client_operation_id {
            save_client_operation(&transaction, operation_id, "send_source", &result, now)?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    pub fn pending_outbox(&self) -> Result<Vec<PendingOutbox>, StorageError> {
        let now = unix_time_ms();
        let mut statement = self
            .connection
            .prepare(
                "SELECT o.peer_device_id, o.event_id, o.event_type, o.payload_json, p.last_ip,
                        o.requires_receipt
                 FROM outbox o
                 JOIN peers p ON p.device_id = o.peer_device_id
                 WHERE o.state = 'pending' AND o.next_attempt_at_ms <= ?1
                   AND p.last_ip IS NOT NULL
                 ORDER BY o.outbox_id LIMIT 16",
            )
            .map_err(StorageError::Write)?;
        let rows = statement
            .query_map([now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(StorageError::Write)?;
        rows.map(|row| {
            let (peer, event, event_type, payload_json, last_ip, requires_receipt) =
                row.map_err(StorageError::Write)?;
            Ok(PendingOutbox {
                peer_device_id: peer.parse().map_err(|_| StorageError::InvalidStoredId)?,
                event_id: event.parse().map_err(|_| StorageError::InvalidStoredId)?,
                event_type,
                payload_json,
                last_ip,
                requires_receipt: requires_receipt != 0,
            })
        })
        .collect()
    }

    pub fn mark_outbox_retry(
        &self,
        peer_device_id: DeviceId,
        event_id: EventId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        self.connection
            .execute(
                "UPDATE outbox SET attempts = attempts + 1,
                    next_attempt_at_ms = ?3 + MIN(30000, 1000 * (attempts + 1)),
                    updated_at_ms = ?3
                 WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), event_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn mark_outbox_protocol_failure(
        &mut self,
        peer_device_id: DeviceId,
        event_id: EventId,
        failure_reason: &str,
    ) -> Result<(), StorageError> {
        let payload_json = self
            .connection
            .query_row(
                "SELECT payload_json FROM outbox WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), event_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?;
        let Some(payload_json) = payload_json else {
            return Ok(());
        };
        let payload: serde_json::Value = serde_json::from_str(&payload_json)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let message_id = payload
            .pointer("/body/message_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| MessageId::from_str(value).ok());
        let transfer_id = payload
            .pointer("/body/transfer_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| TransferId::from_str(value).ok());
        let failure_reason = failure_reason.chars().take(64).collect::<String>();
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(message_id) = message_id {
            transaction
                .execute(
                    "UPDATE message_deliveries SET state = 'failed', failure_reason = ?3,
                        updated_at_ms = ?4
                     WHERE message_id = ?1 AND recipient_device_id = ?2
                       AND state NOT IN ('stored', 'completed', 'cancelled')",
                    params![
                        message_id.to_string(),
                        peer_device_id.to_string(),
                        failure_reason,
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
            update_message_delivery_aggregate(&transaction, message_id)?;
        }
        if let Some(transfer_id) = transfer_id {
            transaction
                .execute(
                    "UPDATE transfers SET state = 'failed', failure_reason = ?2,
                        updated_at_ms = ?3
                     WHERE transfer_id = ?1 AND state NOT IN ('completed', 'cancelled')",
                    params![transfer_id.to_string(), failure_reason, now],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "DELETE FROM outbox WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), event_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn mark_text_delivered(
        &mut self,
        peer_device_id: DeviceId,
        original_event_id: EventId,
        message_id: MessageId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'stored', updated_at_ms = ?3
                 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![message_id.to_string(), peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        update_message_delivery_aggregate(&transaction, message_id)?;
        transaction
            .execute(
                "DELETE FROM outbox WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), original_event_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn mark_file_offer_stored(
        &mut self,
        peer_device_id: DeviceId,
        original_event_id: EventId,
        message_id: MessageId,
        transfer_id: TransferId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = CASE
                        WHEN state IN ('queued', 'offered') THEN 'offered'
                        ELSE state
                    END, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND peer_device_id = ?2
                   AND direction = 'send'",
                params![transfer_id.to_string(), peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        transaction
            .execute(
                "UPDATE message_deliveries SET state = CASE
                        WHEN state = 'queued' THEN 'stored'
                        ELSE state
                    END, updated_at_ms = ?3
                 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![message_id.to_string(), peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET state = CASE
                        WHEN state = 'queued' THEN 'sending'
                        ELSE state
                    END WHERE message_id = ?1",
                [message_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "DELETE FROM outbox WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), original_event_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_text(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        message_id: MessageId,
        conversation_id: &str,
        text: &str,
        created_at_ms: i64,
        group_revision: Option<u64>,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        self.receive_text_message(
            local_device_id,
            sender_device_id,
            event_id,
            message_id,
            conversation_id,
            "text",
            text,
            created_at_ms,
            group_revision,
            receipt_json,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_text_message(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        message_id: MessageId,
        conversation_id: &str,
        message_kind: &str,
        text: &str,
        created_at_ms: i64,
        group_revision: Option<u64>,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        validate_text_message(message_kind, text)?;
        if conversation_id.starts_with("g:") {
            return self.receive_group_text(
                local_device_id,
                sender_device_id,
                event_id,
                message_id,
                conversation_id,
                message_kind,
                text,
                created_at_ms,
                group_revision,
                receipt_json,
            );
        }
        if group_revision.is_some() {
            return Err(StorageError::PeerNotFound);
        }
        let expected = PrivateConversationId::new(local_device_id, sender_device_id)
            .map_err(|_| StorageError::PeerNotFound)?
            .to_string();
        if conversation_id != expected {
            return Err(StorageError::PeerNotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = transaction
            .query_row(
                "SELECT receipt_json FROM processed_events
                 WHERE sender_device_id = ?1 AND event_id = ?2",
                params![sender_device_id.to_string(), event_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .flatten()
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let sender_name = transaction
            .query_row(
                "SELECT device_name FROM peers WHERE device_id = ?1",
                [sender_device_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::PeerNotFound)?;
        let now = unix_time_ms();
        transaction
            .execute(
                "INSERT INTO conversations(
                    conversation_id, kind, state, peer_device_id, title_cache,
                    last_activity_at_ms, unread_count, created_at_ms
                 ) VALUES (?1, 'private', 'active', ?2, ?3, ?4, 0, ?4)
                 ON CONFLICT(conversation_id) DO UPDATE SET
                    title_cache = excluded.title_cache,
                    state = 'active', deleted_at_ms = NULL",
                params![
                    conversation_id,
                    sender_device_id.to_string(),
                    sender_name,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO messages(
                    message_id, conversation_id, sender_device_id, kind, state,
                    text_content, created_at_ms, received_at_ms, local_sort_order
                 ) VALUES (?1, ?2, ?3, ?4, 'delivered', ?5, ?6, ?7, ?8)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    sender_device_id.to_string(),
                    message_kind,
                    text,
                    created_at_ms,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2,
                    last_activity_at_ms = ?3, unread_count = unread_count + 1
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO processed_events(
                    sender_device_id, event_id, event_type, receipt_json, processed_at_ms
                 ) VALUES (?1, ?2, 'text_message', ?3, ?4)",
                params![
                    sender_device_id.to_string(),
                    event_id.to_string(),
                    receipt_json,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_file_offer(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        message_id: MessageId,
        transfer_id: TransferId,
        conversation_id: &str,
        created_at_ms: i64,
        group_revision: Option<u64>,
        manifest: &TransferManifest,
        clipboard_metadata: Option<&ClipboardImageMetadata>,
        receipt_json: &str,
    ) -> Result<ReceiveOfferResult, StorageError> {
        validate_manifest(manifest)?;
        validate_clipboard_image_offer_metadata(
            manifest.message_kind,
            clipboard_metadata.map(|metadata| metadata.content_fingerprint.as_str()),
            clipboard_metadata.is_some_and(|metadata| metadata.automatic),
        )?;
        if clipboard_metadata.is_some_and(|metadata| {
            metadata.origin_device_id != sender_device_id || metadata.clipboard_sequence == 0
        }) {
            return Err(StorageError::InvalidClipboardImageMetadata);
        }
        let is_group = conversation_id.starts_with("g:");
        if !is_group {
            let expected = PrivateConversationId::new(local_device_id, sender_device_id)
                .map_err(|_| StorageError::PeerNotFound)?
                .to_string();
            if conversation_id != expected || group_revision.is_some() {
                return Err(StorageError::PeerNotFound);
            }
        }
        let total_size = to_sql_u64(manifest.total_size)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = transaction
            .query_row(
                "SELECT receipt_json FROM processed_events
                 WHERE sender_device_id = ?1 AND event_id = ?2",
                params![sender_device_id.to_string(), event_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .flatten()
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(ReceiveOfferResult {
                transfer_id,
                receipt_json: saved,
                duplicate: true,
                auto_receive_base_ref: None,
            });
        }
        if let Some(metadata) = clipboard_metadata {
            let duplicate_sequence = transaction
                .query_row(
                    "SELECT 1 FROM clipboard_dedup
                     WHERE origin_device_id = ?1 AND clipboard_sequence = ?2",
                    params![
                        metadata.origin_device_id.to_string(),
                        i64::try_from(metadata.clipboard_sequence)
                            .map_err(|_| StorageError::ClipboardSequenceExhausted)?,
                    ],
                    |_| Ok(()),
                )
                .optional()
                .map_err(StorageError::Write)?
                .is_some();
            if duplicate_sequence {
                insert_processed_event(
                    &transaction,
                    sender_device_id,
                    event_id,
                    "file_offer",
                    receipt_json,
                    unix_time_ms(),
                )?;
                transaction.commit().map_err(StorageError::Write)?;
                return Ok(ReceiveOfferResult {
                    transfer_id,
                    receipt_json: receipt_json.to_owned(),
                    duplicate: true,
                    auto_receive_base_ref: None,
                });
            }
        }
        if is_group {
            let group_id: GroupId = conversation_id
                .parse()
                .map_err(|_| StorageError::GroupNotFound)?;
            let offered_revision = group_revision.ok_or(StorageError::GroupRevisionConflict)?;
            let (local_revision, state, local_joined, sender_joined) = transaction
                .query_row(
                    "SELECT g.revision, g.state,
                        EXISTS(SELECT 1 FROM group_members l WHERE l.group_id = g.group_id
                            AND l.device_id = ?2 AND l.membership = 'joined'),
                        EXISTS(SELECT 1 FROM group_members s WHERE s.group_id = g.group_id
                            AND s.device_id = ?3 AND s.membership = 'joined')
                     FROM groups g WHERE g.group_id = ?1",
                    params![
                        group_id.to_string(),
                        local_device_id.to_string(),
                        sender_device_id.to_string()
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, bool>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(StorageError::Write)?
                .ok_or(StorageError::GroupNotFound)?;
            if state != "active"
                || !local_joined
                || !sender_joined
                || offered_revision > local_revision as u64
            {
                return Err(StorageError::GroupRevisionConflict);
            }
        }
        let (sender_name, relation, receive_override) = transaction
            .query_row(
                "SELECT device_name, relation, receive_policy_override
                 FROM peers WHERE device_id = ?1",
                [sender_device_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::PeerNotFound)?;
        let (default_policy, default_receive_ref) = transaction
            .query_row(
                "SELECT default_receive_policy, default_receive_ref
                 FROM app_settings WHERE singleton_id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(StorageError::Write)?;
        let mut auto_receive_base_ref = (relation != "nearby"
            && receive_override.as_deref().unwrap_or(&default_policy) == "auto_accept")
            .then_some(default_receive_ref.clone())
            .flatten();
        if let Some(metadata) = clipboard_metadata
            && metadata.automatic
        {
            let mode = transaction
                .query_row(
                    "SELECT clipboard_mode FROM own_device_bindings
                     WHERE peer_device_id = ?1 AND state = 'active'",
                    [sender_device_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(StorageError::Write)?;
            if !matches!(mode.as_deref(), Some("receive_only" | "bidirectional")) {
                return Err(StorageError::ClipboardBindingRequired);
            }
            auto_receive_base_ref = default_receive_ref;
        }
        let now = unix_time_ms();
        if !is_group {
            transaction
                .execute(
                    "INSERT INTO conversations(
                        conversation_id, kind, state, peer_device_id, title_cache,
                        last_activity_at_ms, unread_count, created_at_ms
                     ) VALUES (?1, 'private', 'active', ?2, ?3, ?4, 0, ?4)
                     ON CONFLICT(conversation_id) DO UPDATE SET
                        title_cache = excluded.title_cache,
                        state = 'active', deleted_at_ms = NULL",
                    params![
                        conversation_id,
                        sender_device_id.to_string(),
                        sender_name,
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
        }
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(
                    message_id, conversation_id, sender_device_id, kind, state,
                    display_name, total_size, entry_count, created_at_ms,
                    received_at_ms, local_sort_order, group_revision
                 ) VALUES (?1, ?2, ?3, ?4, 'delivered', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    sender_device_id.to_string(),
                    message_kind_name(manifest.message_kind),
                    manifest.display_name,
                    total_size,
                    i64::from(manifest.entry_count),
                    created_at_ms,
                    now,
                    sort_order,
                    group_revision.map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                ],
            )
            .map_err(StorageError::Write)?;
        if let Some(metadata) = clipboard_metadata {
            insert_clipboard_image_dedup(&transaction, message_id, metadata, now)?;
        }
        transaction
            .execute(
                "INSERT INTO transfers(
                    transfer_id, message_id, peer_device_id, direction, state,
                    display_name, total_size, entry_count, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, 'receive', 'offered', ?4, ?5, ?6, ?7, ?7)",
                params![
                    transfer_id.to_string(),
                    message_id.to_string(),
                    sender_device_id.to_string(),
                    manifest.display_name,
                    total_size,
                    i64::from(manifest.entry_count),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        insert_transfer_entries(&transaction, transfer_id, &manifest.entries, false, now)?;
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2,
                    last_activity_at_ms = ?3, unread_count = unread_count + 1,
                    deleted_at_ms = NULL
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO processed_events(
                    sender_device_id, event_id, event_type, receipt_json, processed_at_ms
                 ) VALUES (?1, ?2, 'file_offer', ?3, ?4)",
                params![
                    sender_device_id.to_string(),
                    event_id.to_string(),
                    receipt_json,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(ReceiveOfferResult {
            transfer_id,
            receipt_json: receipt_json.to_owned(),
            duplicate: false,
            auto_receive_base_ref,
        })
    }

    pub fn accept_incoming_offer(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
        receive_base_ref: &str,
        prepared: &[PreparedReceiveEntry],
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "decide_incoming_offer",
        )? {
            return Ok(saved);
        }
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.direction != "receive" || transfer.state != "offered" {
            return Err(StorageError::TransferNotFound);
        }
        let prepared_by_id = prepared
            .iter()
            .map(|entry| (entry.entry_id, entry))
            .collect::<HashMap<_, _>>();
        if prepared_by_id.len() != entries.len() {
            return Err(StorageError::TransferNotFound);
        }
        let mut accepted_offsets = Vec::new();
        let mut persisted_bytes = 0_u64;
        for entry in &entries {
            let prepared = prepared_by_id
                .get(&entry.entry_id)
                .ok_or(StorageError::TransferNotFound)?;
            if prepared.persisted_offset > entry.size
                || (entry.entry_kind == TransferEntryKind::File
                    && prepared.partial_ref.as_deref().is_none_or(str::is_empty))
                || (entry.entry_kind == TransferEntryKind::Directory
                    && prepared.partial_ref.is_some())
            {
                return Err(StorageError::TransferNotFound);
            }
            if entry.entry_kind == TransferEntryKind::File {
                accepted_offsets.push(AcceptedOffset {
                    entry_id: entry.entry_id,
                    offset: prepared.persisted_offset,
                });
                persisted_bytes = persisted_bytes
                    .checked_add(prepared.persisted_offset)
                    .ok_or(StorageError::Manifest(ManifestError::SizeOverflow))?;
            }
        }
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "file_accept",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "transfer_id": transfer_id,
                "entries": accepted_offsets,
            }
        })
        .to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<EventId>(
            &transaction,
            client_operation_id,
            "decide_incoming_offer",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        for entry in &entries {
            let prepared = prepared_by_id[&entry.entry_id];
            transaction
                .execute(
                    "UPDATE transfer_entries SET destination_ref = ?2, partial_ref = ?3,
                        persisted_offset = ?4, state = ?5, updated_at_ms = ?6
                     WHERE entry_id = ?1 AND transfer_id = ?7",
                    params![
                        entry.entry_id.to_string(),
                        prepared.destination_ref,
                        prepared.partial_ref,
                        to_sql_u64(prepared.persisted_offset)?,
                        if entry.entry_kind == TransferEntryKind::Directory {
                            "completed"
                        } else {
                            "queued"
                        },
                        now,
                        transfer_id.to_string(),
                    ],
                )
                .map_err(StorageError::Write)?;
        }
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = 'accepted', receive_base_ref = ?2,
                    persisted_bytes = ?3, updated_at_ms = ?4
                 WHERE transfer_id = ?1 AND direction = 'receive' AND state = 'offered'",
                params![
                    transfer_id.to_string(),
                    receive_base_ref,
                    to_sql_u64(persisted_bytes)?,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        transaction
            .execute(
                "UPDATE peers SET relation = CASE WHEN relation = 'nearby' THEN 'known' ELSE relation END,
                    known_at_ms = COALESCE(known_at_ms, ?2), updated_at_ms = ?2
                 WHERE device_id = ?1",
                params![transfer.peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO outbox(
                    peer_device_id, event_id, event_type, payload_json,
                    requires_receipt, state, attempts, next_attempt_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'file_accept', ?3, 1, 'pending', 0, ?4, ?4, ?4)",
                params![
                    transfer.peer_device_id.to_string(),
                    event_id.to_string(),
                    payload_json,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "decide_incoming_offer",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn reject_incoming_offer(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
        network_reason: &str,
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "decide_incoming_offer",
        )? {
            return Ok(saved);
        }
        if !matches!(
            network_reason,
            "user_rejected" | "not_enough_space" | "invalid_path" | "unsupported"
        ) {
            return Err(StorageError::TransferNotFound);
        }
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != "receive" || transfer.state != "offered" {
            return Err(StorageError::TransferNotFound);
        }
        let failure_reason = match network_reason {
            "user_rejected" => "rejected",
            "not_enough_space" => "not_enough_space",
            "invalid_path" => "invalid_path",
            _ => "unsupported",
        };
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "file_reject",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "transfer_id": transfer_id,
                "reason": network_reason,
            }
        })
        .to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<EventId>(
            &transaction,
            client_operation_id,
            "decide_incoming_offer",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        transaction
            .execute(
                "UPDATE transfers SET state = 'failed', failure_reason = ?2, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = 'receive' AND state = 'offered'",
                params![transfer_id.to_string(), failure_reason, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO outbox(
                    peer_device_id, event_id, event_type, payload_json,
                    requires_receipt, state, attempts, next_attempt_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'file_reject', ?3, 1, 'pending', 0, ?4, ?4, ?4)",
                params![
                    transfer.peer_device_id.to_string(),
                    event_id.to_string(),
                    payload_json,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "decide_incoming_offer",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn receive_file_accept(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
        offsets: &[AcceptedOffset],
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.direction != "send" || transfer.peer_device_id != sender_device_id {
            return Err(StorageError::TransferNotFound);
        }
        let offsets = offsets
            .iter()
            .map(|offset| (offset.entry_id, offset.offset))
            .collect::<HashMap<_, _>>();
        let file_count = entries
            .iter()
            .filter(|entry| entry.entry_kind == TransferEntryKind::File)
            .count();
        if offsets.len() != file_count {
            return Err(StorageError::TransferNotFound);
        }
        let mut persisted_bytes = 0_u64;
        for entry in entries
            .iter()
            .filter(|entry| entry.entry_kind == TransferEntryKind::File)
        {
            let offset = offsets
                .get(&entry.entry_id)
                .copied()
                .ok_or(StorageError::TransferNotFound)?;
            if offset > entry.size {
                return Err(StorageError::TransferNotFound);
            }
            persisted_bytes = persisted_bytes
                .checked_add(offset)
                .ok_or(StorageError::Manifest(ManifestError::SizeOverflow))?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        for (entry_id, offset) in &offsets {
            transaction
                .execute(
                    "UPDATE transfer_entries SET persisted_offset = ?2, state = 'queued',
                        updated_at_ms = ?3 WHERE entry_id = ?1 AND transfer_id = ?4",
                    params![
                        entry_id.to_string(),
                        to_sql_u64(*offset)?,
                        unix_time_ms(),
                        transfer_id.to_string(),
                    ],
                )
                .map_err(StorageError::Write)?;
        }
        let now = unix_time_ms();
        transaction
            .execute(
                "UPDATE transfers SET state = CASE
                        WHEN paused_by_user = 1 THEN 'paused' ELSE 'accepted'
                    END, persisted_bytes = ?2, failure_reason = NULL, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = 'send'
                    AND state IN ('offered', 'queued', 'paused', 'accepted')",
                params![transfer_id.to_string(), to_sql_u64(persisted_bytes)?, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'accepted', updated_at_ms = ?3
                 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![
                    transfer.message_id.to_string(),
                    sender_device_id.to_string(),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "file_accept",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn receive_file_reject(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
        failure_reason: &str,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != "send" || transfer.peer_device_id != sender_device_id {
            return Err(StorageError::TransferNotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let now = unix_time_ms();
        transaction
            .execute(
                "UPDATE transfers SET state = 'failed', failure_reason = ?2, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = 'send'",
                params![transfer_id.to_string(), failure_reason, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'rejected', failure_reason = ?3,
                    updated_at_ms = ?4 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![
                    transfer.message_id.to_string(),
                    sender_device_id.to_string(),
                    failure_reason,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET state = 'failed' WHERE message_id = ?1",
                [transfer.message_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "file_reject",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn mark_control_event_stored(
        &self,
        peer_device_id: DeviceId,
        original_event_id: EventId,
    ) -> Result<(), StorageError> {
        self.connection
            .execute(
                "DELETE FROM outbox WHERE peer_device_id = ?1 AND event_id = ?2",
                params![peer_device_id.to_string(), original_event_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn pause_transfer(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "pause_transfer",
        )? {
            return Ok(saved);
        }
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if !matches!(
            transfer.state.as_str(),
            "queued" | "accepted" | "transferring"
        ) {
            return Err(StorageError::TransferNotFound);
        }
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = transfer_control_payload(
            profile,
            event_id,
            "transfer_pause",
            serde_json::json!({
                "transfer_id": transfer_id,
                "reason": "user",
            }),
            now,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) =
            saved_client_operation::<EventId>(&transaction, client_operation_id, "pause_transfer")?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = 'paused', paused_by_user = 1,
                    failure_reason = NULL, updated_at_ms = ?2
                 WHERE transfer_id = ?1
                   AND state IN ('queued', 'accepted', 'transferring')",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?2
                 WHERE transfer_id = ?1 AND state = 'transferring'",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "transfer_pause",
            &payload_json,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "pause_transfer",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn resume_transfer(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "resume_transfer",
        )? {
            return Ok(saved);
        }
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.state != "paused" && !transfer.paused_by_user {
            return Err(StorageError::TransferNotFound);
        }
        let offsets = file_offsets(&entries);
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = transfer_control_payload(
            profile,
            event_id,
            "transfer_resume",
            serde_json::json!({
                "transfer_id": transfer_id,
                "entries": offsets,
            }),
            now,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) =
            saved_client_operation::<EventId>(&transaction, client_operation_id, "resume_transfer")?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = 'accepted', paused_by_user = 0,
                    failure_reason = NULL, updated_at_ms = ?2
                 WHERE transfer_id = ?1
                   AND (state = 'paused' OR paused_by_user = 1)
                   AND state NOT IN ('completed', 'cancelled')",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "transfer_resume",
            &payload_json,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "resume_transfer",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn cancel_transfer(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "cancel_transfer",
        )? {
            return Ok(saved);
        }
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if matches!(transfer.state.as_str(), "completed" | "cancelled") {
            return Err(StorageError::TransferNotFound);
        }
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = transfer_control_payload(
            profile,
            event_id,
            "transfer_cancel",
            serde_json::json!({
                "transfer_id": transfer_id,
                "cancelled_by": profile.device_id,
            }),
            now,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) =
            saved_client_operation::<EventId>(&transaction, client_operation_id, "cancel_transfer")?
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        mark_transfer_cancelled(&transaction, &transfer, now)?;
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "transfer_cancel",
            &payload_json,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "cancel_transfer",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        remove_partial_files(&entries);
        Ok(event_id)
    }

    pub fn receive_transfer_pause(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.peer_device_id != sender_device_id {
            return Err(StorageError::TransferNotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let now = unix_time_ms();
        if !matches!(
            transfer.state.as_str(),
            "completed" | "cancelled" | "failed"
        ) {
            transaction
                .execute(
                    "UPDATE transfers SET state = 'paused', paused_by_user = 0,
                        failure_reason = NULL, updated_at_ms = ?2
                     WHERE transfer_id = ?1",
                    params![transfer_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
            transaction
                .execute(
                    "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?2
                     WHERE transfer_id = ?1 AND state = 'transferring'",
                    params![transfer_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "transfer_pause",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn receive_transfer_resume(
        &mut self,
        profile: &LocalProfile,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
        offsets: &[AcceptedOffset],
        receipt_json: &str,
    ) -> Result<ReceiveTransferControlResult, StorageError> {
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.peer_device_id != sender_device_id
            || matches!(transfer.state.as_str(), "completed" | "cancelled")
        {
            return Err(StorageError::TransferNotFound);
        }
        validate_offsets(&entries, offsets)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(ReceiveTransferControlResult {
                receipt_json: saved,
                queued_resume_reply: false,
            });
        }
        let now = unix_time_ms();
        let mut queued_resume_reply = false;
        if transfer.direction == "send" {
            let offset_by_id = offsets
                .iter()
                .map(|offset| (offset.entry_id, offset.offset))
                .collect::<HashMap<_, _>>();
            for entry in entries
                .iter()
                .filter(|entry| entry.entry_kind == TransferEntryKind::File)
            {
                transaction
                    .execute(
                        "UPDATE transfer_entries SET persisted_offset = ?2, state = 'queued',
                            updated_at_ms = ?3 WHERE transfer_id = ?1 AND entry_id = ?4",
                        params![
                            transfer_id.to_string(),
                            to_sql_u64(offset_by_id[&entry.entry_id])?,
                            now,
                            entry.entry_id.to_string(),
                        ],
                    )
                    .map_err(StorageError::Write)?;
            }
            let persisted = offsets.iter().try_fold(0_u64, |sum, offset| {
                sum.checked_add(offset.offset)
                    .ok_or(StorageError::Manifest(ManifestError::SizeOverflow))
            })?;
            transaction
                .execute(
                    "UPDATE transfers SET state = 'accepted', paused_by_user = 0,
                        failure_reason = NULL, persisted_bytes = ?2, updated_at_ms = ?3
                     WHERE transfer_id = ?1",
                    params![transfer_id.to_string(), to_sql_u64(persisted)?, now],
                )
                .map_err(StorageError::Write)?;
            transaction
                .execute(
                    "UPDATE message_deliveries SET state = 'accepted', failure_reason = NULL,
                        updated_at_ms = ?3
                     WHERE message_id = ?1 AND recipient_device_id = ?2",
                    params![
                        transfer.message_id.to_string(),
                        sender_device_id.to_string(),
                        now,
                    ],
                )
                .map_err(StorageError::Write)?;
            update_message_delivery_aggregate(&transaction, transfer.message_id)?;
        } else {
            transaction
                .execute(
                    "UPDATE transfers SET state = 'accepted', paused_by_user = 0,
                        failure_reason = NULL, updated_at_ms = ?2 WHERE transfer_id = ?1",
                    params![transfer_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
            let reply_event_id = EventId::generate();
            let reply_json = transfer_control_payload(
                profile,
                reply_event_id,
                "transfer_resume",
                serde_json::json!({
                    "transfer_id": transfer_id,
                    "entries": file_offsets(&entries),
                }),
                now,
            );
            insert_outbox_event(
                &transaction,
                sender_device_id,
                reply_event_id,
                "transfer_resume",
                &reply_json,
                now,
            )?;
            queued_resume_reply = true;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "transfer_resume",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(ReceiveTransferControlResult {
            receipt_json: receipt_json.to_owned(),
            queued_resume_reply,
        })
    }

    pub fn receive_transfer_cancel(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.peer_device_id != sender_device_id {
            return Err(StorageError::TransferNotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let now = unix_time_ms();
        if transfer.state != "completed" {
            mark_transfer_cancelled(&transaction, &transfer, now)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "transfer_cancel",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        if transfer.state != "completed" {
            remove_partial_files(&entries);
        }
        Ok(receipt_json.to_owned())
    }

    pub fn reconcile_interrupted_transfers(
        &mut self,
        profile: &LocalProfile,
    ) -> Result<(), StorageError> {
        let receive_entries = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT e.entry_id, e.transfer_id, e.size, e.persisted_offset,
                            e.partial_ref, e.destination_ref
                     FROM transfer_entries e
                     JOIN transfers t ON t.transfer_id = e.transfer_id
                     WHERE t.direction = 'receive'
                       AND t.state IN ('accepted', 'transferring', 'queued', 'paused')
                       AND e.entry_kind = 'file'",
                )
                .map_err(StorageError::Write)?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })
                .map_err(StorageError::Write)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Write)?
        };

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let now = unix_time_ms();
        for (entry_id, transfer_id, size_raw, persisted_raw, partial, destination) in
            receive_entries
        {
            let size = from_sql_u64(size_raw)?;
            let persisted = from_sql_u64(persisted_raw)?.min(size);
            let destination_complete = destination.as_deref().is_some_and(|path| {
                fs::metadata(path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
            });
            let (safe_offset, entry_state) = if partial
                .as_deref()
                .is_some_and(|path| path.starts_with("content://"))
            {
                (
                    persisted,
                    if persisted == size {
                        "completed"
                    } else {
                        "queued"
                    },
                )
            } else if destination_complete {
                (size, "completed")
            } else if let Some(partial) = partial.as_deref() {
                let actual = fs::metadata(partial)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
                    .min(size);
                let safe = persisted.min(actual);
                if actual > safe {
                    fs::OpenOptions::new()
                        .write(true)
                        .open(partial)
                        .and_then(|file| file.set_len(safe))
                        .map_err(|error| StorageError::DirectoryUnwritable(error.to_string()))?;
                }
                (safe, if safe == size { "completed" } else { "queued" })
            } else {
                (0, "queued")
            };
            transaction
                .execute(
                    "UPDATE transfer_entries SET persisted_offset = ?3, state = ?4,
                        updated_at_ms = ?5 WHERE entry_id = ?1 AND transfer_id = ?2",
                    params![
                        entry_id,
                        transfer_id,
                        to_sql_u64(safe_offset)?,
                        entry_state,
                        now,
                    ],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?1
                 WHERE state = 'transferring'",
                [now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET state = CASE WHEN paused_by_user = 1
                        THEN 'paused' ELSE 'queued' END,
                    failure_reason = CASE WHEN paused_by_user = 1
                        THEN NULL ELSE 'connection_error' END,
                    updated_at_ms = ?1 WHERE state = 'transferring'",
                [now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET persisted_bytes = (
                    SELECT COALESCE(SUM(e.persisted_offset), 0)
                    FROM transfer_entries e
                    WHERE e.transfer_id = transfers.transfer_id
                      AND e.entry_kind = 'file'
                 ), updated_at_ms = ?1
                 WHERE state IN ('accepted', 'queued', 'paused')",
                [now],
            )
            .map_err(StorageError::Write)?;

        let resumable = {
            let mut statement = transaction
                .prepare(
                    "SELECT transfer_id FROM transfers
                     WHERE state IN ('accepted', 'queued') AND paused_by_user = 0",
                )
                .map_err(StorageError::Write)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(StorageError::Write)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Write)?
        };
        for transfer_id_text in resumable {
            let transfer_id = transfer_id_text
                .parse::<TransferId>()
                .map_err(|_| StorageError::InvalidStoredId)?;
            let peer_text = transaction
                .query_row(
                    "SELECT peer_device_id FROM transfers WHERE transfer_id = ?1",
                    [transfer_id_text.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .map_err(StorageError::Write)?;
            let peer_device_id = peer_text
                .parse::<DeviceId>()
                .map_err(|_| StorageError::InvalidStoredId)?;
            let offsets = {
                let mut statement = transaction
                    .prepare(
                        "SELECT entry_id, persisted_offset FROM transfer_entries
                         WHERE transfer_id = ?1 AND entry_kind = 'file' ORDER BY rowid",
                    )
                    .map_err(StorageError::Write)?;
                statement
                    .query_map([transfer_id_text.as_str()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .map_err(StorageError::Write)?
                    .map(|row| {
                        let (entry_id, offset) = row.map_err(StorageError::Write)?;
                        Ok(AcceptedOffset {
                            entry_id: entry_id
                                .parse()
                                .map_err(|_| StorageError::InvalidStoredId)?,
                            offset: from_sql_u64(offset)?,
                        })
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            };
            transaction
                .execute(
                    "DELETE FROM outbox WHERE peer_device_id = ?1
                       AND event_type = 'transfer_resume'
                       AND json_extract(payload_json, '$.body.transfer_id') = ?2",
                    params![peer_device_id.to_string(), transfer_id_text],
                )
                .map_err(StorageError::Write)?;
            let event_id = EventId::generate();
            let payload_json = transfer_control_payload(
                profile,
                event_id,
                "transfer_resume",
                serde_json::json!({
                    "transfer_id": transfer_id,
                    "entries": offsets,
                }),
                now,
            );
            insert_outbox_event(
                &transaction,
                peer_device_id,
                event_id,
                "transfer_resume",
                &payload_json,
                now,
            )?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn schedulable_send_transfers(
        &self,
        limit: u32,
    ) -> Result<Vec<(TransferId, String)>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT t.transfer_id, p.last_ip
                 FROM transfers t
                 JOIN message_deliveries d ON d.message_id = t.message_id
                    AND d.recipient_device_id = t.peer_device_id
                 JOIN peers p ON p.device_id = t.peer_device_id
                 WHERE t.direction = 'send' AND t.state IN ('accepted', 'queued')
                   AND t.paused_by_user = 0 AND d.state = 'accepted'
                   AND p.last_ip IS NOT NULL
                 ORDER BY CASE WHEN t.persisted_bytes > 0 THEN 0 ELSE 1 END,
                          t.updated_at_ms, t.created_at_ms
                 LIMIT ?1",
            )
            .map_err(StorageError::Write)?;
        statement
            .query_map([i64::from(limit.clamp(1, 4))], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(StorageError::Write)?
            .map(|row| {
                let (transfer_id, ip) = row.map_err(StorageError::Write)?;
                Ok((
                    transfer_id
                        .parse()
                        .map_err(|_| StorageError::InvalidStoredId)?,
                    ip,
                ))
            })
            .collect()
    }

    pub fn claim_send_transfer(&self, transfer_id: TransferId) -> Result<bool, StorageError> {
        let now = unix_time_ms();
        self.connection
            .execute(
                "UPDATE transfers SET state = 'transferring', failure_reason = NULL,
                    updated_at_ms = ?2
                 WHERE transfer_id = ?1 AND direction = 'send'
                   AND state IN ('accepted', 'queued') AND paused_by_user = 0
                   AND EXISTS (
                     SELECT 1 FROM message_deliveries d
                     WHERE d.message_id = transfers.message_id
                       AND d.recipient_device_id = transfers.peer_device_id
                       AND d.state = 'accepted'
                   )",
                params![transfer_id.to_string(), now],
            )
            .map(|changed| changed == 1)
            .map_err(StorageError::Write)
    }

    pub fn claim_receive_transfer(
        &self,
        transfer_id: TransferId,
        peer_device_id: DeviceId,
    ) -> Result<bool, StorageError> {
        let now = unix_time_ms();
        self.connection
            .execute(
                "UPDATE transfers SET state = 'transferring', failure_reason = NULL,
                    updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND peer_device_id = ?2 AND direction = 'receive'
                   AND state IN ('accepted', 'queued') AND paused_by_user = 0",
                params![transfer_id.to_string(), peer_device_id.to_string(), now],
            )
            .map(|changed| changed == 1)
            .map_err(StorageError::Write)
    }

    pub fn persist_transfer_offset(
        &mut self,
        transfer_id: TransferId,
        entry_id: EntryId,
        offset: u64,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let changed = transaction
            .execute(
                "UPDATE transfer_entries SET persisted_offset = ?3,
                    state = CASE WHEN ?3 = size THEN 'completed' ELSE 'transferring' END,
                    updated_at_ms = ?4
                 WHERE transfer_id = ?1 AND entry_id = ?2 AND entry_kind = 'file'
                   AND ?3 BETWEEN persisted_offset AND size",
                params![
                    transfer_id.to_string(),
                    entry_id.to_string(),
                    to_sql_u64(offset)?,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        transaction
            .execute(
                "UPDATE transfers SET persisted_bytes = (
                    SELECT COALESCE(SUM(persisted_offset), 0) FROM transfer_entries
                    WHERE transfer_id = ?1 AND entry_kind = 'file'
                 ), updated_at_ms = ?2 WHERE transfer_id = ?1",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn interrupt_transfer(&self, transfer_id: TransferId) -> Result<(), StorageError> {
        let now = unix_time_ms();
        self.connection
            .execute(
                "UPDATE transfers SET state = 'queued', failure_reason = 'connection_error',
                    updated_at_ms = ?2
                 WHERE transfer_id = ?1 AND state = 'transferring' AND paused_by_user = 0",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        self.connection
            .execute(
                "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?2
                 WHERE transfer_id = ?1 AND state = 'transferring'",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn suspend_network_transfers(&mut self) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let now = unix_time_ms();
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?1
                 WHERE state = 'transferring'",
                [now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET state = 'queued', paused_by_user = 0,
                    failure_reason = CASE
                        WHEN state IN ('transferring', 'verifying') THEN 'connection_error'
                        ELSE failure_reason
                    END,
                    updated_at_ms = ?1
                 WHERE state IN ('accepted', 'transferring', 'verifying')
                   AND paused_by_user = 0",
                [now],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn fail_receive_transfer(
        &mut self,
        profile: &LocalProfile,
        transfer_id: TransferId,
        failure_reason: &str,
    ) -> Result<EventId, StorageError> {
        if !matches!(
            failure_reason,
            "not_enough_space" | "permission_lost" | "invalid_path"
        ) {
            return Err(StorageError::TransferNotFound);
        }
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != "receive" {
            return Err(StorageError::TransferNotFound);
        }
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "file_reject",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "transfer_id": transfer_id,
                "reason": failure_reason,
            }
        })
        .to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = 'failed', failure_reason = ?2,
                    paused_by_user = 0, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = 'receive'
                   AND state IN ('accepted', 'queued', 'transferring', 'verifying')",
                params![transfer_id.to_string(), failure_reason, now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'queued', updated_at_ms = ?2
                 WHERE transfer_id = ?1 AND state = 'transferring'",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "file_reject",
            &payload_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn fail_send_transfer_source(
        &mut self,
        transfer_id: TransferId,
        failure_reason: &str,
    ) -> Result<(), StorageError> {
        if !matches!(failure_reason, "source_changed" | "permission_lost") {
            return Err(StorageError::TransferNotFound);
        }
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != "send" {
            return Err(StorageError::TransferNotFound);
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET state = 'failed', failure_reason = ?2,
                    paused_by_user = 0, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = 'send'",
                params![transfer_id.to_string(), failure_reason, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'failed', failure_reason = ?3,
                    updated_at_ms = ?4
                 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![
                    transfer.message_id.to_string(),
                    transfer.peer_device_id.to_string(),
                    failure_reason,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET state = 'failed' WHERE message_id = ?1",
                [transfer.message_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn fail_transfer_source_changed(
        &self,
        transfer_id: TransferId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        self.connection
            .execute(
                "UPDATE transfers SET state = 'failed', failure_reason = 'source_changed',
                    updated_at_ms = ?2 WHERE transfer_id = ?1 AND direction = 'send'",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        self.connection
            .execute(
                "UPDATE messages SET state = 'failed' WHERE message_id = (
                    SELECT message_id FROM transfers WHERE transfer_id = ?1
                 )",
                [transfer_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn replace_receive_destination(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
        receive_base_ref: &str,
        prepared: &[PreparedReceiveEntry],
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "replace_receive_destination",
        )? {
            return Ok(saved);
        }
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.direction != "receive"
            || transfer.state != "failed"
            || !matches!(
                transfer.failure_reason.as_deref(),
                Some("not_enough_space" | "permission_lost" | "invalid_path")
            )
            || receive_base_ref.trim().is_empty()
        {
            return Err(StorageError::TransferNotFound);
        }
        let prepared_by_id = prepared
            .iter()
            .map(|entry| (entry.entry_id, entry))
            .collect::<HashMap<_, _>>();
        if prepared_by_id.len() != entries.len() {
            return Err(StorageError::TransferNotFound);
        }

        let mut offsets = Vec::new();
        let mut persisted_bytes = 0_u64;
        for entry in &entries {
            let prepared = prepared_by_id
                .get(&entry.entry_id)
                .ok_or(StorageError::TransferNotFound)?;
            if prepared.persisted_offset > entry.size
                || prepared.destination_ref.trim().is_empty()
                || (entry.entry_kind == TransferEntryKind::File
                    && prepared.partial_ref.as_deref().is_none_or(str::is_empty))
                || (entry.entry_kind == TransferEntryKind::Directory
                    && prepared.partial_ref.is_some())
            {
                return Err(StorageError::TransferNotFound);
            }
            if entry.entry_kind == TransferEntryKind::File {
                offsets.push(AcceptedOffset {
                    entry_id: entry.entry_id,
                    offset: prepared.persisted_offset,
                });
                persisted_bytes = persisted_bytes
                    .checked_add(prepared.persisted_offset)
                    .ok_or(StorageError::Manifest(ManifestError::SizeOverflow))?;
            }
        }

        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = transfer_control_payload(
            profile,
            event_id,
            "transfer_resume",
            serde_json::json!({
                "transfer_id": transfer_id,
                "entries": offsets,
            }),
            now,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<EventId>(
            &transaction,
            client_operation_id,
            "replace_receive_destination",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        for entry in &entries {
            let prepared = prepared_by_id[&entry.entry_id];
            transaction
                .execute(
                    "UPDATE transfer_entries SET destination_ref = ?2, partial_ref = ?3,
                        persisted_offset = ?4, state = ?5, updated_at_ms = ?6
                     WHERE entry_id = ?1 AND transfer_id = ?7",
                    params![
                        entry.entry_id.to_string(),
                        prepared.destination_ref,
                        prepared.partial_ref,
                        to_sql_u64(prepared.persisted_offset)?,
                        if entry.entry_kind == TransferEntryKind::Directory {
                            "completed"
                        } else {
                            "queued"
                        },
                        now,
                        transfer_id.to_string(),
                    ],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "UPDATE transfers SET state = 'accepted', failure_reason = NULL,
                    receive_base_ref = ?2, persisted_bytes = ?3, paused_by_user = 0,
                    updated_at_ms = ?4 WHERE transfer_id = ?1",
                params![
                    transfer_id.to_string(),
                    receive_base_ref,
                    to_sql_u64(persisted_bytes)?,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "transfer_resume",
            &payload_json,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "replace_receive_destination",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn replace_transfer_sources(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        transfer_id: TransferId,
        sources: &[SourceItem],
    ) -> Result<EventId, StorageError> {
        if let Some(saved) = replay_client_operation::<EventId>(
            &mut self.connection,
            client_operation_id,
            "replace_transfer_source",
        )? {
            return Ok(saved);
        }
        let (transfer, entries) = self.get_transfer(transfer_id)?;
        if transfer.direction != "send"
            || transfer.state != "failed"
            || !matches!(
                transfer.failure_reason.as_deref(),
                Some("source_changed" | "permission_lost")
            )
            || entries.len() != sources.len()
        {
            return Err(StorageError::TransferSourceMismatch);
        }
        let source_by_path: HashMap<&str, &SourceItem> = sources
            .iter()
            .map(|source| (source.relative_path.as_str(), source))
            .collect();
        if source_by_path.len() != sources.len()
            || entries.iter().any(|entry| {
                let Some(source) = source_by_path.get(entry.relative_path.as_str()) else {
                    return true;
                };
                source.entry_kind != entry.entry_kind
                    || source.size != entry.size
                    || source.modified_at_ms != entry.modified_at_ms
                    || (entry.entry_kind == TransferEntryKind::File
                        && source.source_ref.as_deref().is_none_or(str::is_empty))
            })
        {
            return Err(StorageError::TransferSourceMismatch);
        }

        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = transfer_control_payload(
            profile,
            event_id,
            "transfer_resume",
            serde_json::json!({
                "transfer_id": transfer_id,
                "entries": file_offsets(&entries),
            }),
            now,
        );
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<EventId>(
            &transaction,
            client_operation_id,
            "replace_transfer_source",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        for entry in &entries {
            let source = source_by_path[entry.relative_path.as_str()];
            transaction
                .execute(
                    "UPDATE transfer_entries SET source_ref = ?2, state = 'queued',
                        updated_at_ms = ?3 WHERE entry_id = ?1",
                    params![entry.entry_id.to_string(), source.source_ref, now],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "UPDATE transfers SET state = 'accepted', failure_reason = NULL,
                    paused_by_user = 0, updated_at_ms = ?2 WHERE transfer_id = ?1",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET state = 'queued' WHERE message_id = ?1",
                [transfer.message_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        insert_outbox_event(
            &transaction,
            transfer.peer_device_id,
            event_id,
            "transfer_resume",
            &payload_json,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "replace_transfer_source",
            &event_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(event_id)
    }

    pub fn complete_send_transfer(&mut self, transfer_id: TransferId) -> Result<(), StorageError> {
        self.complete_transfer(transfer_id, "send")
    }

    pub fn persist_published_destination(
        &self,
        transfer_id: TransferId,
        entry_id: EntryId,
        destination_ref: &str,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let changed = self
            .connection
            .execute(
                "UPDATE transfer_entries
                 SET destination_ref = ?3, partial_ref = NULL, updated_at_ms = ?4
                 WHERE transfer_id = ?1 AND entry_id = ?2 AND entry_kind = 'file'
                   AND EXISTS(
                       SELECT 1 FROM transfers t
                       WHERE t.transfer_id = transfer_entries.transfer_id
                         AND t.direction = 'receive'
                   )",
                params![
                    transfer_id.to_string(),
                    entry_id.to_string(),
                    destination_ref,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        Ok(())
    }

    pub fn completed_clipboard_image(
        &self,
        transfer_id: TransferId,
    ) -> Result<Option<CompletedClipboardImage>, StorageError> {
        self.connection
            .query_row(
                "SELECT e.destination_ref, d.origin_device_id, d.clipboard_sequence,
                        d.content_fingerprint
                 FROM transfers t
                 JOIN messages m ON m.message_id = t.message_id
                 JOIN transfer_entries e ON e.transfer_id = t.transfer_id
                 JOIN clipboard_dedup d ON d.message_id = m.message_id
                 JOIN own_device_bindings b ON b.peer_device_id = t.peer_device_id
                 WHERE t.transfer_id = ?1 AND t.direction = 'receive'
                   AND t.state = 'completed' AND m.kind = 'clipboard_image'
                   AND e.entry_kind = 'file' AND b.state = 'active'
                   AND b.clipboard_mode IN ('receive_only', 'bidirectional')
                 LIMIT 1",
                [transfer_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)
            .and_then(|record| {
                record
                    .and_then(|(reference, origin, sequence, fingerprint)| {
                        reference.map(|reference| (reference, origin, sequence, fingerprint))
                    })
                    .map(|(reference, origin, sequence, fingerprint)| {
                        Ok(CompletedClipboardImage {
                            reference,
                            metadata: ClipboardImageMetadata {
                                origin_device_id: origin
                                    .parse()
                                    .map_err(|_| StorageError::InvalidStoredId)?,
                                clipboard_sequence: u64::try_from(sequence)
                                    .map_err(|_| StorageError::InvalidStoredId)?,
                                content_fingerprint: fingerprint,
                                automatic: true,
                            },
                        })
                    })
                    .transpose()
            })
    }

    pub fn complete_receive_transfer(
        &mut self,
        profile: &LocalProfile,
        transfer_id: TransferId,
    ) -> Result<(), StorageError> {
        self.complete_transfer(transfer_id, "receive")?;
        let (transfer, _) = self.get_transfer(transfer_id)?;
        let event_id = EventId::generate();
        let now = unix_time_ms();
        let payload_json = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "original_event_id": event_id,
                "message_id": transfer.message_id,
                "transfer_id": transfer_id,
                "stage": "completed",
                "at_ms": now,
            }
        })
        .to_string();
        self.connection
            .execute(
                "INSERT INTO outbox(
                    peer_device_id, event_id, event_type, payload_json,
                    requires_receipt, state, attempts, next_attempt_at_ms,
                    created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 'delivery_receipt', ?3, 0, 'pending', 0, ?4, ?4, ?4)",
                params![
                    transfer.peer_device_id.to_string(),
                    event_id.to_string(),
                    payload_json,
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn receive_transfer_completed(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        transfer_id: TransferId,
    ) -> Result<(), StorageError> {
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != "send" || transfer.peer_device_id != sender_device_id {
            return Err(StorageError::TransferNotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if saved_receipt(&transaction, sender_device_id, event_id)?.is_some() {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(());
        }
        let now = unix_time_ms();
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'completed', persisted_offset = size,
                    updated_at_ms = ?2 WHERE transfer_id = ?1",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET state = 'completed', failure_reason = NULL,
                    persisted_bytes = total_size, completed_at_ms = COALESCE(completed_at_ms, ?2),
                    updated_at_ms = ?2 WHERE transfer_id = ?1 AND state != 'cancelled'",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'completed', failure_reason = NULL,
                    updated_at_ms = ?3 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![
                    transfer.message_id.to_string(),
                    sender_device_id.to_string(),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
        update_message_delivery_aggregate(&transaction, transfer.message_id)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "delivery_receipt",
            "{}",
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    fn complete_transfer(
        &mut self,
        transfer_id: TransferId,
        direction: &str,
    ) -> Result<(), StorageError> {
        let (transfer, _) = self.get_transfer(transfer_id)?;
        if transfer.direction != direction {
            return Err(StorageError::TransferNotFound);
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'completed', persisted_offset = size,
                    updated_at_ms = ?2 WHERE transfer_id = ?1",
                params![transfer_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let changed = transaction
            .execute(
                "UPDATE transfers SET state = 'completed', failure_reason = NULL,
                    persisted_bytes = total_size, completed_at_ms = ?3, updated_at_ms = ?3
                 WHERE transfer_id = ?1 AND direction = ?2
                   AND state IN ('transferring', 'verifying')",
                params![transfer_id.to_string(), direction, now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::TransferNotFound);
        }
        if direction == "send" {
            transaction
                .execute(
                    "UPDATE message_deliveries SET state = 'completed', updated_at_ms = ?3
                     WHERE message_id = ?1 AND recipient_device_id = ?2",
                    params![
                        transfer.message_id.to_string(),
                        transfer.peer_device_id.to_string(),
                        now,
                    ],
                )
                .map_err(StorageError::Write)?;
            update_message_delivery_aggregate(&transaction, transfer.message_id)?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn get_transfer(
        &self,
        transfer_id: TransferId,
    ) -> Result<(TransferRecord, Vec<TransferEntryRecord>), StorageError> {
        let transfer_raw = self
            .connection
            .query_row(
                "SELECT t.transfer_id, t.message_id, m.conversation_id,
                        t.peer_device_id, t.direction, t.state,
                        t.failure_reason, t.display_name, t.total_size, t.entry_count,
                        t.persisted_bytes, t.receive_base_ref,
                        CASE WHEN t.state = 'completed' THEN
                            CASE WHEN m.kind = 'folder'
                                THEN (SELECT CASE WHEN t.direction = 'send'
                                    THEN e.source_ref ELSE e.destination_ref END
                                    FROM transfer_entries e
                                    WHERE e.transfer_id = t.transfer_id
                                      AND e.entry_kind = 'directory'
                                      AND e.relative_path = t.display_name
                                    ORDER BY e.rowid LIMIT 1)
                                ELSE (SELECT CASE WHEN t.direction = 'send'
                                    THEN e.source_ref ELSE e.destination_ref END
                                    FROM transfer_entries e
                                    WHERE e.transfer_id = t.transfer_id
                                      AND e.entry_kind = 'file'
                                    ORDER BY e.relative_path LIMIT 1)
                            END
                        END,
                        t.paused_by_user
                 FROM transfers t
                 JOIN messages m ON m.message_id = t.message_id
                 WHERE t.transfer_id = ?1",
                [transfer_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, i64>(13)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::TransferNotFound)?;
        let transfer = TransferRecord {
            transfer_id: transfer_raw
                .0
                .parse()
                .map_err(|_| StorageError::InvalidStoredId)?,
            message_id: transfer_raw
                .1
                .parse()
                .map_err(|_| StorageError::InvalidStoredId)?,
            conversation_id: transfer_raw.2,
            peer_device_id: transfer_raw
                .3
                .parse()
                .map_err(|_| StorageError::InvalidStoredId)?,
            direction: transfer_raw.4,
            state: transfer_raw.5,
            failure_reason: transfer_raw.6,
            display_name: transfer_raw.7,
            total_size: from_sql_u64(transfer_raw.8)?,
            entry_count: u32::try_from(transfer_raw.9)
                .map_err(|_| StorageError::InvalidStoredId)?,
            persisted_bytes: from_sql_u64(transfer_raw.10)?,
            receive_base_ref: transfer_raw.11,
            local_file_ref: transfer_raw.12,
            paused_by_user: transfer_raw.13 != 0,
        };
        let mut statement = self
            .connection
            .prepare(
                "SELECT entry_id, transfer_id, entry_kind, relative_path, size,
                        modified_at_ms, source_ref, destination_ref, partial_ref,
                        persisted_offset, state
                 FROM transfer_entries WHERE transfer_id = ?1 ORDER BY rowid",
            )
            .map_err(StorageError::Write)?;
        let entries = statement
            .query_map([transfer_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })
            .map_err(StorageError::Write)?
            .map(|row| {
                row.map_err(StorageError::Write)
                    .and_then(parse_transfer_entry)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((transfer, entries))
    }

    pub fn list_transfers(&self) -> Result<Vec<TransferRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT transfer_id FROM transfers ORDER BY updated_at_ms DESC, created_at_ms DESC",
            )
            .map_err(StorageError::Write)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StorageError::Write)?
            .map(|row| {
                row.map_err(StorageError::Write)?
                    .parse()
                    .map_err(|_| StorageError::InvalidStoredId)
            })
            .collect::<Result<Vec<TransferId>, StorageError>>()?;
        ids.into_iter()
            .map(|transfer_id| self.get_transfer(transfer_id).map(|record| record.0))
            .collect()
    }

    pub fn clear_completed_transfers(
        &mut self,
        client_operation_id: ClientOperationId,
    ) -> Result<u64, StorageError> {
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "clear_completed_transfers",
            |transaction, _| {
                let deleted = transaction
                    .execute("DELETE FROM transfers WHERE state = 'completed'", [])
                    .map_err(StorageError::Write)?;
                u64::try_from(deleted).map_err(|_| StorageError::InvalidStoredId)
            },
        )
    }

    pub fn list_conversations(&self) -> Result<Vec<ConversationRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.conversation_id, c.title_cache, c.peer_device_id, c.group_id, c.kind,
                        COALESCE(m.text_content, m.display_name),
                        c.last_activity_at_ms, c.unread_count,
                        CASE WHEN c.kind = 'group' THEN (
                            SELECT COUNT(*) FROM group_members gm
                            WHERE gm.group_id = c.group_id AND gm.membership = 'joined'
                        ) ELSE 2 END
                 FROM conversations c
                 LEFT JOIN messages m ON m.message_id = c.last_message_id
                 WHERE c.deleted_at_ms IS NULL
                 ORDER BY c.last_activity_at_ms DESC",
            )
            .map_err(StorageError::Write)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(StorageError::Write)?;
        rows.map(|row| {
            let (
                conversation_id,
                title,
                peer,
                group,
                kind,
                preview,
                last_activity,
                unread,
                members,
            ) = row.map_err(StorageError::Write)?;
            Ok(ConversationRecord {
                conversation_id,
                title,
                peer_device_id: peer
                    .map(|value| value.parse().map_err(|_| StorageError::InvalidStoredId))
                    .transpose()?,
                group_id: group
                    .map(|value| value.parse().map_err(|_| StorageError::InvalidStoredId))
                    .transpose()?,
                is_group: kind == "group",
                member_count: u32::try_from(members).unwrap_or(u32::MAX),
                last_message_preview: preview,
                last_activity_at_ms: last_activity,
                unread_count: u32::try_from(unread).unwrap_or(u32::MAX),
            })
        })
        .collect()
    }

    pub fn mark_conversation_read(
        &mut self,
        client_operation_id: ClientOperationId,
        local_device_id: DeviceId,
        conversation_id: &str,
        through_sort_order: i64,
    ) -> Result<(), StorageError> {
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "mark_conversation_read",
            |transaction, _now| {
                let changed = transaction
                    .execute(
                        "UPDATE conversations SET unread_count = (
                            SELECT COUNT(*) FROM messages m
                            WHERE m.conversation_id = conversations.conversation_id
                              AND m.sender_device_id != ?2
                              AND m.local_sort_order > ?3
                              AND m.deleted_at_ms IS NULL
                         )
                         WHERE conversation_id = ?1 AND deleted_at_ms IS NULL",
                        params![
                            conversation_id,
                            local_device_id.to_string(),
                            through_sort_order
                        ],
                    )
                    .map_err(StorageError::Write)?;
                if changed == 0 {
                    return Err(StorageError::ConversationNotFound);
                }
                Ok(())
            },
        )
    }

    pub fn incomplete_transfer_ids_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<TransferId>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT t.transfer_id FROM transfers t
                 JOIN messages m ON m.message_id = t.message_id
                 WHERE m.conversation_id = ?1
                   AND t.state NOT IN ('completed', 'cancelled')",
            )
            .map_err(StorageError::Write)?;
        statement
            .query_map([conversation_id], |row| row.get::<_, String>(0))
            .map_err(StorageError::Write)?
            .map(|row| {
                row.map_err(StorageError::Write)?
                    .parse()
                    .map_err(|_| StorageError::InvalidStoredId)
            })
            .collect()
    }

    pub fn delete_conversation(
        &mut self,
        client_operation_id: ClientOperationId,
        conversation_id: &str,
    ) -> Result<DeleteConversationResult, StorageError> {
        let replay_check = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<DeleteConversationResult>(
            &replay_check,
            client_operation_id,
            "delete_conversation",
        )? {
            replay_check.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        replay_check.commit().map_err(StorageError::Write)?;

        let transfer_ids = self.incomplete_transfer_ids_for_conversation(conversation_id)?;
        let mut partial_statement = self
            .connection
            .prepare(
                "SELECT e.partial_ref FROM transfer_entries e
                 JOIN transfers t ON t.transfer_id = e.transfer_id
                 JOIN messages m ON m.message_id = t.message_id
                 WHERE m.conversation_id = ?1
                   AND t.direction = 'receive'
                   AND t.state NOT IN ('completed', 'cancelled')
                   AND e.partial_ref IS NOT NULL",
            )
            .map_err(StorageError::Write)?;
        let partial_refs = partial_statement
            .query_map([conversation_id], |row| row.get::<_, String>(0))
            .map_err(StorageError::Write)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::Write)?;
        drop(partial_statement);

        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<DeleteConversationResult>(
            &transaction,
            client_operation_id,
            "delete_conversation",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let exists = transaction
            .query_row(
                "SELECT 1 FROM conversations WHERE conversation_id = ?1",
                [conversation_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !exists {
            return Err(StorageError::ConversationNotFound);
        }
        transaction
            .execute(
                "DELETE FROM outbox WHERE
                    event_id IN (
                        SELECT d.last_event_id FROM message_deliveries d
                        JOIN messages m ON m.message_id = d.message_id
                        WHERE m.conversation_id = ?1 AND d.last_event_id IS NOT NULL
                    ) OR (
                        json_valid(payload_json) AND (
                            json_extract(payload_json, '$.body.conversation_id') = ?1 OR
                            json_extract(payload_json, '$.body.transfer_id') IN (
                                SELECT t.transfer_id FROM transfers t
                                JOIN messages m ON m.message_id = t.message_id
                                WHERE m.conversation_id = ?1
                            )
                        )
                    )",
                [conversation_id],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'cancelled',
                    failure_reason = 'user_cancelled', updated_at_ms = ?2
                 WHERE message_id IN (
                    SELECT t.message_id FROM transfers t
                    JOIN messages m ON m.message_id = t.message_id
                    WHERE m.conversation_id = ?1
                      AND t.state NOT IN ('completed', 'cancelled')
                 ) AND state NOT IN ('completed', 'cancelled')",
                params![conversation_id, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET state = 'cancelled'
                 WHERE conversation_id = ?1 AND message_id IN (
                    SELECT message_id FROM transfers
                    WHERE state NOT IN ('completed', 'cancelled')
                 )",
                [conversation_id],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfer_entries SET state = 'cancelled', updated_at_ms = ?2
                 WHERE transfer_id IN (
                    SELECT t.transfer_id FROM transfers t
                    JOIN messages m ON m.message_id = t.message_id
                    WHERE m.conversation_id = ?1
                      AND t.state NOT IN ('completed', 'cancelled')
                 ) AND state != 'completed'",
                params![conversation_id, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE transfers SET state = 'cancelled', paused_by_user = 0,
                    failure_reason = 'user_cancelled', updated_at_ms = ?2
                 WHERE message_id IN (
                    SELECT message_id FROM messages WHERE conversation_id = ?1
                 ) AND state NOT IN ('completed', 'cancelled')",
                params![conversation_id, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE messages SET deleted_at_ms = ?2 WHERE conversation_id = ?1",
                params![conversation_id, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations SET deleted_at_ms = ?2,
                    last_message_id = NULL, unread_count = 0
                 WHERE conversation_id = ?1",
                params![conversation_id, now],
            )
            .map_err(StorageError::Write)?;
        let result = DeleteConversationResult {
            cancelled_transfer_ids: transfer_ids,
        };
        save_client_operation(
            &transaction,
            client_operation_id,
            "delete_conversation",
            &result,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        for partial in partial_refs {
            if !partial.starts_with("content://") {
                match fs::remove_file(partial) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => {}
                }
            }
        }
        Ok(result)
    }

    pub fn list_messages(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<MessageRecord>, StorageError> {
        let limit = limit.clamp(1, 100);
        let mut statement = self
            .connection
            .prepare(
                "SELECT m.message_id, m.conversation_id, m.sender_device_id, m.kind,
                        m.state, COALESCE(m.text_content, m.display_name, ''),
                        m.total_size, m.entry_count, m.created_at_ms, m.local_sort_order,
                        t.transfer_id, t.state, t.persisted_bytes,
                        CASE WHEN t.state = 'completed' THEN
                            CASE WHEN m.kind = 'folder'
                                THEN (SELECT CASE WHEN t.direction = 'send'
                                    THEN e.source_ref ELSE e.destination_ref END
                                    FROM transfer_entries e
                                    WHERE e.transfer_id = t.transfer_id
                                      AND e.entry_kind = 'directory'
                                      AND e.relative_path = t.display_name
                                    ORDER BY e.rowid LIMIT 1)
                                ELSE (SELECT CASE WHEN t.direction = 'send'
                                    THEN e.source_ref ELSE e.destination_ref END
                                    FROM transfer_entries e
                                    WHERE e.transfer_id = t.transfer_id
                                      AND e.entry_kind = 'file'
                                    ORDER BY e.relative_path LIMIT 1)
                            END
                        END,
                        (SELECT COUNT(*) FROM message_deliveries d
                         WHERE d.message_id = m.message_id),
                        (SELECT COUNT(*) FROM message_deliveries d
                         WHERE d.message_id = m.message_id AND (
                            (m.kind IN ('file', 'image', 'folder', 'clipboard_image')
                                AND d.state = 'completed') OR
                            (m.kind NOT IN ('file', 'image', 'folder', 'clipboard_image')
                                AND d.state IN ('stored', 'accepted', 'completed'))
                         ))
                 FROM messages m
                 LEFT JOIN transfers t ON t.transfer_id = (
                    SELECT t2.transfer_id FROM transfers t2
                    WHERE t2.message_id = m.message_id
                    ORDER BY CASE t2.state
                        WHEN 'transferring' THEN 0 WHEN 'accepted' THEN 1
                        WHEN 'offered' THEN 2 WHEN 'queued' THEN 3
                        WHEN 'paused' THEN 4 WHEN 'failed' THEN 5
                        WHEN 'cancelled' THEN 6 ELSE 7 END,
                        t2.updated_at_ms DESC LIMIT 1
                 )
                 WHERE m.conversation_id = ?1 AND m.deleted_at_ms IS NULL
                 ORDER BY m.local_sort_order DESC LIMIT ?2",
            )
            .map_err(StorageError::Write)?;
        let rows = statement
            .query_map(params![conversation_id, limit], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, i64>(14)?,
                    row.get::<_, i64>(15)?,
                ))
            })
            .map_err(StorageError::Write)?;
        let mut messages = rows
            .map(|row| {
                let (
                    message,
                    conversation,
                    sender,
                    kind,
                    state,
                    text,
                    total_size,
                    entry_count,
                    created,
                    sort,
                    transfer_id,
                    transfer_state,
                    persisted_bytes,
                    local_file_ref,
                    delivery_count,
                    delivered_count,
                ) = row.map_err(StorageError::Write)?;
                Ok(MessageRecord {
                    message_id: message.parse().map_err(|_| StorageError::InvalidStoredId)?,
                    conversation_id: conversation,
                    sender_device_id: sender.parse().map_err(|_| StorageError::InvalidStoredId)?,
                    kind,
                    state,
                    text,
                    total_size: total_size.map(from_sql_u64).transpose()?,
                    entry_count: entry_count
                        .map(|value| {
                            u32::try_from(value).map_err(|_| StorageError::InvalidStoredId)
                        })
                        .transpose()?,
                    transfer_id: transfer_id
                        .map(|value| value.parse().map_err(|_| StorageError::InvalidStoredId))
                        .transpose()?,
                    transfer_state,
                    persisted_bytes: persisted_bytes.map(from_sql_u64).transpose()?,
                    local_file_ref,
                    created_at_ms: created,
                    local_sort_order: sort,
                    delivered_count: u32::try_from(delivered_count).unwrap_or(u32::MAX),
                    delivery_count: u32::try_from(delivery_count).unwrap_or(u32::MAX),
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        messages.reverse();
        Ok(messages)
    }

    pub fn list_message_deliveries(
        &self,
        message_id: MessageId,
    ) -> Result<Vec<MessageDeliveryRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT d.recipient_device_id,
                        COALESCE(p.device_name, d.recipient_device_id),
                        d.state, d.failure_reason, d.updated_at_ms,
                        CASE
                          WHEN m.kind IN ('file', 'image', 'folder', 'clipboard_image')
                            THEN d.state = 'completed'
                          ELSE d.state IN ('stored', 'accepted', 'completed')
                        END
                 FROM message_deliveries d
                 JOIN messages m ON m.message_id = d.message_id
                 LEFT JOIN peers p ON p.device_id = d.recipient_device_id
                 WHERE d.message_id = ?1 AND m.deleted_at_ms IS NULL
                 ORDER BY COALESCE(p.device_name, d.recipient_device_id)",
            )
            .map_err(StorageError::Write)?;
        statement
            .query_map([message_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, bool>(5)?,
                ))
            })
            .map_err(StorageError::Write)?
            .map(|row| {
                let (recipient, name, state, failure_reason, updated_at_ms, delivered) =
                    row.map_err(StorageError::Write)?;
                Ok(MessageDeliveryRecord {
                    recipient_device_id: recipient
                        .parse()
                        .map_err(|_| StorageError::InvalidStoredId)?,
                    recipient_name: name,
                    state,
                    failure_reason,
                    updated_at_ms,
                    delivered,
                })
            })
            .collect()
    }

    fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationRecord>, StorageError> {
        self.connection
            .query_row(
                "SELECT c.conversation_id, c.title_cache, c.peer_device_id, c.group_id, c.kind,
                        COALESCE(m.text_content, m.display_name),
                        c.last_activity_at_ms, c.unread_count,
                        CASE WHEN c.kind = 'group' THEN (
                            SELECT COUNT(*) FROM group_members gm
                            WHERE gm.group_id = c.group_id AND gm.membership = 'joined'
                        ) ELSE 2 END
                 FROM conversations c
                 LEFT JOIN messages m ON m.message_id = c.last_message_id
                 WHERE c.conversation_id = ?1 AND c.deleted_at_ms IS NULL",
                [conversation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .map(
                |(id, title, peer, group, kind, preview, activity, unread, members)| {
                    Ok(ConversationRecord {
                        conversation_id: id,
                        title,
                        peer_device_id: peer
                            .map(|value| value.parse().map_err(|_| StorageError::InvalidStoredId))
                            .transpose()?,
                        group_id: group
                            .map(|value| value.parse().map_err(|_| StorageError::InvalidStoredId))
                            .transpose()?,
                        is_group: kind == "group",
                        member_count: u32::try_from(members).unwrap_or(u32::MAX),
                        last_message_preview: preview,
                        last_activity_at_ms: activity,
                        unread_count: u32::try_from(unread).unwrap_or(u32::MAX),
                    })
                },
            )
            .transpose()
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        let mut version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(StorageError::Migration)?;
        if version > SCHEMA_VERSION {
            return Err(StorageError::UnsupportedSchema(version));
        }
        if version == 0 {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StorageError::Migration)?;
            transaction
                .execute_batch(MIGRATION_V1)
                .map_err(StorageError::Migration)?;
            transaction
                .pragma_update(None, "user_version", 1)
                .map_err(StorageError::Migration)?;
            transaction.commit().map_err(StorageError::Migration)?;
            version = 1;
        }
        if version == 1 {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StorageError::Migration)?;
            transaction
                .execute_batch(MIGRATION_V2)
                .map_err(StorageError::Migration)?;
            transaction
                .pragma_update(None, "user_version", 2)
                .map_err(StorageError::Migration)?;
            transaction.commit().map_err(StorageError::Migration)?;
            version = 2;
        }
        if version == 2 {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StorageError::Migration)?;
            transaction
                .execute_batch(MIGRATION_V3)
                .map_err(StorageError::Migration)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(StorageError::Migration)?;
            transaction.commit().map_err(StorageError::Migration)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn connection(&self) -> &Connection {
        &self.connection
    }
}

type RawTransferEntry = (
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    String,
);

fn parse_transfer_entry(raw: RawTransferEntry) -> Result<TransferEntryRecord, StorageError> {
    Ok(TransferEntryRecord {
        entry_id: raw.0.parse().map_err(|_| StorageError::InvalidStoredId)?,
        transfer_id: raw.1.parse().map_err(|_| StorageError::InvalidStoredId)?,
        entry_kind: match raw.2.as_str() {
            "file" => TransferEntryKind::File,
            "directory" => TransferEntryKind::Directory,
            _ => return Err(StorageError::InvalidStoredId),
        },
        relative_path: raw.3,
        size: from_sql_u64(raw.4)?,
        modified_at_ms: raw.5,
        source_ref: raw.6,
        destination_ref: raw.7,
        partial_ref: raw.8,
        persisted_offset: from_sql_u64(raw.9)?,
        state: raw.10,
    })
}

fn insert_transfer_entries(
    transaction: &rusqlite::Transaction<'_>,
    transfer_id: TransferId,
    entries: &[ManifestEntry],
    include_source_ref: bool,
    now: i64,
) -> Result<(), StorageError> {
    for entry in entries {
        transaction
            .execute(
                "INSERT INTO transfer_entries(
                    entry_id, transfer_id, entry_kind, relative_path, size,
                    modified_at_ms, source_ref, persisted_offset, state, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'queued', ?8)",
                params![
                    entry.entry_id.to_string(),
                    transfer_id.to_string(),
                    transfer_entry_kind_name(entry.entry_kind),
                    entry.relative_path,
                    to_sql_u64(entry.size)?,
                    entry.modified_at_ms,
                    include_source_ref
                        .then(|| entry.source_ref.clone())
                        .flatten(),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
    }
    Ok(())
}

fn validate_clipboard_image_offer_metadata(
    message_kind: MessageKind,
    fingerprint: Option<&str>,
    automatic: bool,
) -> Result<(), StorageError> {
    let is_clipboard_image = message_kind == MessageKind::ClipboardImage;
    if is_clipboard_image != fingerprint.is_some() || (automatic && !is_clipboard_image) {
        return Err(StorageError::InvalidClipboardImageMetadata);
    }
    if let Some(fingerprint) = fingerprint
        && (fingerprint.len() != 64
            || !fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        return Err(StorageError::InvalidClipboardImageMetadata);
    }
    Ok(())
}

fn require_clipboard_send_binding(
    transaction: &rusqlite::Transaction<'_>,
    peer_device_id: DeviceId,
) -> Result<(), StorageError> {
    let mode = transaction
        .query_row(
            "SELECT clipboard_mode FROM own_device_bindings
             WHERE peer_device_id = ?1 AND state = 'active'",
            [peer_device_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(StorageError::Write)?;
    if !matches!(mode.as_deref(), Some("send_only" | "bidirectional")) {
        return Err(StorageError::ClipboardBindingRequired);
    }
    Ok(())
}

fn allocate_clipboard_image_metadata(
    transaction: &rusqlite::Transaction<'_>,
    origin_device_id: DeviceId,
    fingerprint: &str,
    automatic: bool,
    now: i64,
) -> Result<ClipboardImageMetadata, StorageError> {
    let previous = transaction
        .query_row(
            "SELECT clipboard_sequence FROM local_profile WHERE singleton_id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StorageError::Write)?;
    let next = previous
        .checked_add(1)
        .ok_or(StorageError::ClipboardSequenceExhausted)?;
    let clipboard_sequence =
        u64::try_from(next).map_err(|_| StorageError::ClipboardSequenceExhausted)?;
    transaction
        .execute(
            "UPDATE local_profile SET clipboard_sequence = ?1, updated_at_ms = ?2
             WHERE singleton_id = 1",
            params![next, now],
        )
        .map_err(StorageError::Write)?;
    Ok(ClipboardImageMetadata {
        origin_device_id,
        clipboard_sequence,
        content_fingerprint: fingerprint.to_owned(),
        automatic,
    })
}

fn insert_clipboard_image_dedup(
    transaction: &rusqlite::Transaction<'_>,
    message_id: MessageId,
    metadata: &ClipboardImageMetadata,
    now: i64,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO clipboard_dedup(origin_device_id, clipboard_sequence, message_id,
                content_fingerprint, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                metadata.origin_device_id.to_string(),
                i64::try_from(metadata.clipboard_sequence)
                    .map_err(|_| StorageError::ClipboardSequenceExhausted)?,
                message_id.to_string(),
                metadata.content_fingerprint,
                now,
            ],
        )
        .map_err(StorageError::Write)?;
    transaction
        .execute(
            "DELETE FROM clipboard_dedup
             WHERE created_at_ms < ?1
                AND rowid NOT IN (
                    SELECT rowid FROM clipboard_dedup
                    ORDER BY created_at_ms DESC LIMIT 10000
                )",
            [now.saturating_sub(30 * 24 * 60 * 60 * 1000)],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

pub(super) fn run_client_operation<T, F>(
    connection: &mut Connection,
    client_operation_id: ClientOperationId,
    operation_type: &str,
    action: F,
) -> Result<T, StorageError>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce(&Transaction<'_>, i64) -> Result<T, StorageError>,
{
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StorageError::Write)?;
    if let Some(saved) = saved_client_operation(&transaction, client_operation_id, operation_type)?
    {
        transaction.commit().map_err(StorageError::Write)?;
        return Ok(saved);
    }
    let now = unix_time_ms();
    let result = action(&transaction, now)?;
    save_client_operation(
        &transaction,
        client_operation_id,
        operation_type,
        &result,
        now,
    )?;
    transaction.commit().map_err(StorageError::Write)?;
    Ok(result)
}

fn replay_client_operation<T: DeserializeOwned>(
    connection: &mut Connection,
    client_operation_id: ClientOperationId,
    operation_type: &str,
) -> Result<Option<T>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StorageError::Write)?;
    let saved = saved_client_operation(&transaction, client_operation_id, operation_type)?;
    transaction.commit().map_err(StorageError::Write)?;
    Ok(saved)
}

pub(super) fn saved_client_operation<T: DeserializeOwned>(
    transaction: &Transaction<'_>,
    client_operation_id: ClientOperationId,
    operation_type: &str,
) -> Result<Option<T>, StorageError> {
    let saved = transaction
        .query_row(
            "SELECT result_json FROM client_operations
             WHERE client_operation_id = ?1 AND operation_type = ?2",
            params![client_operation_id.to_string(), operation_type],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(StorageError::Write)?
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                StorageError::Serialization(format!(
                    "invalid saved client operation result: {error}"
                ))
            })
        })
        .transpose()?;
    if saved.is_some() {
        eprintln!(
            "replayed client operation: id={client_operation_id}, type={operation_type}; returning the first result"
        );
    }
    Ok(saved)
}

pub(super) fn save_client_operation<T: Serialize>(
    transaction: &Transaction<'_>,
    client_operation_id: ClientOperationId,
    operation_type: &str,
    result: &T,
    now: i64,
) -> Result<(), StorageError> {
    let result_json = serde_json::to_string(result)
        .map_err(|error| StorageError::Serialization(error.to_string()))?;
    transaction
        .execute(
            "INSERT INTO client_operations(
                client_operation_id, operation_type, result_json, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                client_operation_id.to_string(),
                operation_type,
                result_json,
                now
            ],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

fn saved_receipt(
    transaction: &rusqlite::Transaction<'_>,
    sender_device_id: DeviceId,
    event_id: EventId,
) -> Result<Option<String>, StorageError> {
    transaction
        .query_row(
            "SELECT receipt_json FROM processed_events
             WHERE sender_device_id = ?1 AND event_id = ?2",
            params![sender_device_id.to_string(), event_id.to_string()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(StorageError::Write)
        .map(Option::flatten)
}

fn insert_processed_event(
    transaction: &rusqlite::Transaction<'_>,
    sender_device_id: DeviceId,
    event_id: EventId,
    event_type: &str,
    receipt_json: &str,
    now: i64,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO processed_events(
                sender_device_id, event_id, event_type, receipt_json, processed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                sender_device_id.to_string(),
                event_id.to_string(),
                event_type,
                receipt_json,
                now,
            ],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

fn insert_outbox_event(
    transaction: &rusqlite::Transaction<'_>,
    peer_device_id: DeviceId,
    event_id: EventId,
    event_type: &str,
    payload_json: &str,
    now: i64,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "INSERT INTO outbox(
                peer_device_id, event_id, event_type, payload_json,
                requires_receipt, state, attempts, next_attempt_at_ms,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, 1, 'pending', 0, ?5, ?5, ?5)",
            params![
                peer_device_id.to_string(),
                event_id.to_string(),
                event_type,
                payload_json,
                now,
            ],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

fn update_message_delivery_aggregate(
    transaction: &rusqlite::Transaction<'_>,
    message_id: MessageId,
) -> Result<(), StorageError> {
    let kind = transaction
        .query_row(
            "SELECT kind FROM messages WHERE message_id = ?1",
            [message_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .map_err(StorageError::Write)?;
    let file_message = matches!(
        kind.as_str(),
        "file" | "image" | "folder" | "clipboard_image"
    );
    let (total, stored, completed, waiting, failed) = transaction
        .query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN state IN ('stored', 'accepted', 'completed') THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state = 'completed' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state IN ('queued', 'sending') THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state IN ('rejected', 'failed', 'cancelled') THEN 1 ELSE 0 END)
             FROM message_deliveries WHERE message_id = ?1",
            [message_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(StorageError::Write)?;
    let delivered = if file_message { completed } else { stored };
    let state = if total == 0 || delivered == total {
        "delivered"
    } else if delivered > 0 {
        "partially_delivered"
    } else if waiting > 0 || (file_message && stored > 0) {
        "queued"
    } else if failed == total {
        "failed"
    } else {
        "sending"
    };
    transaction
        .execute(
            "UPDATE messages SET state = ?2 WHERE message_id = ?1",
            params![message_id.to_string(), state],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

fn transfer_control_payload(
    profile: &LocalProfile,
    event_id: EventId,
    event_type: &str,
    body: serde_json::Value,
    now: i64,
) -> String {
    serde_json::json!({
        "version": 1,
        "type": event_type,
        "event_id": event_id,
        "sender_device_id": profile.device_id,
        "sent_at_ms": now,
        "body": body,
    })
    .to_string()
}

fn file_offsets(entries: &[TransferEntryRecord]) -> Vec<AcceptedOffset> {
    entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
        .map(|entry| AcceptedOffset {
            entry_id: entry.entry_id,
            offset: entry.persisted_offset,
        })
        .collect()
}

fn validate_offsets(
    entries: &[TransferEntryRecord],
    offsets: &[AcceptedOffset],
) -> Result<(), StorageError> {
    let offset_by_id = offsets
        .iter()
        .map(|offset| (offset.entry_id, offset.offset))
        .collect::<HashMap<_, _>>();
    let files = entries
        .iter()
        .filter(|entry| entry.entry_kind == TransferEntryKind::File)
        .collect::<Vec<_>>();
    if offset_by_id.len() != offsets.len() || offset_by_id.len() != files.len() {
        return Err(StorageError::TransferNotFound);
    }
    if files.iter().any(|entry| {
        offset_by_id
            .get(&entry.entry_id)
            .is_none_or(|offset| *offset > entry.size)
    }) {
        return Err(StorageError::TransferNotFound);
    }
    Ok(())
}

fn mark_transfer_cancelled(
    transaction: &rusqlite::Transaction<'_>,
    transfer: &TransferRecord,
    now: i64,
) -> Result<(), StorageError> {
    transaction
        .execute(
            "UPDATE transfers SET state = 'cancelled', paused_by_user = 0,
                failure_reason = 'user_cancelled', updated_at_ms = ?2
             WHERE transfer_id = ?1 AND state != 'completed'",
            params![transfer.transfer_id.to_string(), now],
        )
        .map_err(StorageError::Write)?;
    transaction
        .execute(
            "UPDATE transfer_entries SET state = 'cancelled', updated_at_ms = ?2
             WHERE transfer_id = ?1 AND state != 'completed'",
            params![transfer.transfer_id.to_string(), now],
        )
        .map_err(StorageError::Write)?;
    transaction
        .execute(
            "UPDATE messages SET state = 'cancelled' WHERE message_id = ?1",
            [transfer.message_id.to_string()],
        )
        .map_err(StorageError::Write)?;
    if transfer.direction == "send" {
        transaction
            .execute(
                "UPDATE message_deliveries SET state = 'cancelled',
                    failure_reason = 'user_cancelled', updated_at_ms = ?3
                 WHERE message_id = ?1 AND recipient_device_id = ?2",
                params![
                    transfer.message_id.to_string(),
                    transfer.peer_device_id.to_string(),
                    now,
                ],
            )
            .map_err(StorageError::Write)?;
    }
    Ok(())
}

fn remove_partial_files(entries: &[TransferEntryRecord]) {
    for partial in entries
        .iter()
        .filter_map(|entry| entry.partial_ref.as_deref())
    {
        match fs::remove_file(partial) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

fn to_sql_u64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::Manifest(ManifestError::SizeOverflow))
}

fn from_sql_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidStoredId)
}

fn transfer_entry_kind_name(kind: TransferEntryKind) -> &'static str {
    match kind {
        TransferEntryKind::File => "file",
        TransferEntryKind::Directory => "directory",
    }
}

fn message_kind_name(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Text => "text",
        MessageKind::File => "file",
        MessageKind::Image => "image",
        MessageKind::Folder => "folder",
        MessageKind::ClipboardText => "clipboard_text",
        MessageKind::ClipboardImage => "clipboard_image",
        MessageKind::System => "system",
    }
}

fn next_sort_order(
    transaction: &rusqlite::Transaction<'_>,
    conversation_id: &str,
) -> Result<i64, StorageError> {
    transaction
        .query_row(
            "SELECT COALESCE(MAX(local_sort_order), 0) + 1 FROM messages
             WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get(0),
        )
        .map_err(StorageError::Write)
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "windows",
        Platform::Android => "android",
    }
}

fn validate_device_name(device_name: &str) -> Result<(), StorageError> {
    let characters = device_name.chars().count();
    if !(1..=32).contains(&characters) || device_name.len() > 128 {
        return Err(StorageError::InvalidDeviceName);
    }
    Ok(())
}

fn valid_receive_policy(value: &str) -> bool {
    matches!(value, "auto_accept" | "ask_every_time")
}

fn validate_text(text: &str) -> Result<(), StorageError> {
    if !(1..=20_000).contains(&text.chars().count()) {
        return Err(StorageError::InvalidText);
    }
    Ok(())
}

fn validate_text_message(message_kind: &str, text: &str) -> Result<(), StorageError> {
    match message_kind {
        "text" => validate_text(text),
        "clipboard_text" if !text.is_empty() && text.len() <= MAX_CLIPBOARD_TEXT_BYTES => Ok(()),
        "clipboard_text" => Err(StorageError::ClipboardTooLarge),
        _ => Err(StorageError::InvalidText),
    }
}

fn unix_time_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_repeatable_and_enables_required_pragmas() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lan_chat.db");
        let first = Storage::open(&path).unwrap();
        assert_eq!(
            first
                .connection()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        assert_eq!(
            first
                .connection()
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            first
                .connection()
                .query_row(
                    "SELECT count(*) FROM sqlite_schema
                     WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            17
        );
        drop(first);
        Storage::open(path).unwrap();
    }

    #[test]
    fn existing_v1_database_migrates_to_the_larger_manifest_limit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lan_chat.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(MIGRATION_V1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);

        let migrated = Storage::open(&path).unwrap();
        assert_eq!(
            migrated
                .connection()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        let transfers_sql = migrated
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'transfers'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert!(transfers_sql.contains("BETWEEN 1 AND 20001"));
        assert_eq!(
            migrated
                .connection()
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                    [],
                    |row| { row.get::<_, String>(0) }
                )
                .unwrap(),
            SCHEMA_VERSION.to_string()
        );
        assert_eq!(
            migrated
                .connection()
                .query_row(
                    "SELECT dflt_value FROM pragma_table_info('app_settings')
                     WHERE name = 'auto_open_receive_directory'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "0"
        );
    }

    #[test]
    fn device_id_is_stable_across_restarts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lan_chat.db");
        let first_id = Storage::open(&path)
            .unwrap()
            .load_or_create_profile("书房电脑", Platform::Windows)
            .unwrap()
            .device_id;
        let second = Storage::open(&path)
            .unwrap()
            .load_or_create_profile("新名称", Platform::Windows)
            .unwrap();
        assert_eq!(second.device_id, first_id);
        assert_eq!(second.device_name, "书房电脑");
    }

    #[test]
    fn settings_and_peer_receive_overrides_are_persisted_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("lan_chat.db")).unwrap();
        storage
            .load_or_create_profile("书房电脑", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([9; 16]);
        storage
            .upsert_nearby_peer(peer, "手机", Platform::Android, "192.168.1.9", 1)
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE peers SET relation = 'known' WHERE device_id = ?1",
                [peer.to_string()],
            )
            .unwrap();

        let updated = storage
            .update_app_settings(
                ClientOperationId::generate(),
                Some("ask_every_time"),
                Some("D:/Receive"),
                false,
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
                Some("debug"),
            )
            .unwrap();
        assert_eq!(updated.default_receive_policy, "ask_every_time");
        assert_eq!(updated.default_receive_ref.as_deref(), Some("D:/Receive"));
        assert!(!updated.notifications_enabled);
        assert!(!updated.close_to_tray);
        assert!(updated.start_on_boot);
        assert!(!updated.android_keep_online);
        assert!(updated.auto_open_receive_directory);
        assert_eq!(updated.log_level, "debug");

        storage
            .set_peer_receive_policy(ClientOperationId::generate(), peer, Some("auto_accept"))
            .unwrap();
        let peer_policy = storage.list_peer_receive_policies().unwrap().remove(0);
        assert_eq!(
            peer_policy.receive_policy_override.as_deref(),
            Some("auto_accept")
        );
        assert_eq!(peer_policy.effective_receive_policy, "auto_accept");

        assert!(
            storage
                .update_app_settings(
                    ClientOperationId::generate(),
                    Some("invalid"),
                    None,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .is_err()
        );
        assert!(
            storage
                .set_peer_receive_policy(ClientOperationId::generate(), peer, Some("invalid"),)
                .is_err()
        );
        storage
            .update_app_settings(
                ClientOperationId::generate(),
                None,
                None,
                true,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(storage.app_settings().unwrap().default_receive_ref, None);
    }

    #[test]
    fn schema_checks_reject_invalid_values() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("lan_chat.db")).unwrap();
        let error = storage.connection().execute(
            "INSERT INTO peers(
                device_id, device_name, platform, relation,
                created_at_ms, updated_at_ms
             ) VALUES ('d_bad', '', 'ios', 'nearby', 1, 1)",
            [],
        );
        assert!(error.is_err());
    }

    #[test]
    fn invalid_device_names_are_rejected_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("lan_chat.db")).unwrap();
        assert!(
            storage
                .load_or_create_profile("", Platform::Windows)
                .is_err()
        );
        assert!(
            storage
                .load_or_create_profile(&"a".repeat(33), Platform::Windows)
                .is_err()
        );
    }

    #[test]
    fn nearby_peer_upsert_preserves_a_known_relation() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("lan_chat.db")).unwrap();
        let peer = DeviceId::from_bytes([7; 16]);
        storage
            .upsert_nearby_peer(peer, "手机", Platform::Android, "192.168.1.8", 10)
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE peers SET relation = 'known' WHERE device_id = ?1",
                [peer.to_string()],
            )
            .unwrap();
        storage
            .upsert_nearby_peer(peer, "手机 2", Platform::Android, "192.168.1.9", 20)
            .unwrap();
        let relation = storage
            .connection()
            .query_row(
                "SELECT relation FROM peers WHERE device_id = ?1",
                [peer.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(relation, "known");
    }

    #[test]
    fn private_text_outbox_receipt_and_receive_dedup_are_transactional() {
        let temp = tempfile::tempdir().unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
        let mut sender = Storage::open(&sender_path).unwrap();
        let sender_profile = sender
            .load_or_create_profile("电脑", Platform::Windows)
            .unwrap();
        let mut receiver = Storage::open(&receiver_path).unwrap();
        let receiver_profile = receiver
            .load_or_create_profile("手机", Platform::Android)
            .unwrap();
        sender
            .upsert_nearby_peer(
                receiver_profile.device_id,
                &receiver_profile.device_name,
                receiver_profile.platform,
                "192.168.1.2",
                1,
            )
            .unwrap();
        receiver
            .upsert_nearby_peer(
                sender_profile.device_id,
                &sender_profile.device_name,
                sender_profile.platform,
                "192.168.1.3",
                1,
            )
            .unwrap();
        let conversation = sender
            .open_private_conversation(
                ClientOperationId::generate(),
                sender_profile.device_id,
                receiver_profile.device_id,
            )
            .unwrap();
        let outgoing = sender
            .create_outgoing_text(&sender_profile, &conversation.conversation_id, "你好")
            .unwrap();
        let pending = sender.pending_outbox().unwrap();
        assert_eq!(pending.len(), 1);
        let envelope: serde_json::Value = serde_json::from_str(&pending[0].payload_json).unwrap();
        let event_id: EventId = envelope["event_id"].as_str().unwrap().parse().unwrap();
        let receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver_profile.device_id,
            "body": {
                "original_event_id": event_id,
                "message_id": outgoing.message_id,
                "stage": "stored"
            }
        })
        .to_string();
        receiver
            .receive_text(
                receiver_profile.device_id,
                sender_profile.device_id,
                event_id,
                outgoing.message_id,
                &conversation.conversation_id,
                "你好",
                outgoing.created_at_ms,
                None,
                &receipt,
            )
            .unwrap();
        receiver
            .receive_text(
                receiver_profile.device_id,
                sender_profile.device_id,
                event_id,
                outgoing.message_id,
                &conversation.conversation_id,
                "你好",
                outgoing.created_at_ms,
                None,
                &receipt,
            )
            .unwrap();
        assert_eq!(
            receiver
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()
                .len(),
            1
        );
        sender
            .mark_text_delivered(receiver_profile.device_id, event_id, outgoing.message_id)
            .unwrap();
        assert!(sender.pending_outbox().unwrap().is_empty());
        assert_eq!(
            sender
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()[0]
                .state,
            "delivered"
        );
    }

    #[test]
    fn conversation_read_delete_and_incoming_reopen_are_transactional() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("receiver.db")).unwrap();
        let local = storage
            .load_or_create_profile("电脑", Platform::Windows)
            .unwrap();
        let sender = DeviceId::from_bytes([11; 16]);
        storage
            .upsert_nearby_peer(sender, "手机", Platform::Android, "192.168.1.11", 1)
            .unwrap();
        let conversation_id = PrivateConversationId::new(local.device_id, sender)
            .unwrap()
            .to_string();
        storage
            .receive_text(
                local.device_id,
                sender,
                EventId::generate(),
                MessageId::generate(),
                &conversation_id,
                "第一条",
                1,
                None,
                "{\"receipt\":1}",
            )
            .unwrap();
        assert_eq!(storage.list_conversations().unwrap()[0].unread_count, 1);

        let read_operation = ClientOperationId::generate();
        storage
            .mark_conversation_read(read_operation, local.device_id, &conversation_id, 1)
            .unwrap();
        storage
            .mark_conversation_read(read_operation, local.device_id, &conversation_id, 0)
            .unwrap();
        assert_eq!(storage.list_conversations().unwrap()[0].unread_count, 0);
        let delete_operation = ClientOperationId::generate();
        let deleted = storage
            .delete_conversation(delete_operation, &conversation_id)
            .unwrap();
        assert_eq!(
            storage
                .delete_conversation(delete_operation, "different-conversation")
                .unwrap(),
            deleted
        );
        assert!(storage.list_conversations().unwrap().is_empty());
        assert!(
            storage
                .list_messages(&conversation_id, 100)
                .unwrap()
                .is_empty()
        );

        storage
            .receive_text(
                local.device_id,
                sender,
                EventId::generate(),
                MessageId::generate(),
                &conversation_id,
                "删除后新消息",
                2,
                None,
                "{\"receipt\":2}",
            )
            .unwrap();
        let reopened = storage.list_conversations().unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].unread_count, 1);
        assert_eq!(
            storage.list_messages(&conversation_id, 100).unwrap().len(),
            1
        );
    }

    #[test]
    fn file_offer_creates_complete_graph_and_duplicate_receive_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("photo.jpg");
        fs::write(&source_path, b"image-bytes").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::Image).unwrap();
        let mut sender = Storage::open(temp.path().join("sender.db")).unwrap();
        let sender_profile = sender
            .load_or_create_profile("电脑", Platform::Windows)
            .unwrap();
        let mut receiver = Storage::open(temp.path().join("receiver.db")).unwrap();
        let receiver_profile = receiver
            .load_or_create_profile("手机", Platform::Android)
            .unwrap();
        sender
            .upsert_nearby_peer(
                receiver_profile.device_id,
                "手机",
                Platform::Android,
                "192.168.1.2",
                1,
            )
            .unwrap();
        receiver
            .upsert_nearby_peer(
                sender_profile.device_id,
                "电脑",
                Platform::Windows,
                "192.168.1.3",
                1,
            )
            .unwrap();
        receiver
            .open_private_conversation(
                ClientOperationId::generate(),
                receiver_profile.device_id,
                sender_profile.device_id,
            )
            .unwrap();
        let conversation = sender
            .open_private_conversation(
                ClientOperationId::generate(),
                sender_profile.device_id,
                receiver_profile.device_id,
            )
            .unwrap();
        let outgoing = sender
            .create_outgoing_offer(
                &sender_profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        let pending = sender.pending_outbox().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_type, "file_offer");
        let (_, send_entries) = sender.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(send_entries.len(), 1);
        assert_eq!(
            send_entries[0].source_ref.as_deref(),
            source_path.canonicalize().unwrap().to_str()
        );

        let receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver_profile.device_id,
            "body": {
                "original_event_id": outgoing.event_id,
                "message_id": outgoing.message_id,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        let automatic_receive_dir = temp.path().join("automatic-receive");
        receiver
            .set_default_receive_ref(
                ClientOperationId::generate(),
                automatic_receive_dir.to_str(),
            )
            .unwrap();
        let first = receiver
            .receive_file_offer(
                receiver_profile.device_id,
                sender_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
                &conversation.conversation_id,
                123,
                None,
                &manifest,
                None,
                &receipt,
            )
            .unwrap();
        let duplicate = receiver
            .receive_file_offer(
                receiver_profile.device_id,
                sender_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
                &conversation.conversation_id,
                123,
                None,
                &manifest,
                None,
                &receipt,
            )
            .unwrap();
        assert!(!first.duplicate);
        assert_eq!(
            first.auto_receive_base_ref.as_deref(),
            automatic_receive_dir.to_str()
        );
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.receipt_json, receipt);
        let (received, receive_entries) = receiver.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(received.direction, "receive");
        assert_eq!(received.state, "offered");
        assert!(receive_entries[0].source_ref.is_none());
        for table in [
            "messages",
            "transfers",
            "transfer_entries",
            "processed_events",
        ] {
            let count = receiver
                .connection()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(count, 1, "unexpected rows in {table}");
        }

        sender
            .mark_file_offer_stored(
                receiver_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
            )
            .unwrap();
        let (_, receive_entries) = receiver.get_transfer(outgoing.transfer_id).unwrap();
        let receive_dir = temp.path().join("received");
        let prepared = crate::transfer::prepare_receive_paths(
            &receive_dir,
            outgoing.transfer_id,
            &manifest.display_name,
            &receive_entries,
        )
        .unwrap();
        let accept_operation = ClientOperationId::generate();
        let accept_event = receiver
            .accept_incoming_offer(
                &receiver_profile,
                accept_operation,
                outgoing.transfer_id,
                receive_dir.to_str().unwrap(),
                &prepared,
            )
            .unwrap();
        assert_eq!(
            receiver
                .accept_incoming_offer(
                    &receiver_profile,
                    accept_operation,
                    outgoing.transfer_id,
                    "ignored-on-replay",
                    &[],
                )
                .unwrap(),
            accept_event
        );
        let accept_outbox = receiver.pending_outbox().unwrap();
        assert_eq!(accept_outbox.len(), 1);
        assert_eq!(accept_outbox[0].event_type, "file_accept");
        let accept_value: serde_json::Value =
            serde_json::from_str(&accept_outbox[0].payload_json).unwrap();
        let offsets: Vec<AcceptedOffset> =
            serde_json::from_value(accept_value["body"]["entries"].clone()).unwrap();
        let accept_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": sender_profile.device_id,
            "body": {
                "original_event_id": accept_event,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        sender
            .receive_file_accept(
                receiver_profile.device_id,
                accept_event,
                outgoing.transfer_id,
                &offsets,
                &accept_receipt,
            )
            .unwrap();
        sender
            .receive_file_accept(
                receiver_profile.device_id,
                accept_event,
                outgoing.transfer_id,
                &offsets,
                &accept_receipt,
            )
            .unwrap();
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "accepted"
        );
        sender
            .mark_file_offer_stored(
                receiver_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
            )
            .unwrap();
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "accepted"
        );
        receiver
            .mark_control_event_stored(sender_profile.device_id, accept_event)
            .unwrap();
        assert!(receiver.pending_outbox().unwrap().is_empty());

        let partial = prepared[0].partial_ref.clone().unwrap();
        fs::write(&partial, b"12345678").unwrap();
        receiver
            .connection()
            .execute(
                "UPDATE transfer_entries SET persisted_offset = 5, state = 'transferring'
                 WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        receiver
            .connection()
            .execute(
                "UPDATE transfers SET state = 'transferring', persisted_bytes = 5
                 WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        receiver
            .reconcile_interrupted_transfers(&receiver_profile)
            .unwrap();
        assert_eq!(fs::metadata(&partial).unwrap().len(), 5);
        assert_eq!(
            receiver.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "queued"
        );
        let startup_resume = receiver.pending_outbox().unwrap().remove(0);
        assert_eq!(startup_resume.event_type, "transfer_resume");
        receiver
            .mark_control_event_stored(sender_profile.device_id, startup_resume.event_id)
            .unwrap();

        let pause_operation = ClientOperationId::generate();
        let pause_event = sender
            .pause_transfer(&sender_profile, pause_operation, outgoing.transfer_id)
            .unwrap();
        assert_eq!(
            sender
                .pause_transfer(&sender_profile, pause_operation, outgoing.transfer_id)
                .unwrap(),
            pause_event
        );
        let late_accept_event = EventId::generate();
        let late_accept_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver_profile.device_id,
            "body": {
                "original_event_id": late_accept_event,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        sender
            .receive_file_accept(
                receiver_profile.device_id,
                late_accept_event,
                outgoing.transfer_id,
                &offsets,
                &late_accept_receipt,
            )
            .unwrap();
        let (still_paused, _) = sender.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(still_paused.state, "paused");
        assert!(still_paused.paused_by_user);
        sender
            .connection()
            .execute(
                "UPDATE transfers SET state = 'accepted' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        let pause_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver_profile.device_id,
            "body": {
                "original_event_id": pause_event,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        receiver
            .receive_transfer_pause(
                sender_profile.device_id,
                pause_event,
                outgoing.transfer_id,
                &pause_receipt,
            )
            .unwrap();
        assert_eq!(
            receiver.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "paused"
        );
        sender
            .mark_control_event_stored(receiver_profile.device_id, pause_event)
            .unwrap();

        let resume_operation = ClientOperationId::generate();
        let resume_event = sender
            .resume_transfer(&sender_profile, resume_operation, outgoing.transfer_id)
            .unwrap();
        let (resumed, _) = sender.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(resumed.state, "accepted");
        assert!(!resumed.paused_by_user);
        assert_eq!(
            sender
                .resume_transfer(&sender_profile, resume_operation, outgoing.transfer_id)
                .unwrap(),
            resume_event
        );
        let resume_outbox = sender.pending_outbox().unwrap();
        let resume_value: serde_json::Value = serde_json::from_str(
            &resume_outbox
                .iter()
                .find(|entry| entry.event_id == resume_event)
                .unwrap()
                .payload_json,
        )
        .unwrap();
        let resume_offsets: Vec<AcceptedOffset> =
            serde_json::from_value(resume_value["body"]["entries"].clone()).unwrap();
        let resume_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": receiver_profile.device_id,
            "body": {
                "original_event_id": resume_event,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        let resume_result = receiver
            .receive_transfer_resume(
                &receiver_profile,
                sender_profile.device_id,
                resume_event,
                outgoing.transfer_id,
                &resume_offsets,
                &resume_receipt,
            )
            .unwrap();
        assert!(resume_result.queued_resume_reply);
        sender
            .mark_control_event_stored(receiver_profile.device_id, resume_event)
            .unwrap();
        let reply = receiver
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "transfer_resume")
            .unwrap();
        let reply_value: serde_json::Value = serde_json::from_str(&reply.payload_json).unwrap();
        let reply_offsets: Vec<AcceptedOffset> =
            serde_json::from_value(reply_value["body"]["entries"].clone()).unwrap();
        let reply_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": sender_profile.device_id,
            "body": {
                "original_event_id": reply.event_id,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        sender
            .receive_transfer_resume(
                &sender_profile,
                receiver_profile.device_id,
                reply.event_id,
                outgoing.transfer_id,
                &reply_offsets,
                &reply_receipt,
            )
            .unwrap();
        receiver
            .mark_control_event_stored(sender_profile.device_id, reply.event_id)
            .unwrap();

        assert!(Path::new(&partial).exists());
        let cancel_operation = ClientOperationId::generate();
        let cancel_event = receiver
            .cancel_transfer(&receiver_profile, cancel_operation, outgoing.transfer_id)
            .unwrap();
        assert_eq!(
            receiver
                .cancel_transfer(&receiver_profile, cancel_operation, outgoing.transfer_id,)
                .unwrap(),
            cancel_event
        );
        assert!(!Path::new(&partial).exists());
        let cancel_receipt = serde_json::json!({
            "version": 1,
            "type": "delivery_receipt",
            "event_id": EventId::generate(),
            "sender_device_id": sender_profile.device_id,
            "body": {
                "original_event_id": cancel_event,
                "transfer_id": outgoing.transfer_id,
                "stage": "stored",
            }
        })
        .to_string();
        sender
            .receive_transfer_cancel(
                receiver_profile.device_id,
                cancel_event,
                outgoing.transfer_id,
                &cancel_receipt,
            )
            .unwrap();
        sender
            .receive_transfer_cancel(
                receiver_profile.device_id,
                cancel_event,
                outgoing.transfer_id,
                &cancel_receipt,
            )
            .unwrap();
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "cancelled"
        );
    }

    #[test]
    fn completed_image_message_exposes_only_its_local_file_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("preview.png");
        fs::write(&source_path, b"preview-image").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::Image).unwrap();
        let mut storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let profile = storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([91; 16]);
        storage
            .upsert_nearby_peer(peer, "Receiver", Platform::Android, "192.168.1.9", 1)
            .unwrap();
        let conversation = storage
            .open_private_conversation(ClientOperationId::generate(), profile.device_id, peer)
            .unwrap();
        let outgoing = storage
            .create_outgoing_offer(
                &profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();

        let queued = storage
            .list_messages(&conversation.conversation_id, 10)
            .unwrap();
        assert_eq!(queued[0].local_file_ref, None);

        storage
            .connection()
            .execute(
                "UPDATE transfers SET state = 'transferring' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        storage
            .complete_send_transfer(outgoing.transfer_id)
            .unwrap();

        let completed = storage
            .list_messages(&conversation.conversation_id, 10)
            .unwrap();
        assert_eq!(
            completed[0].local_file_ref.as_deref(),
            source_path.canonicalize().unwrap().to_str()
        );
    }

    #[test]
    fn completed_folder_exposes_its_root_instead_of_a_child_file() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("album");
        fs::create_dir(&source_path).unwrap();
        fs::write(source_path.join("first.txt"), b"first").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::Folder).unwrap();
        let expected_root = source_path.canonicalize().unwrap();
        assert_eq!(
            manifest.entries[0].source_ref.as_deref(),
            expected_root.to_str()
        );

        let mut storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let profile = storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([93; 16]);
        storage
            .upsert_nearby_peer(peer, "Receiver", Platform::Android, "192.168.1.11", 1)
            .unwrap();
        let conversation = storage
            .open_private_conversation(ClientOperationId::generate(), profile.device_id, peer)
            .unwrap();
        let outgoing = storage
            .create_outgoing_offer(
                &profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE transfers SET state = 'transferring' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        storage
            .complete_send_transfer(outgoing.transfer_id)
            .unwrap();

        let transfer = storage.get_transfer(outgoing.transfer_id).unwrap().0;
        assert_eq!(transfer.local_file_ref.as_deref(), expected_root.to_str());
        let message = storage
            .list_messages(&conversation.conversation_id, 10)
            .unwrap()
            .remove(0);
        assert_eq!(message.local_file_ref.as_deref(), expected_root.to_str());
    }

    #[test]
    fn clearing_completed_transfers_keeps_messages_and_local_files() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("keep-after-clear.txt");
        fs::write(&source_path, b"keep this file").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::File).unwrap();
        let mut storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let profile = storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([92; 16]);
        storage
            .upsert_nearby_peer(peer, "Receiver", Platform::Android, "192.168.1.10", 1)
            .unwrap();
        let conversation = storage
            .open_private_conversation(ClientOperationId::generate(), profile.device_id, peer)
            .unwrap();
        let outgoing = storage
            .create_outgoing_offer(
                &profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE transfers SET state = 'transferring' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        storage
            .complete_send_transfer(outgoing.transfer_id)
            .unwrap();

        let operation = ClientOperationId::generate();
        assert_eq!(storage.clear_completed_transfers(operation).unwrap(), 1);
        assert_eq!(storage.clear_completed_transfers(operation).unwrap(), 1);
        assert!(storage.list_transfers().unwrap().is_empty());
        assert!(source_path.exists());
        let messages = storage
            .list_messages(&conversation.conversation_id, 10)
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_id, outgoing.message_id);
        assert_eq!(messages[0].local_file_ref, None);
    }

    #[test]
    fn outgoing_text_and_file_commands_are_transactionally_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let profile = storage
            .load_or_create_profile("Sender", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([92; 16]);
        storage
            .upsert_nearby_peer(peer, "Receiver", Platform::Android, "192.168.1.92", 1)
            .unwrap();
        let conversation = storage
            .open_private_conversation(ClientOperationId::generate(), profile.device_id, peer)
            .unwrap();

        let text_operation = ClientOperationId::generate();
        let first_text = storage
            .create_outgoing_text_idempotent(
                &profile,
                text_operation,
                &conversation.conversation_id,
                "first payload",
            )
            .unwrap();
        let repeated_text = storage
            .create_outgoing_text_idempotent(
                &profile,
                text_operation,
                &conversation.conversation_id,
                "different retry payload",
            )
            .unwrap();
        assert_eq!(repeated_text, first_text);

        let source_path = temp.path().join("payload.bin");
        fs::write(&source_path, b"idempotent file").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::File).unwrap();
        let file_operation = ClientOperationId::generate();
        let first_offer = storage
            .create_outgoing_offer_idempotent(
                &profile,
                file_operation,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        let repeated_offer = storage
            .create_outgoing_offer_idempotent(
                &profile,
                file_operation,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        assert_eq!(repeated_offer, first_offer);

        for (table, expected) in [
            ("messages", 2_i64),
            ("transfers", 1),
            ("outbox", 2),
            ("client_operations", 3),
        ] {
            let count = storage
                .connection()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(count, expected, "unexpected row count in {table}");
        }
        for operation_type in [
            "open_private_conversation",
            "send_text_message",
            "send_source",
        ] {
            let count = storage
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM client_operations WHERE operation_type = ?1",
                    [operation_type],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(count, 1, "unexpected operation count for {operation_type}");
        }
    }

    #[test]
    fn suspending_network_queues_active_transfers_without_unpausing_user_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source.bin");
        fs::write(&source_path, b"active transfer").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::File).unwrap();
        let mut storage = Storage::open(temp.path().join("sender.db")).unwrap();
        let profile = storage
            .load_or_create_profile("computer", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([23; 16]);
        storage
            .upsert_nearby_peer(peer, "phone", Platform::Android, "192.168.1.23", 1)
            .unwrap();
        let conversation = storage
            .open_private_conversation(ClientOperationId::generate(), profile.device_id, peer)
            .unwrap();
        let outgoing = storage
            .create_outgoing_offer(
                &profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE transfers SET state = 'transferring' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        storage
            .connection()
            .execute(
                "UPDATE transfer_entries SET state = 'transferring' WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();

        storage.suspend_network_transfers().unwrap();
        let (queued, entries) = storage.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(queued.state, "queued");
        assert_eq!(queued.failure_reason.as_deref(), Some("connection_error"));
        assert!(entries.iter().all(|entry| entry.state == "queued"));

        storage
            .connection()
            .execute(
                "UPDATE transfers SET state = 'paused', paused_by_user = 1,
                    failure_reason = NULL WHERE transfer_id = ?1",
                [outgoing.transfer_id.to_string()],
            )
            .unwrap();
        storage.suspend_network_transfers().unwrap();
        let (paused, _) = storage.get_transfer(outgoing.transfer_id).unwrap();
        assert_eq!(paused.state, "paused");
        assert!(paused.paused_by_user);
        assert_eq!(paused.failure_reason, None);
    }

    #[test]
    fn replacement_sources_require_exact_metadata_before_resume() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("archive.bin");
        fs::write(&source_path, b"same-content").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::File).unwrap();
        let mut sender = Storage::open(temp.path().join("sender.db")).unwrap();
        let sender_profile = sender
            .load_or_create_profile("computer", Platform::Windows)
            .unwrap();
        let receiver_id = DeviceId::generate();
        sender
            .upsert_nearby_peer(receiver_id, "phone", Platform::Android, "192.168.1.2", 1)
            .unwrap();
        let conversation = sender
            .open_private_conversation(
                ClientOperationId::generate(),
                sender_profile.device_id,
                receiver_id,
            )
            .unwrap();
        let outgoing = sender
            .create_outgoing_offer(
                &sender_profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        sender
            .fail_transfer_source_changed(outgoing.transfer_id)
            .unwrap();
        let sources = manifest
            .entries
            .iter()
            .map(|entry| SourceItem {
                entry_kind: entry.entry_kind,
                source_ref: entry.source_ref.clone(),
                relative_path: entry.relative_path.clone(),
                size: entry.size,
                modified_at_ms: entry.modified_at_ms,
            })
            .collect::<Vec<_>>();
        let mut changed = sources.clone();
        changed[0].modified_at_ms += 1;
        assert!(matches!(
            sender.replace_transfer_sources(
                &sender_profile,
                ClientOperationId::generate(),
                outgoing.transfer_id,
                &changed,
            ),
            Err(StorageError::TransferSourceMismatch)
        ));
        let replace_operation = ClientOperationId::generate();
        let replace_event = sender
            .replace_transfer_sources(
                &sender_profile,
                replace_operation,
                outgoing.transfer_id,
                &sources,
            )
            .unwrap();
        assert_eq!(
            sender
                .replace_transfer_sources(
                    &sender_profile,
                    replace_operation,
                    outgoing.transfer_id,
                    &changed,
                )
                .unwrap(),
            replace_event
        );
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "accepted"
        );
    }

    #[test]
    fn failed_receive_can_replace_destination_and_resume_from_real_partial_length() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("large.bin");
        fs::write(&source_path, b"0123456789").unwrap();
        let manifest = crate::transfer::enumerate_path(&source_path, MessageKind::File).unwrap();
        let mut sender = Storage::open(temp.path().join("sender.db")).unwrap();
        let sender_profile = sender
            .load_or_create_profile("computer", Platform::Windows)
            .unwrap();
        let mut receiver = Storage::open(temp.path().join("receiver.db")).unwrap();
        let receiver_profile = receiver
            .load_or_create_profile("phone", Platform::Android)
            .unwrap();
        sender
            .upsert_nearby_peer(
                receiver_profile.device_id,
                "phone",
                Platform::Android,
                "192.168.1.2",
                1,
            )
            .unwrap();
        receiver
            .upsert_nearby_peer(
                sender_profile.device_id,
                "computer",
                Platform::Windows,
                "192.168.1.3",
                1,
            )
            .unwrap();
        let conversation = sender
            .open_private_conversation(
                ClientOperationId::generate(),
                sender_profile.device_id,
                receiver_profile.device_id,
            )
            .unwrap();
        let outgoing = sender
            .create_outgoing_offer(
                &sender_profile,
                &conversation.conversation_id,
                &manifest,
                None,
                false,
            )
            .unwrap();
        receiver
            .receive_file_offer(
                receiver_profile.device_id,
                sender_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
                &conversation.conversation_id,
                1,
                None,
                &manifest,
                None,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let (_, entries) = receiver.get_transfer(outgoing.transfer_id).unwrap();
        let first_dir = temp.path().join("first");
        let first_prepared = crate::transfer::prepare_receive_paths(
            &first_dir,
            outgoing.transfer_id,
            &manifest.display_name,
            &entries,
        )
        .unwrap();
        receiver
            .accept_incoming_offer(
                &receiver_profile,
                ClientOperationId::generate(),
                outgoing.transfer_id,
                first_dir.to_str().unwrap(),
                &first_prepared,
            )
            .unwrap();

        let failure_event = receiver
            .fail_receive_transfer(&receiver_profile, outgoing.transfer_id, "not_enough_space")
            .unwrap();
        let failed = receiver.get_transfer(outgoing.transfer_id).unwrap().0;
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.failure_reason.as_deref(), Some("not_enough_space"));
        let failure_outbox = receiver
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_id == failure_event)
            .unwrap();
        assert_eq!(failure_outbox.event_type, "file_reject");
        let failure_payload: serde_json::Value =
            serde_json::from_str(&failure_outbox.payload_json).unwrap();
        assert_eq!(failure_payload["body"]["reason"], "not_enough_space");
        sender
            .receive_file_reject(
                receiver_profile.device_id,
                failure_event,
                outgoing.transfer_id,
                "not_enough_space",
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "failed"
        );

        receiver
            .receive_transfer_pause(
                sender_profile.device_id,
                EventId::generate(),
                outgoing.transfer_id,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let still_failed = receiver.get_transfer(outgoing.transfer_id).unwrap().0;
        assert_eq!(still_failed.state, "failed");
        assert_eq!(
            still_failed.failure_reason.as_deref(),
            Some("not_enough_space")
        );

        let (_, entries) = receiver.get_transfer(outgoing.transfer_id).unwrap();
        let replacement_dir = temp.path().join("replacement");
        let prepared_once = crate::transfer::prepare_receive_paths(
            &replacement_dir,
            outgoing.transfer_id,
            &manifest.display_name,
            &entries,
        )
        .unwrap();
        fs::write(prepared_once[0].partial_ref.as_ref().unwrap(), b"0123").unwrap();
        let prepared = crate::transfer::prepare_receive_paths(
            &replacement_dir,
            outgoing.transfer_id,
            &manifest.display_name,
            &entries,
        )
        .unwrap();
        assert_eq!(prepared[0].persisted_offset, 4);
        let replace_operation = ClientOperationId::generate();
        let resume_event = receiver
            .replace_receive_destination(
                &receiver_profile,
                replace_operation,
                outgoing.transfer_id,
                replacement_dir.to_str().unwrap(),
                &prepared,
            )
            .unwrap();
        assert_eq!(
            receiver
                .replace_receive_destination(
                    &receiver_profile,
                    replace_operation,
                    outgoing.transfer_id,
                    "ignored-on-replay",
                    &[],
                )
                .unwrap(),
            resume_event
        );
        let resumed = receiver.get_transfer(outgoing.transfer_id).unwrap().0;
        assert_eq!(resumed.state, "accepted");
        assert_eq!(resumed.failure_reason, None);
        assert_eq!(resumed.persisted_bytes, 4);
        let resume = receiver
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_id == resume_event)
            .unwrap();
        assert_eq!(resume.event_type, "transfer_resume");
        let payload: serde_json::Value = serde_json::from_str(&resume.payload_json).unwrap();
        assert_eq!(payload["body"]["entries"][0]["offset"], 4);
        let offsets: Vec<AcceptedOffset> =
            serde_json::from_value(payload["body"]["entries"].clone()).unwrap();
        sender
            .receive_transfer_resume(
                &sender_profile,
                receiver_profile.device_id,
                resume_event,
                outgoing.transfer_id,
                &offsets,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(
            sender.get_transfer(outgoing.transfer_id).unwrap().0.state,
            "accepted"
        );
        assert_eq!(
            sender
                .schedulable_send_transfers(4)
                .unwrap()
                .iter()
                .map(|(transfer_id, _)| *transfer_id)
                .collect::<Vec<_>>(),
            vec![outgoing.transfer_id]
        );
    }

    #[test]
    fn clipboard_image_offer_persists_sequence_and_deduplicates_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("clipboard.png");
        fs::write(&source_path, b"clipboard-image-bytes").unwrap();
        let manifest =
            crate::transfer::enumerate_path(&source_path, MessageKind::ClipboardImage).unwrap();
        let sender_path = temp.path().join("sender.db");
        let receiver_path = temp.path().join("receiver.db");
        let mut sender = Storage::open(&sender_path).unwrap();
        let sender_profile = sender
            .load_or_create_profile("computer", Platform::Windows)
            .unwrap();
        let mut receiver = Storage::open(&receiver_path).unwrap();
        let receiver_profile = receiver
            .load_or_create_profile("phone", Platform::Android)
            .unwrap();
        sender
            .upsert_nearby_peer(
                receiver_profile.device_id,
                &receiver_profile.device_name,
                receiver_profile.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        receiver
            .upsert_nearby_peer(
                sender_profile.device_id,
                &sender_profile.device_name,
                sender_profile.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        let conversation = sender
            .open_private_conversation(
                ClientOperationId::generate(),
                sender_profile.device_id,
                receiver_profile.device_id,
            )
            .unwrap();
        let fingerprint = "a".repeat(64);
        let outgoing = sender
            .create_outgoing_offer(
                &sender_profile,
                &conversation.conversation_id,
                &manifest,
                Some(&fingerprint),
                false,
            )
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&sender.pending_outbox().unwrap().remove(0).payload_json).unwrap();
        assert_eq!(
            payload["body"]["origin_device_id"],
            sender_profile.device_id.to_string()
        );
        assert_eq!(payload["body"]["clipboard_sequence"], 1);
        assert_eq!(payload["body"]["content_fingerprint"], fingerprint);
        assert_eq!(payload["body"]["automatic"], false);

        let metadata = ClipboardImageMetadata {
            origin_device_id: sender_profile.device_id,
            clipboard_sequence: 1,
            content_fingerprint: "a".repeat(64),
            automatic: false,
        };
        receiver
            .receive_file_offer(
                receiver_profile.device_id,
                sender_profile.device_id,
                outgoing.event_id,
                outgoing.message_id,
                outgoing.transfer_id,
                &conversation.conversation_id,
                1,
                None,
                &manifest,
                Some(&metadata),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        drop(receiver);

        let mut receiver = Storage::open(&receiver_path).unwrap();
        let duplicate = receiver
            .receive_file_offer(
                receiver_profile.device_id,
                sender_profile.device_id,
                EventId::generate(),
                MessageId::generate(),
                TransferId::generate(),
                &conversation.conversation_id,
                2,
                None,
                &manifest,
                Some(&metadata),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(
            receiver
                .connection()
                .query_row("SELECT COUNT(*) FROM transfers", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
