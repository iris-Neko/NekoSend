use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{
    LocalProfile, MAX_CLIPBOARD_TEXT_BYTES, Storage, StorageError, insert_outbox_event,
    insert_processed_event, next_sort_order, run_client_operation, save_client_operation,
    saved_client_operation, saved_receipt, unix_time_ms,
};
use crate::domain::{
    BindingId, ClientOperationId, DeviceId, EventId, MessageId, PrivateConversationId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnDeviceBindingRecord {
    pub peer_device_id: DeviceId,
    pub peer_name: String,
    pub binding_id: BindingId,
    pub state: String,
    pub clipboard_mode: String,
    pub requested_by_device_id: DeviceId,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSendRecord {
    pub message_id: MessageId,
    pub event_id: EventId,
    pub clipboard_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveClipboardResult {
    pub receipt_json: String,
    pub duplicate: bool,
    pub write_to_system: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct BindingOperationResult {
    binding_id: BindingId,
}

impl Storage {
    pub fn list_own_device_bindings(
        &self,
        state: Option<&str>,
    ) -> Result<Vec<OwnDeviceBindingRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT b.peer_device_id, p.device_name, b.binding_id, b.state,
                        b.clipboard_mode, b.requested_by_device_id,
                        b.created_at_ms, b.updated_at_ms
                 FROM own_device_bindings b JOIN peers p ON p.device_id = b.peer_device_id
                 WHERE (?1 IS NULL OR b.state = ?1)
                 ORDER BY b.updated_at_ms DESC",
            )
            .map_err(StorageError::Write)?;
        statement
            .query_map([state], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .map_err(StorageError::Write)?
            .map(|row| {
                let row = row.map_err(StorageError::Write)?;
                Ok(OwnDeviceBindingRecord {
                    peer_device_id: row.0.parse().map_err(|_| StorageError::InvalidStoredId)?,
                    peer_name: row.1,
                    binding_id: row.2.parse().map_err(|_| StorageError::InvalidStoredId)?,
                    state: row.3,
                    clipboard_mode: row.4,
                    requested_by_device_id: row
                        .5
                        .parse()
                        .map_err(|_| StorageError::InvalidStoredId)?,
                    created_at_ms: row.6,
                    updated_at_ms: row.7,
                })
            })
            .collect()
    }

    pub fn request_own_device_binding(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        peer_device_id: DeviceId,
    ) -> Result<BindingId, StorageError> {
        if peer_device_id == profile.device_id {
            return Err(StorageError::PeerNotFound);
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = transaction
            .query_row(
                "SELECT result_json FROM client_operations
                 WHERE client_operation_id = ?1 AND operation_type = 'request_own_device_binding'",
                [client_operation_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
        {
            let result: BindingOperationResult =
                serde_json::from_str(&saved).map_err(|_| StorageError::InvalidStoredId)?;
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(result.binding_id);
        }
        let peer_exists = transaction
            .query_row(
                "SELECT 1 FROM peers WHERE device_id = ?1",
                [peer_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !peer_exists {
            return Err(StorageError::PeerNotFound);
        }
        let active = transaction
            .query_row(
                "SELECT binding_id FROM own_device_bindings
                 WHERE peer_device_id = ?1 AND state = 'active'",
                [peer_device_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?;
        let binding_id = if let Some(active) = active {
            active.parse().map_err(|_| StorageError::InvalidStoredId)?
        } else {
            let binding_id = BindingId::generate();
            transaction
                .execute(
                    "INSERT INTO own_device_bindings(peer_device_id, binding_id, state,
                        clipboard_mode, requested_by_device_id, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, 'pending_outbound', 'off', ?3, ?4, ?4)
                     ON CONFLICT(peer_device_id) DO UPDATE SET binding_id = excluded.binding_id,
                        state = 'pending_outbound', clipboard_mode = 'off',
                        requested_by_device_id = excluded.requested_by_device_id,
                        created_at_ms = excluded.created_at_ms,
                        updated_at_ms = excluded.updated_at_ms",
                    params![
                        peer_device_id.to_string(),
                        binding_id.to_string(),
                        profile.device_id.to_string(),
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
            let event_id = EventId::generate();
            let payload = serde_json::json!({
                "version": 1,
                "type": "own_device_bind",
                "event_id": event_id,
                "sender_device_id": profile.device_id,
                "sent_at_ms": now,
                "body": {
                    "binding_id": binding_id,
                    "requester_name": profile.device_name
                }
            })
            .to_string();
            insert_outbox_event(
                &transaction,
                peer_device_id,
                event_id,
                "own_device_bind",
                &payload,
                now,
            )?;
            binding_id
        };
        let result_json = serde_json::to_string(&BindingOperationResult { binding_id })
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO client_operations(client_operation_id, operation_type,
                    result_json, created_at_ms)
                 VALUES (?1, 'request_own_device_binding', ?2, ?3)",
                params![client_operation_id.to_string(), result_json, now],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(binding_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_own_device_bind(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        binding_id: BindingId,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let peer_exists = transaction
            .query_row(
                "SELECT 1 FROM peers WHERE device_id = ?1",
                [sender_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !peer_exists || sender_device_id == local_device_id {
            return Err(StorageError::PeerNotFound);
        }
        let already_active = transaction
            .query_row(
                "SELECT state FROM own_device_bindings WHERE peer_device_id = ?1",
                [sender_device_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .as_deref()
            == Some("active");
        if !already_active {
            transaction
                .execute(
                    "INSERT INTO own_device_bindings(peer_device_id, binding_id, state,
                        clipboard_mode, requested_by_device_id, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, 'pending_inbound', 'off', ?1, ?3, ?3)
                     ON CONFLICT(peer_device_id) DO UPDATE SET binding_id = excluded.binding_id,
                        state = 'pending_inbound', clipboard_mode = 'off',
                        requested_by_device_id = excluded.requested_by_device_id,
                        created_at_ms = excluded.created_at_ms,
                        updated_at_ms = excluded.updated_at_ms",
                    params![sender_device_id.to_string(), binding_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "own_device_bind",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn decide_own_device_binding(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        binding_id: BindingId,
        accept: bool,
    ) -> Result<DeviceId, StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<DeviceId>(
            &transaction,
            client_operation_id,
            "decide_own_device_binding",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let peer: String = transaction
            .query_row(
                "SELECT peer_device_id FROM own_device_bindings
                 WHERE binding_id = ?1 AND state = 'pending_inbound'",
                [binding_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::BindingNotFound)?;
        transaction
            .execute(
                "UPDATE own_device_bindings SET state = ?2, clipboard_mode = 'off',
                    updated_at_ms = ?3 WHERE binding_id = ?1",
                params![
                    binding_id.to_string(),
                    if accept { "active" } else { "rejected" },
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        if accept {
            transaction
                .execute(
                    "UPDATE peers SET relation = 'own_device', known_at_ms = COALESCE(known_at_ms, ?2),
                        updated_at_ms = ?2 WHERE device_id = ?1",
                    params![peer, now],
                )
                .map_err(StorageError::Write)?;
        }
        let peer_device_id: DeviceId = peer.parse().map_err(|_| StorageError::InvalidStoredId)?;
        let event_id = EventId::generate();
        let payload = serde_json::json!({
            "version": 1,
            "type": "own_device_bind_reply",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "binding_id": binding_id,
                "decision": if accept { "accepted" } else { "rejected" }
            }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            peer_device_id,
            event_id,
            "own_device_bind_reply",
            &payload,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "decide_own_device_binding",
            &peer_device_id,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(peer_device_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_own_device_bind_reply(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        binding_id: BindingId,
        accepted: bool,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let changed = transaction
            .execute(
                "UPDATE own_device_bindings SET state = ?4, clipboard_mode = 'off',
                    updated_at_ms = ?3
                 WHERE peer_device_id = ?1 AND binding_id = ?2 AND state = 'pending_outbound'",
                params![
                    sender_device_id.to_string(),
                    binding_id.to_string(),
                    now,
                    if accepted { "active" } else { "rejected" }
                ],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::BindingNotFound);
        }
        if accepted {
            transaction
                .execute(
                    "UPDATE peers SET relation = 'own_device', known_at_ms = COALESCE(known_at_ms, ?2),
                        updated_at_ms = ?2 WHERE device_id = ?1",
                    params![sender_device_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "own_device_bind_reply",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn remove_own_device_binding(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        peer_device_id: DeviceId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if saved_client_operation::<()>(
            &transaction,
            client_operation_id,
            "remove_own_device_binding",
        )?
        .is_some()
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(());
        }
        let binding: String = transaction
            .query_row(
                "SELECT binding_id FROM own_device_bindings
                 WHERE peer_device_id = ?1 AND state IN ('active', 'pending_outbound', 'pending_inbound')",
                [peer_device_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::BindingNotFound)?;
        let binding_id: BindingId = binding.parse().map_err(|_| StorageError::InvalidStoredId)?;
        transaction
            .execute(
                "UPDATE own_device_bindings SET state = 'removed', clipboard_mode = 'off',
                    updated_at_ms = ?2 WHERE peer_device_id = ?1",
                params![peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE peers SET relation = 'known', updated_at_ms = ?2 WHERE device_id = ?1",
                params![peer_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let event_id = EventId::generate();
        let payload = serde_json::json!({
            "version": 1,
            "type": "own_device_unbind",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": { "binding_id": binding_id }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            peer_device_id,
            event_id,
            "own_device_unbind",
            &payload,
            now,
        )?;
        save_client_operation(
            &transaction,
            client_operation_id,
            "remove_own_device_binding",
            &(),
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn receive_own_device_unbind(
        &mut self,
        sender_device_id: DeviceId,
        event_id: EventId,
        binding_id: BindingId,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let changed = transaction
            .execute(
                "UPDATE own_device_bindings SET state = 'removed', clipboard_mode = 'off',
                    updated_at_ms = ?3 WHERE peer_device_id = ?1 AND binding_id = ?2",
                params![sender_device_id.to_string(), binding_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::BindingNotFound);
        }
        transaction
            .execute(
                "UPDATE peers SET relation = 'known', updated_at_ms = ?2 WHERE device_id = ?1",
                params![sender_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "own_device_unbind",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn set_clipboard_mode(
        &mut self,
        client_operation_id: ClientOperationId,
        peer_device_id: DeviceId,
        mode: &str,
    ) -> Result<(), StorageError> {
        if !matches!(mode, "off" | "send_only" | "receive_only" | "bidirectional") {
            return Err(StorageError::InvalidClipboardMode);
        }
        run_client_operation(
            &mut self.connection,
            client_operation_id,
            "set_clipboard_mode",
            |transaction, now| {
                let changed = transaction
                    .execute(
                        "UPDATE own_device_bindings SET clipboard_mode = ?2, updated_at_ms = ?3
                         WHERE peer_device_id = ?1 AND state = 'active'",
                        params![peer_device_id.to_string(), mode, now],
                    )
                    .map_err(StorageError::Write)?;
                if changed == 0 {
                    return Err(StorageError::ClipboardBindingRequired);
                }
                Ok(())
            },
        )
    }

    pub fn submit_local_clipboard_text(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        peer_device_id: DeviceId,
        text: &str,
        automatic: bool,
    ) -> Result<ClipboardSendRecord, StorageError> {
        validate_clipboard_text(text)?;
        let conversation_id = PrivateConversationId::new(profile.device_id, peer_device_id)
            .map_err(|_| StorageError::PeerNotFound)?
            .to_string();
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<ClipboardSendRecord>(
            &transaction,
            client_operation_id,
            "submit_local_clipboard_text",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        if automatic {
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
        }
        let exists = transaction
            .query_row(
                "SELECT 1 FROM conversations WHERE conversation_id = ?1 AND kind = 'private'",
                [&conversation_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !exists {
            return Err(StorageError::PeerNotFound);
        }
        let previous_sequence = transaction
            .query_row(
                "SELECT clipboard_sequence FROM local_profile WHERE singleton_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(StorageError::Write)?;
        let next_sequence = previous_sequence
            .checked_add(1)
            .ok_or(StorageError::ClipboardSequenceExhausted)?;
        let sequence =
            u64::try_from(next_sequence).map_err(|_| StorageError::ClipboardSequenceExhausted)?;
        transaction
            .execute(
                "UPDATE local_profile SET clipboard_sequence = ?1, updated_at_ms = ?2
                 WHERE singleton_id = 1",
                params![next_sequence, now],
            )
            .map_err(StorageError::Write)?;
        let message_id = MessageId::generate();
        let event_id = EventId::generate();
        let fingerprint = clipboard_fingerprint(text);
        let sort_order = next_sort_order(&transaction, &conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(message_id, conversation_id, sender_device_id, kind,
                    state, text_content, created_at_ms, local_sort_order)
                 VALUES (?1, ?2, ?3, 'clipboard_text', 'queued', ?4, ?5, ?6)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    profile.device_id.to_string(),
                    text,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO message_deliveries(message_id, recipient_device_id, state,
                    last_event_id, updated_at_ms) VALUES (?1, ?2, 'queued', ?3, ?4)",
                params![
                    message_id.to_string(),
                    peer_device_id.to_string(),
                    event_id.to_string(),
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO clipboard_dedup(origin_device_id, clipboard_sequence, message_id,
                    content_fingerprint, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile.device_id.to_string(),
                    next_sequence,
                    message_id.to_string(),
                    fingerprint,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        let payload = serde_json::json!({
            "version": 1,
            "type": "clipboard_update",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "message_id": message_id,
                "conversation_id": conversation_id,
                "origin_device_id": profile.device_id,
                "clipboard_sequence": sequence,
                "content_type": "text",
                "text": text,
                "automatic": automatic
            }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            peer_device_id,
            event_id,
            "clipboard_update",
            &payload,
            now,
        )?;
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let result = ClipboardSendRecord {
            message_id,
            event_id,
            clipboard_sequence: sequence,
        };
        save_client_operation(
            &transaction,
            client_operation_id,
            "submit_local_clipboard_text",
            &result,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_clipboard_text(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        message_id: MessageId,
        conversation_id: &str,
        origin_device_id: DeviceId,
        clipboard_sequence: u64,
        text: &str,
        automatic: bool,
        receipt_json: &str,
    ) -> Result<ReceiveClipboardResult, StorageError> {
        validate_clipboard_text(text)?;
        if origin_device_id != sender_device_id || clipboard_sequence == 0 {
            return Err(StorageError::PeerNotFound);
        }
        let expected = PrivateConversationId::new(local_device_id, sender_device_id)
            .map_err(|_| StorageError::PeerNotFound)?
            .to_string();
        if conversation_id != expected {
            return Err(StorageError::PeerNotFound);
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(ReceiveClipboardResult {
                receipt_json: saved,
                duplicate: true,
                write_to_system: false,
            });
        }
        let (sender_name, mode) = transaction
            .query_row(
                "SELECT p.device_name, b.clipboard_mode
                 FROM peers p LEFT JOIN own_device_bindings b
                    ON b.peer_device_id = p.device_id AND b.state = 'active'
                 WHERE p.device_id = ?1",
                [sender_device_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::PeerNotFound)?;
        let write_to_system =
            automatic && matches!(mode.as_deref(), Some("receive_only" | "bidirectional"));
        if automatic && mode.is_none() {
            return Err(StorageError::ClipboardBindingRequired);
        }
        let duplicate_sequence = transaction
            .query_row(
                "SELECT 1 FROM clipboard_dedup
                 WHERE origin_device_id = ?1 AND clipboard_sequence = ?2",
                params![
                    origin_device_id.to_string(),
                    i64::try_from(clipboard_sequence).unwrap_or(i64::MAX)
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
                "clipboard_update",
                receipt_json,
                now,
            )?;
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(ReceiveClipboardResult {
                receipt_json: receipt_json.to_owned(),
                duplicate: true,
                write_to_system: false,
            });
        }
        transaction
            .execute(
                "INSERT INTO conversations(conversation_id, kind, state, peer_device_id,
                    title_cache, last_activity_at_ms, unread_count, created_at_ms)
                 VALUES (?1, 'private', 'active', ?2, ?3, ?4, 0, ?4)
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
                "INSERT OR IGNORE INTO messages(message_id, conversation_id, sender_device_id,
                    kind, state, text_content, created_at_ms, received_at_ms, local_sort_order)
                 VALUES (?1, ?2, ?3, 'clipboard_text', 'delivered', ?4, ?5, ?6, ?7)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    sender_device_id.to_string(),
                    text,
                    now,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO clipboard_dedup(origin_device_id, clipboard_sequence, message_id,
                    content_fingerprint, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    origin_device_id.to_string(),
                    i64::try_from(clipboard_sequence)
                        .map_err(|_| StorageError::ClipboardSequenceExhausted)?,
                    message_id.to_string(),
                    clipboard_fingerprint(text),
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3,
                    unread_count = unread_count + 1, deleted_at_ms = NULL
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "clipboard_update",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(ReceiveClipboardResult {
            receipt_json: receipt_json.to_owned(),
            duplicate: false,
            write_to_system,
        })
    }
}

fn validate_clipboard_text(text: &str) -> Result<(), StorageError> {
    if text.is_empty() || text.len() > MAX_CLIPBOARD_TEXT_BYTES {
        return Err(StorageError::ClipboardTooLarge);
    }
    Ok(())
}

fn clipboard_fingerprint(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:x}:{hash:016x}", text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ClientOperationId, Platform};

    #[test]
    fn binding_clipboard_dedup_and_unbind_are_transactional() {
        let temp = tempfile::tempdir().unwrap();
        let mut left_storage = Storage::open(temp.path().join("left.db")).unwrap();
        let left = left_storage
            .load_or_create_profile("Windows", Platform::Windows)
            .unwrap();
        let mut right_storage = Storage::open(temp.path().join("right.db")).unwrap();
        let right = right_storage
            .load_or_create_profile("Android", Platform::Android)
            .unwrap();
        left_storage
            .upsert_nearby_peer(
                right.device_id,
                &right.device_name,
                right.platform,
                "127.0.0.2",
                1,
            )
            .unwrap();
        right_storage
            .upsert_nearby_peer(
                left.device_id,
                &left.device_name,
                left.platform,
                "127.0.0.1",
                1,
            )
            .unwrap();
        let conversation = left_storage
            .open_private_conversation(
                ClientOperationId::generate(),
                left.device_id,
                right.device_id,
            )
            .unwrap();

        let operation_id = ClientOperationId::generate();
        let binding_id = left_storage
            .request_own_device_binding(&left, operation_id, right.device_id)
            .unwrap();
        assert_eq!(
            left_storage
                .request_own_device_binding(&left, operation_id, right.device_id)
                .unwrap(),
            binding_id
        );
        let bind_event = left_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "own_device_bind")
            .unwrap();
        right_storage
            .receive_own_device_bind(
                right.device_id,
                left.device_id,
                bind_event.event_id,
                binding_id,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        right_storage
            .receive_own_device_bind(
                right.device_id,
                left.device_id,
                bind_event.event_id,
                binding_id,
                "different receipt is ignored",
            )
            .unwrap();
        assert_eq!(
            right_storage
                .list_own_device_bindings(Some("pending_inbound"))
                .unwrap()
                .len(),
            1
        );
        left_storage
            .mark_control_event_stored(right.device_id, bind_event.event_id)
            .unwrap();

        let decide_operation = ClientOperationId::generate();
        let decided_peer = right_storage
            .decide_own_device_binding(&right, decide_operation, binding_id, true)
            .unwrap();
        assert_eq!(decided_peer, left.device_id);
        assert_eq!(
            right_storage
                .decide_own_device_binding(&right, decide_operation, binding_id, false)
                .unwrap(),
            left.device_id
        );
        assert_eq!(
            right_storage
                .pending_outbox()
                .unwrap()
                .iter()
                .filter(|entry| entry.event_type == "own_device_bind_reply")
                .count(),
            1
        );
        let reply_event = right_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "own_device_bind_reply")
            .unwrap();
        left_storage
            .receive_own_device_bind_reply(
                right.device_id,
                reply_event.event_id,
                binding_id,
                true,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(
            left_storage
                .list_own_device_bindings(Some("active"))
                .unwrap()
                .single()
                .clipboard_mode,
            "off"
        );
        assert_eq!(
            right_storage
                .list_own_device_bindings(Some("active"))
                .unwrap()
                .len(),
            1
        );

        let mode_operation = ClientOperationId::generate();
        left_storage
            .set_clipboard_mode(mode_operation, right.device_id, "send_only")
            .unwrap();
        left_storage
            .set_clipboard_mode(mode_operation, right.device_id, "off")
            .unwrap();
        assert_eq!(
            left_storage
                .list_own_device_bindings(Some("active"))
                .unwrap()
                .single()
                .clipboard_mode,
            "send_only"
        );
        right_storage
            .set_clipboard_mode(
                ClientOperationId::generate(),
                left.device_id,
                "receive_only",
            )
            .unwrap();
        let clipboard_operation = ClientOperationId::generate();
        let sent = left_storage
            .submit_local_clipboard_text(
                &left,
                clipboard_operation,
                right.device_id,
                "192.168.1.20",
                true,
            )
            .unwrap();
        let repeated = left_storage
            .submit_local_clipboard_text(
                &left,
                clipboard_operation,
                right.device_id,
                "different text is ignored",
                true,
            )
            .unwrap();
        assert_eq!(sent, repeated);
        assert_eq!(
            left_storage
                .pending_outbox()
                .unwrap()
                .iter()
                .filter(|entry| entry.event_type == "clipboard_update")
                .count(),
            1
        );
        let clipboard_event = left_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "clipboard_update")
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&clipboard_event.payload_json).unwrap();
        let received = right_storage
            .receive_clipboard_text(
                right.device_id,
                left.device_id,
                clipboard_event.event_id,
                sent.message_id,
                &conversation.conversation_id,
                left.device_id,
                sent.clipboard_sequence,
                payload["body"]["text"].as_str().unwrap(),
                true,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert!(received.write_to_system);
        assert!(!received.duplicate);
        let duplicate = right_storage
            .receive_clipboard_text(
                right.device_id,
                left.device_id,
                clipboard_event.event_id,
                sent.message_id,
                &conversation.conversation_id,
                left.device_id,
                sent.clipboard_sequence,
                "192.168.1.20",
                true,
                "ignored",
            )
            .unwrap();
        assert!(duplicate.duplicate);
        assert!(!duplicate.write_to_system);
        assert_eq!(
            right_storage
                .list_messages(&conversation.conversation_id, 100)
                .unwrap()
                .iter()
                .filter(|message| message.message_id == sent.message_id)
                .count(),
            1
        );

        let remove_operation = ClientOperationId::generate();
        left_storage
            .remove_own_device_binding(&left, remove_operation, right.device_id)
            .unwrap();
        left_storage
            .remove_own_device_binding(&left, remove_operation, right.device_id)
            .unwrap();
        assert_eq!(
            left_storage
                .pending_outbox()
                .unwrap()
                .iter()
                .filter(|entry| entry.event_type == "own_device_unbind")
                .count(),
            1
        );
        let unbind_event = left_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "own_device_unbind")
            .unwrap();
        right_storage
            .receive_own_device_unbind(
                left.device_id,
                unbind_event.event_id,
                binding_id,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(
            left_storage
                .list_own_device_bindings(None)
                .unwrap()
                .single()
                .state,
            "removed"
        );
        let right_binding = right_storage
            .list_own_device_bindings(None)
            .unwrap()
            .single()
            .clone();
        assert_eq!(right_binding.state, "removed");
        assert_eq!(right_binding.clipboard_mode, "off");
        assert!(matches!(
            left_storage.submit_local_clipboard_text(
                &left,
                ClientOperationId::generate(),
                right.device_id,
                "cannot auto send",
                true
            ),
            Err(StorageError::ClipboardBindingRequired)
        ));
    }

    trait Single<T> {
        fn single(&self) -> &T;
    }

    impl<T> Single<T> for Vec<T> {
        fn single(&self) -> &T {
            assert_eq!(self.len(), 1);
            &self[0]
        }
    }
}
