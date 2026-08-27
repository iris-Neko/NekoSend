use std::collections::HashSet;

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{
    LocalProfile, MessageRecord, Storage, StorageError, insert_outbox_event,
    insert_processed_event, next_sort_order, save_client_operation, saved_client_operation,
    saved_receipt, unix_time_ms,
};
use crate::domain::{ClientOperationId, DeviceId, EventId, GroupId, InviteId, MessageId};

const MAX_GROUP_MEMBERS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMemberSnapshot {
    pub device_id: DeviceId,
    pub role: String,
    pub membership: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSnapshot {
    pub group_id: GroupId,
    pub name: String,
    pub owner_device_id: DeviceId,
    pub revision: u64,
    pub created_at_ms: i64,
    #[serde(default = "active_group_state")]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disbanded_at_ms: Option<i64>,
    pub members: Vec<GroupMemberSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRecord {
    pub snapshot: GroupSnapshot,
    pub local_role: String,
    pub local_membership: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInvitationRecord {
    pub invite_id: InviteId,
    pub group_id: GroupId,
    pub group_name: String,
    pub inviter_device_id: DeviceId,
    pub inviter_name: String,
    pub member_device_ids: Vec<DeviceId>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupResult {
    pub group_id: GroupId,
    pub conversation_id: String,
    pub invite_ids: Vec<InviteId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateGroupResult {
    pub group_id: GroupId,
    pub revision: u64,
    pub invite_ids: Vec<InviteId>,
}

impl Storage {
    pub fn create_group(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        name: &str,
        member_device_ids: &[DeviceId],
    ) -> Result<CreateGroupResult, StorageError> {
        validate_group_name(name)?;
        let unique = member_device_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != member_device_ids.len()
            || unique.contains(&profile.device_id)
            || member_device_ids.is_empty()
            || member_device_ids.len() + 1 > MAX_GROUP_MEMBERS
        {
            return Err(StorageError::InvalidGroupMembers);
        }

        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = transaction
            .query_row(
                "SELECT result_json FROM client_operations
                 WHERE client_operation_id = ?1 AND operation_type = 'create_group'",
                [client_operation_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
        {
            let result = serde_json::from_str(&saved).map_err(|_| StorageError::InvalidStoredId)?;
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(result);
        }

        for member in member_device_ids {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM peers WHERE device_id = ?1",
                    [member.to_string()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(StorageError::Write)?
                .is_some();
            if !exists {
                return Err(StorageError::PeerNotFound);
            }
        }
        let previous_sequence = transaction
            .query_row(
                "SELECT group_sequence FROM local_profile WHERE singleton_id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(StorageError::Write)?;
        let next_sequence = previous_sequence
            .checked_add(1)
            .ok_or(StorageError::GroupSequenceExhausted)?;
        let group_id = GroupId::new(
            profile.device_id,
            u64::try_from(next_sequence).map_err(|_| StorageError::GroupSequenceExhausted)?,
        )
        .map_err(|_| StorageError::GroupSequenceExhausted)?;
        transaction
            .execute(
                "UPDATE local_profile SET group_sequence = ?1, updated_at_ms = ?2
                 WHERE singleton_id = 1",
                params![next_sequence, now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO groups(group_id, name, owner_device_id, revision, state,
                    created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, 'active', ?4, ?4)",
                params![
                    group_id.to_string(),
                    name,
                    profile.device_id.to_string(),
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "INSERT INTO group_members(group_id, device_id, role, membership,
                    joined_at_ms, updated_at_ms)
                 VALUES (?1, ?2, 'owner', 'joined', ?3, ?3)",
                params![group_id.to_string(), profile.device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        for member in member_device_ids {
            transaction
                .execute(
                    "INSERT INTO group_members(group_id, device_id, role, membership, updated_at_ms)
                     VALUES (?1, ?2, 'member', 'invited', ?3)",
                    params![group_id.to_string(), member.to_string(), now],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "INSERT INTO conversations(conversation_id, kind, state, group_id, title_cache,
                    last_activity_at_ms, unread_count, created_at_ms)
                 VALUES (?1, 'group', 'active', ?1, ?2, ?3, 0, ?3)",
                params![group_id.to_string(), name, now],
            )
            .map_err(StorageError::Write)?;

        let snapshot = build_group_snapshot(&transaction, &group_id)?;
        let snapshot_json = serde_json::to_string(&snapshot)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let mut invite_ids = Vec::with_capacity(member_device_ids.len());
        for member in member_device_ids {
            let invite_id = InviteId::generate();
            let event_id = EventId::generate();
            let payload = serde_json::json!({
                "version": 1,
                "type": "group_invite",
                "event_id": event_id,
                "sender_device_id": profile.device_id,
                "sent_at_ms": now,
                "body": { "invite_id": invite_id, "group": snapshot }
            })
            .to_string();
            transaction
                .execute(
                    "INSERT INTO group_invitations(invite_id, group_id, peer_device_id,
                        direction, state, snapshot_json, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, 'send', 'pending', ?4, ?5, ?5)",
                    params![
                        invite_id.to_string(),
                        group_id.to_string(),
                        member.to_string(),
                        snapshot_json,
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
            insert_outbox_event(
                &transaction,
                *member,
                event_id,
                "group_invite",
                &payload,
                now,
            )?;
            invite_ids.push(invite_id);
        }
        let result = CreateGroupResult {
            group_id: group_id.clone(),
            conversation_id: group_id.to_string(),
            invite_ids,
        };
        let result_json = serde_json::to_string(&result)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO client_operations(client_operation_id, operation_type,
                    result_json, created_at_ms) VALUES (?1, 'create_group', ?2, ?3)",
                params![client_operation_id.to_string(), result_json, now],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    pub fn receive_group_invite(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        invite_id: InviteId,
        snapshot: &GroupSnapshot,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        validate_group_snapshot(snapshot)?;
        let local_member = snapshot
            .members
            .iter()
            .find(|member| member.device_id == local_device_id)
            .ok_or(StorageError::InvalidGroupMembers)?;
        if snapshot.owner_device_id != sender_device_id
            || local_member.membership != "invited"
            || local_member.role != "member"
        {
            return Err(StorageError::InvalidGroupMembers);
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let inviter_exists = transaction
            .query_row(
                "SELECT 1 FROM peers WHERE device_id = ?1",
                [sender_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !inviter_exists {
            return Err(StorageError::PeerNotFound);
        }
        let snapshot_json = serde_json::to_string(snapshot)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO group_invitations(invite_id, group_id, peer_device_id,
                    direction, state, snapshot_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 'receive', 'pending', ?4, ?5, ?5)",
                params![
                    invite_id.to_string(),
                    snapshot.group_id.to_string(),
                    sender_device_id.to_string(),
                    snapshot_json,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_invite",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn list_group_invitations(&self) -> Result<Vec<GroupInvitationRecord>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT i.invite_id, i.group_id, i.peer_device_id, p.device_name,
                        i.snapshot_json, i.created_at_ms
                 FROM group_invitations i
                 JOIN peers p ON p.device_id = i.peer_device_id
                 WHERE i.direction = 'receive' AND i.state = 'pending'
                 ORDER BY i.created_at_ms DESC",
            )
            .map_err(StorageError::Write)?;
        let rows = statement
            .query_map([], |row| {
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
            let (invite, group, inviter, inviter_name, snapshot, created_at_ms) =
                row.map_err(StorageError::Write)?;
            let snapshot: GroupSnapshot =
                serde_json::from_str(&snapshot).map_err(|_| StorageError::InvalidStoredId)?;
            Ok(GroupInvitationRecord {
                invite_id: invite.parse().map_err(|_| StorageError::InvalidStoredId)?,
                group_id: group.parse().map_err(|_| StorageError::InvalidStoredId)?,
                group_name: snapshot.name,
                inviter_device_id: inviter.parse().map_err(|_| StorageError::InvalidStoredId)?,
                inviter_name,
                member_device_ids: snapshot
                    .members
                    .into_iter()
                    .map(|member| member.device_id)
                    .collect(),
                created_at_ms,
            })
        })
        .collect()
    }

    pub fn decide_group_invite(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        invite_id: InviteId,
        accept: bool,
    ) -> Result<Option<GroupId>, StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_client_operation::<Option<GroupId>>(
            &transaction,
            client_operation_id,
            "decide_group_invite",
        )? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let (group_id_text, inviter_text, state, snapshot_json) = transaction
            .query_row(
                "SELECT group_id, peer_device_id, state, snapshot_json
                 FROM group_invitations
                 WHERE invite_id = ?1 AND direction = 'receive'",
                [invite_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::InviteNotFound)?;
        let group_id: GroupId = group_id_text
            .parse()
            .map_err(|_| StorageError::InvalidStoredId)?;
        if state != "pending" {
            let result = accept.then_some(group_id);
            save_client_operation(
                &transaction,
                client_operation_id,
                "decide_group_invite",
                &result,
                now,
            )?;
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(result);
        }
        let inviter: DeviceId = inviter_text
            .parse()
            .map_err(|_| StorageError::InvalidStoredId)?;
        let mut snapshot: GroupSnapshot =
            serde_json::from_str(&snapshot_json).map_err(|_| StorageError::InvalidStoredId)?;
        validate_group_snapshot(&snapshot)?;
        if accept {
            snapshot
                .members
                .iter_mut()
                .find(|member| member.device_id == profile.device_id)
                .ok_or(StorageError::InvalidGroupMembers)?
                .membership = "joined".to_owned();
            apply_group_snapshot(&transaction, profile.device_id, &snapshot, now)?;
            insert_group_system_message(
                &transaction,
                profile.device_id,
                group_id.clone(),
                "你加入了群聊",
                now,
            )?;
        }
        transaction
            .execute(
                "UPDATE group_invitations SET state = ?2, updated_at_ms = ?3
                 WHERE invite_id = ?1",
                params![
                    invite_id.to_string(),
                    if accept { "accepted" } else { "rejected" },
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        let event_id = EventId::generate();
        let decision = if accept { "accepted" } else { "rejected" };
        let payload = serde_json::json!({
            "version": 1,
            "type": "group_invite_reply",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "invite_id": invite_id,
                "group_id": group_id,
                "decision": decision
            }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            inviter,
            event_id,
            "group_invite_reply",
            &payload,
            now,
        )?;
        let result = accept.then_some(group_id);
        save_client_operation(
            &transaction,
            client_operation_id,
            "decide_group_invite",
            &result,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_group_invite_reply(
        &mut self,
        profile: &LocalProfile,
        sender_device_id: DeviceId,
        event_id: EventId,
        invite_id: InviteId,
        group_id: GroupId,
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
        let owner: String = transaction
            .query_row(
                "SELECT owner_device_id FROM groups WHERE group_id = ?1 AND state = 'active'",
                [group_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        if owner != profile.device_id.to_string() {
            return Err(StorageError::NotGroupOwner);
        }
        let invitation_matches = transaction
            .query_row(
                "SELECT 1 FROM group_invitations
                 WHERE invite_id = ?1 AND group_id = ?2 AND peer_device_id = ?3
                   AND direction = 'send' AND state = 'pending'",
                params![
                    invite_id.to_string(),
                    group_id.to_string(),
                    sender_device_id.to_string()
                ],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !invitation_matches {
            return Err(StorageError::InviteNotFound);
        }
        let previous_revision = current_group_revision(&transaction, &group_id)?;
        transaction
            .execute(
                "UPDATE group_invitations SET state = ?2, updated_at_ms = ?3
                 WHERE invite_id = ?1",
                params![
                    invite_id.to_string(),
                    if accepted { "accepted" } else { "rejected" },
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE group_members
                 SET membership = ?3, joined_at_ms = CASE WHEN ?3 = 'joined' THEN ?4 ELSE NULL END,
                     updated_at_ms = ?4
                 WHERE group_id = ?1 AND device_id = ?2 AND membership = 'invited'",
                params![
                    group_id.to_string(),
                    sender_device_id.to_string(),
                    if accepted { "joined" } else { "removed" },
                    now
                ],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE groups SET revision = revision + 1, updated_at_ms = ?2
                 WHERE group_id = ?1",
                params![group_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let snapshot = build_group_snapshot(&transaction, &group_id)?;
        enqueue_group_update(
            &transaction,
            profile,
            previous_revision,
            &snapshot,
            if accepted {
                "member_joined"
            } else {
                "invite_rejected"
            },
            now,
        )?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_invite_reply",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_group_update(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        previous_revision: u64,
        snapshot: &GroupSnapshot,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        validate_group_snapshot(snapshot)?;
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let existing = transaction
            .query_row(
                "SELECT owner_device_id, revision FROM groups WHERE group_id = ?1",
                [snapshot.group_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(StorageError::Write)?;
        if let Some((owner, local_revision)) = existing {
            if sender_device_id.to_string() != owner {
                return Err(StorageError::NotGroupOwner);
            }
            let local_revision =
                u64::try_from(local_revision).map_err(|_| StorageError::InvalidStoredId)?;
            if snapshot.revision <= local_revision {
                insert_processed_event(
                    &transaction,
                    sender_device_id,
                    event_id,
                    "group_update",
                    receipt_json,
                    now,
                )?;
                transaction.commit().map_err(StorageError::Write)?;
                return Ok(receipt_json.to_owned());
            }
            if previous_revision != local_revision || snapshot.revision != local_revision + 1 {
                let sync_event_id = EventId::generate();
                let payload = serde_json::json!({
                    "version": 1,
                    "type": "group_sync_request",
                    "event_id": sync_event_id,
                    "sender_device_id": local_device_id,
                    "sent_at_ms": now,
                    "body": {
                        "group_id": snapshot.group_id,
                        "known_revision": local_revision
                    }
                })
                .to_string();
                insert_outbox_event(
                    &transaction,
                    sender_device_id,
                    sync_event_id,
                    "group_sync_request",
                    &payload,
                    now,
                )?;
                insert_processed_event(
                    &transaction,
                    sender_device_id,
                    event_id,
                    "group_update",
                    receipt_json,
                    now,
                )?;
                transaction.commit().map_err(StorageError::Write)?;
                return Ok(receipt_json.to_owned());
            }
        } else {
            let joined = snapshot
                .members
                .iter()
                .any(|member| member.device_id == local_device_id && member.membership == "joined");
            if !joined || sender_device_id != snapshot.owner_device_id || previous_revision != 0 {
                return Err(StorageError::GroupNotFound);
            }
        }
        apply_group_snapshot(&transaction, local_device_id, snapshot, now)?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_update",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_group_sync_request(
        &mut self,
        profile: &LocalProfile,
        sender_device_id: DeviceId,
        event_id: EventId,
        group_id: GroupId,
        known_revision: u64,
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
        let requester_is_member = transaction
            .query_row(
                "SELECT 1 FROM group_members
                 WHERE group_id = ?1 AND device_id = ?2 AND membership = 'joined'",
                params![group_id.to_string(), sender_device_id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(StorageError::Write)?
            .is_some();
        if !requester_is_member {
            return Err(StorageError::GroupNotFound);
        }
        let snapshot = build_group_snapshot(&transaction, &group_id)?;
        let response_event_id = EventId::generate();
        let payload = serde_json::json!({
            "version": 1,
            "type": "group_sync_response",
            "event_id": response_event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "group": snapshot,
                "requested_revision": known_revision
            }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            sender_device_id,
            response_event_id,
            "group_sync_response",
            &payload,
            now,
        )?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_sync_request",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_group_sync_response(
        &mut self,
        local_device_id: DeviceId,
        sender_device_id: DeviceId,
        event_id: EventId,
        requested_revision: u64,
        snapshot: &GroupSnapshot,
        receipt_json: &str,
    ) -> Result<String, StorageError> {
        validate_group_snapshot(snapshot)?;
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
        let (known_owner, local_revision) = transaction
            .query_row(
                "SELECT owner_device_id, revision FROM groups WHERE group_id = ?1",
                [snapshot.group_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        let local_revision =
            u64::try_from(local_revision).map_err(|_| StorageError::InvalidStoredId)?;
        if known_owner != sender_device_id.to_string() || requested_revision > local_revision {
            return Err(StorageError::GroupRevisionConflict);
        }
        if snapshot.revision > local_revision {
            apply_group_snapshot(&transaction, local_device_id, snapshot, now)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_sync_response",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn create_outgoing_group_text(
        &mut self,
        profile: &LocalProfile,
        conversation_id: &str,
        text: &str,
        message_kind: &str,
        client_operation_id: Option<ClientOperationId>,
    ) -> Result<MessageRecord, StorageError> {
        let group_id: GroupId = conversation_id
            .parse()
            .map_err(|_| StorageError::GroupNotFound)?;
        let now = unix_time_ms();
        let message_id = MessageId::generate();
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
        let (revision, state, membership) = transaction
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
        if state != "active" || membership != "joined" {
            return Err(StorageError::GroupNotFound);
        }
        let recipients = joined_group_members(&transaction, &group_id, profile.device_id)?;
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        transaction
            .execute(
                "INSERT INTO messages(message_id, conversation_id, sender_device_id, kind,
                    state, text_content, group_revision, created_at_ms, local_sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    profile.device_id.to_string(),
                    message_kind,
                    if recipients.is_empty() {
                        "delivered"
                    } else {
                        "queued"
                    },
                    text,
                    revision,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        for recipient in &recipients {
            let event_id = EventId::generate();
            let payload = serde_json::json!({
                "version": 1,
                "type": "text_message",
                "event_id": event_id,
                "sender_device_id": profile.device_id,
                "sent_at_ms": now,
                "body": {
                    "message_id": message_id,
                    "conversation_id": group_id,
                    "message_kind": message_kind,
                    "text": text,
                    "created_at_ms": now,
                    "group_revision": revision
                }
            })
            .to_string();
            transaction
                .execute(
                    "INSERT INTO message_deliveries(message_id, recipient_device_id, state,
                        updated_at_ms) VALUES (?1, ?2, 'queued', ?3)",
                    params![message_id.to_string(), recipient.to_string(), now],
                )
                .map_err(StorageError::Write)?;
            insert_outbox_event(
                &transaction,
                *recipient,
                event_id,
                "text_message",
                &payload,
                now,
            )?;
        }
        transaction
            .execute(
                "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3
                 WHERE conversation_id = ?1",
                params![conversation_id, message_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let result = MessageRecord {
            message_id,
            conversation_id: conversation_id.to_owned(),
            sender_device_id: profile.device_id,
            kind: message_kind.to_owned(),
            state: if recipients.is_empty() {
                "delivered"
            } else {
                "queued"
            }
            .to_owned(),
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
            delivery_count: u32::try_from(recipients.len()).unwrap_or(u32::MAX),
        };
        if let Some(operation_id) = client_operation_id {
            save_client_operation(&transaction, operation_id, operation_type, &result, now)?;
        }
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn receive_group_text(
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
        let group_id: GroupId = conversation_id
            .parse()
            .map_err(|_| StorageError::GroupNotFound)?;
        let revision = group_revision.ok_or(StorageError::GroupRevisionConflict)?;
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = saved_receipt(&transaction, sender_device_id, event_id)? {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(saved);
        }
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
        if state != "active" || !local_joined || !sender_joined || revision > local_revision as u64
        {
            return Err(StorageError::GroupRevisionConflict);
        }
        let sort_order = next_sort_order(&transaction, conversation_id)?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO messages(message_id, conversation_id, sender_device_id,
                    kind, state, text_content, group_revision, created_at_ms, received_at_ms,
                    local_sort_order)
                 VALUES (?1, ?2, ?3, ?4, 'delivered', ?5, ?6, ?7, ?8, ?9)",
                params![
                    message_id.to_string(),
                    conversation_id,
                    sender_device_id.to_string(),
                    message_kind,
                    text,
                    i64::try_from(revision).map_err(|_| StorageError::GroupRevisionConflict)?,
                    created_at_ms,
                    now,
                    sort_order
                ],
            )
            .map_err(StorageError::Write)?;
        if inserted > 0 {
            transaction
                .execute(
                    "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3,
                        unread_count = unread_count + 1, deleted_at_ms = NULL
                     WHERE conversation_id = ?1",
                    params![conversation_id, message_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
        }
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "text_message",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }

    pub fn get_group(
        &self,
        local_device_id: DeviceId,
        group_id: GroupId,
    ) -> Result<GroupRecord, StorageError> {
        let snapshot = build_group_snapshot(&self.connection, &group_id)?;
        let (role, membership) = self
            .connection
            .query_row(
                "SELECT role, membership FROM group_members
                 WHERE group_id = ?1 AND device_id = ?2",
                params![group_id.to_string(), local_device_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        Ok(GroupRecord {
            snapshot,
            local_role: role,
            local_membership: membership,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_group(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        group_id: GroupId,
        expected_revision: u64,
        name: Option<&str>,
        add_member_ids: &[DeviceId],
        remove_member_ids: &[DeviceId],
        transfer_owner_to: Option<DeviceId>,
        disband: bool,
    ) -> Result<UpdateGroupResult, StorageError> {
        let action_count = usize::from(name.is_some())
            + usize::from(!add_member_ids.is_empty())
            + usize::from(!remove_member_ids.is_empty())
            + usize::from(transfer_owner_to.is_some())
            + usize::from(disband);
        if action_count != 1 {
            return Err(StorageError::InvalidGroupUpdate);
        }
        if let Some(name) = name {
            validate_group_name(name)?;
        }
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if let Some(saved) = transaction
            .query_row(
                "SELECT result_json FROM client_operations
                 WHERE client_operation_id = ?1 AND operation_type = 'update_group'",
                [client_operation_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
        {
            let result = serde_json::from_str(&saved).map_err(|_| StorageError::InvalidStoredId)?;
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(result);
        }
        let (owner, revision, state) = transaction
            .query_row(
                "SELECT owner_device_id, revision, state FROM groups WHERE group_id = ?1",
                [group_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        if owner != profile.device_id.to_string() {
            return Err(StorageError::NotGroupOwner);
        }
        let revision = u64::try_from(revision).map_err(|_| StorageError::InvalidStoredId)?;
        if state != "active" || revision != expected_revision {
            return Err(StorageError::GroupRevisionConflict);
        }

        let mut extra_update_recipients = Vec::new();
        let mut new_invitees = Vec::new();
        if let Some(name) = name {
            transaction
                .execute(
                    "UPDATE groups SET name = ?2 WHERE group_id = ?1",
                    params![group_id.to_string(), name],
                )
                .map_err(StorageError::Write)?;
        } else if !add_member_ids.is_empty() {
            let unique = add_member_ids.iter().copied().collect::<HashSet<_>>();
            if unique.len() != add_member_ids.len() || unique.contains(&profile.device_id) {
                return Err(StorageError::InvalidGroupMembers);
            }
            let active_count = transaction
                .query_row(
                    "SELECT COUNT(*) FROM group_members
                     WHERE group_id = ?1 AND membership IN ('invited', 'joined')",
                    [group_id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(StorageError::Write)?;
            if usize::try_from(active_count).unwrap_or(MAX_GROUP_MEMBERS) + add_member_ids.len()
                > MAX_GROUP_MEMBERS
            {
                return Err(StorageError::GroupFull);
            }
            for member in add_member_ids {
                let peer_exists = transaction
                    .query_row(
                        "SELECT 1 FROM peers WHERE device_id = ?1",
                        [member.to_string()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(StorageError::Write)?
                    .is_some();
                if !peer_exists {
                    return Err(StorageError::PeerNotFound);
                }
                let already_active = transaction
                    .query_row(
                        "SELECT 1 FROM group_members WHERE group_id = ?1 AND device_id = ?2
                         AND membership IN ('invited', 'joined')",
                        params![group_id.to_string(), member.to_string()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(StorageError::Write)?
                    .is_some();
                if already_active {
                    return Err(StorageError::InvalidGroupMembers);
                }
                transaction
                    .execute(
                        "INSERT INTO group_members(group_id, device_id, role, membership,
                            updated_at_ms) VALUES (?1, ?2, 'member', 'invited', ?3)
                         ON CONFLICT(group_id, device_id) DO UPDATE SET role = 'member',
                            membership = 'invited', joined_at_ms = NULL,
                            updated_at_ms = excluded.updated_at_ms",
                        params![group_id.to_string(), member.to_string(), now],
                    )
                    .map_err(StorageError::Write)?;
                new_invitees.push(*member);
            }
        } else if !remove_member_ids.is_empty() {
            let unique = remove_member_ids.iter().copied().collect::<HashSet<_>>();
            if unique.len() != remove_member_ids.len() || unique.contains(&profile.device_id) {
                return Err(StorageError::InvalidGroupMembers);
            }
            for member in remove_member_ids {
                let changed = transaction
                    .execute(
                        "UPDATE group_members SET membership = 'removed', joined_at_ms = NULL,
                            updated_at_ms = ?3
                         WHERE group_id = ?1 AND device_id = ?2 AND role = 'member'
                           AND membership IN ('invited', 'joined')",
                        params![group_id.to_string(), member.to_string(), now],
                    )
                    .map_err(StorageError::Write)?;
                if changed == 0 {
                    return Err(StorageError::InvalidGroupMembers);
                }
                extra_update_recipients.push(*member);
                transaction
                    .execute(
                        "UPDATE group_invitations SET state = 'cancelled', updated_at_ms = ?3
                         WHERE group_id = ?1 AND peer_device_id = ?2 AND state = 'pending'",
                        params![group_id.to_string(), member.to_string(), now],
                    )
                    .map_err(StorageError::Write)?;
            }
        } else if let Some(new_owner) = transfer_owner_to {
            if new_owner == profile.device_id {
                return Err(StorageError::InvalidGroupMembers);
            }
            let joined = transaction
                .query_row(
                    "SELECT 1 FROM group_members WHERE group_id = ?1 AND device_id = ?2
                     AND role = 'member' AND membership = 'joined'",
                    params![group_id.to_string(), new_owner.to_string()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(StorageError::Write)?
                .is_some();
            if !joined {
                return Err(StorageError::InvalidGroupMembers);
            }
            transaction
                .execute(
                    "UPDATE group_members SET role = CASE WHEN device_id = ?2 THEN 'owner'
                        ELSE 'member' END, updated_at_ms = ?3 WHERE group_id = ?1",
                    params![group_id.to_string(), new_owner.to_string(), now],
                )
                .map_err(StorageError::Write)?;
            transaction
                .execute(
                    "UPDATE groups SET owner_device_id = ?2 WHERE group_id = ?1",
                    params![group_id.to_string(), new_owner.to_string()],
                )
                .map_err(StorageError::Write)?;
        } else if disband {
            transaction
                .execute(
                    "UPDATE groups SET state = 'disbanded', disbanded_at_ms = ?2
                     WHERE group_id = ?1",
                    params![group_id.to_string(), now],
                )
                .map_err(StorageError::Write)?;
            transaction
                .execute(
                    "UPDATE conversations SET state = 'disbanded' WHERE group_id = ?1",
                    [group_id.to_string()],
                )
                .map_err(StorageError::Write)?;
        }
        transaction
            .execute(
                "UPDATE groups SET revision = revision + 1, updated_at_ms = ?2
                 WHERE group_id = ?1",
                params![group_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let snapshot = build_group_snapshot(&transaction, &group_id)?;
        let change_reason = if name.is_some() {
            "profile_changed"
        } else if !new_invitees.is_empty() {
            "members_invited"
        } else if !remove_member_ids.is_empty() {
            "members_removed"
        } else if transfer_owner_to.is_some() {
            "owner_transferred"
        } else {
            "disbanded"
        };
        enqueue_group_update_with_extra(
            &transaction,
            profile,
            revision,
            &snapshot,
            change_reason,
            &extra_update_recipients,
            now,
        )?;
        let snapshot_json = serde_json::to_string(&snapshot)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let mut invite_ids = Vec::new();
        for member in new_invitees {
            let invite_id = InviteId::generate();
            let event_id = EventId::generate();
            let payload = serde_json::json!({
                "version": 1,
                "type": "group_invite",
                "event_id": event_id,
                "sender_device_id": profile.device_id,
                "sent_at_ms": now,
                "body": { "invite_id": invite_id, "group": snapshot }
            })
            .to_string();
            transaction
                .execute(
                    "INSERT INTO group_invitations(invite_id, group_id, peer_device_id,
                        direction, state, snapshot_json, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, 'send', 'pending', ?4, ?5, ?5)",
                    params![
                        invite_id.to_string(),
                        group_id.to_string(),
                        member.to_string(),
                        snapshot_json,
                        now
                    ],
                )
                .map_err(StorageError::Write)?;
            insert_outbox_event(
                &transaction,
                member,
                event_id,
                "group_invite",
                &payload,
                now,
            )?;
            invite_ids.push(invite_id);
        }
        let result = UpdateGroupResult {
            group_id: group_id.clone(),
            revision: snapshot.revision,
            invite_ids,
        };
        let result_json = serde_json::to_string(&result)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO client_operations(client_operation_id, operation_type,
                    result_json, created_at_ms) VALUES (?1, 'update_group', ?2, ?3)",
                params![client_operation_id.to_string(), result_json, now],
            )
            .map_err(StorageError::Write)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(result)
    }

    pub fn leave_group(
        &mut self,
        profile: &LocalProfile,
        client_operation_id: ClientOperationId,
        group_id: GroupId,
    ) -> Result<(), StorageError> {
        let now = unix_time_ms();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::Write)?;
        if saved_client_operation::<()>(&transaction, client_operation_id, "leave_group")?.is_some()
        {
            transaction.commit().map_err(StorageError::Write)?;
            return Ok(());
        }
        let (owner, revision, role, membership) = transaction
            .query_row(
                "SELECT g.owner_device_id, g.revision, gm.role, gm.membership
                 FROM groups g JOIN group_members gm ON gm.group_id = g.group_id
                 WHERE g.group_id = ?1 AND gm.device_id = ?2 AND g.state = 'active'",
                params![group_id.to_string(), profile.device_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        if role == "owner" {
            return Err(StorageError::GroupOwnerMustTransfer);
        }
        if membership != "joined" {
            return Err(StorageError::GroupNotFound);
        }
        let owner: DeviceId = owner.parse().map_err(|_| StorageError::InvalidStoredId)?;
        let event_id = EventId::generate();
        let payload = serde_json::json!({
            "version": 1,
            "type": "group_leave_request",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": { "group_id": group_id, "known_revision": revision }
        })
        .to_string();
        insert_outbox_event(
            &transaction,
            owner,
            event_id,
            "group_leave_request",
            &payload,
            now,
        )?;
        transaction
            .execute(
                "UPDATE group_members SET membership = 'left', updated_at_ms = ?3
                 WHERE group_id = ?1 AND device_id = ?2",
                params![group_id.to_string(), profile.device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        transaction
            .execute(
                "UPDATE conversations SET state = 'left' WHERE group_id = ?1",
                [group_id.to_string()],
            )
            .map_err(StorageError::Write)?;
        save_client_operation(&transaction, client_operation_id, "leave_group", &(), now)?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(())
    }

    pub fn receive_group_leave_request(
        &mut self,
        profile: &LocalProfile,
        sender_device_id: DeviceId,
        event_id: EventId,
        group_id: GroupId,
        known_revision: u64,
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
        let revision = current_group_revision(&transaction, &group_id)?;
        let owner = transaction
            .query_row(
                "SELECT owner_device_id FROM groups WHERE group_id = ?1 AND state = 'active'",
                [group_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .ok_or(StorageError::GroupNotFound)?;
        if owner != profile.device_id.to_string() {
            return Err(StorageError::NotGroupOwner);
        }
        if known_revision > revision {
            return Err(StorageError::GroupRevisionConflict);
        }
        let changed = transaction
            .execute(
                "UPDATE group_members SET membership = 'left', joined_at_ms = NULL,
                    updated_at_ms = ?3 WHERE group_id = ?1 AND device_id = ?2
                    AND role = 'member' AND membership = 'joined'",
                params![group_id.to_string(), sender_device_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        if changed == 0 {
            return Err(StorageError::InvalidGroupMembers);
        }
        transaction
            .execute(
                "UPDATE groups SET revision = revision + 1, updated_at_ms = ?2
                 WHERE group_id = ?1",
                params![group_id.to_string(), now],
            )
            .map_err(StorageError::Write)?;
        let snapshot = build_group_snapshot(&transaction, &group_id)?;
        enqueue_group_update_with_extra(
            &transaction,
            profile,
            revision,
            &snapshot,
            "member_left",
            &[sender_device_id],
            now,
        )?;
        insert_processed_event(
            &transaction,
            sender_device_id,
            event_id,
            "group_leave_request",
            receipt_json,
            now,
        )?;
        transaction.commit().map_err(StorageError::Write)?;
        Ok(receipt_json.to_owned())
    }
}

fn active_group_state() -> String {
    "active".to_owned()
}

fn validate_group_name(name: &str) -> Result<(), StorageError> {
    if !(1..=50).contains(&name.chars().count()) || name.len() > 200 {
        return Err(StorageError::InvalidGroupName);
    }
    Ok(())
}

fn validate_group_snapshot(snapshot: &GroupSnapshot) -> Result<(), StorageError> {
    validate_group_name(&snapshot.name)?;
    if snapshot.revision == 0
        || !(2..=MAX_GROUP_MEMBERS).contains(&snapshot.members.len())
        || !matches!(snapshot.state.as_str(), "active" | "disbanded")
    {
        return Err(StorageError::InvalidGroupMembers);
    }
    let unique = snapshot
        .members
        .iter()
        .map(|member| member.device_id)
        .collect::<HashSet<_>>();
    let owners = snapshot
        .members
        .iter()
        .filter(|member| member.role == "owner")
        .collect::<Vec<_>>();
    if unique.len() != snapshot.members.len()
        || owners.len() != 1
        || owners[0].device_id != snapshot.owner_device_id
        || owners[0].membership != "joined"
        || snapshot.members.iter().any(|member| {
            !matches!(member.role.as_str(), "owner" | "member")
                || !matches!(
                    member.membership.as_str(),
                    "invited" | "joined" | "left" | "removed"
                )
        })
    {
        return Err(StorageError::InvalidGroupMembers);
    }
    Ok(())
}

fn build_group_snapshot(
    connection: &rusqlite::Connection,
    group_id: &GroupId,
) -> Result<GroupSnapshot, StorageError> {
    let (name, owner, revision, state, created_at_ms, disbanded_at_ms) = connection
        .query_row(
            "SELECT name, owner_device_id, revision, state, created_at_ms, disbanded_at_ms
             FROM groups WHERE group_id = ?1",
            [group_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(StorageError::Write)?
        .ok_or(StorageError::GroupNotFound)?;
    let mut statement = connection
        .prepare(
            "SELECT device_id, role, membership FROM group_members
             WHERE group_id = ?1 ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, device_id",
        )
        .map_err(StorageError::Write)?;
    let members = statement
        .query_map([group_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(StorageError::Write)?
        .map(|row| {
            let (device_id, role, membership) = row.map_err(StorageError::Write)?;
            Ok(GroupMemberSnapshot {
                device_id: device_id
                    .parse()
                    .map_err(|_| StorageError::InvalidStoredId)?,
                role,
                membership,
            })
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    let snapshot = GroupSnapshot {
        group_id: group_id.clone(),
        name,
        owner_device_id: owner.parse().map_err(|_| StorageError::InvalidStoredId)?,
        revision: u64::try_from(revision).map_err(|_| StorageError::InvalidStoredId)?,
        created_at_ms,
        state,
        disbanded_at_ms,
        members,
    };
    validate_group_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn apply_group_snapshot(
    transaction: &Transaction<'_>,
    local_device_id: DeviceId,
    snapshot: &GroupSnapshot,
    now: i64,
) -> Result<(), StorageError> {
    validate_group_snapshot(snapshot)?;
    transaction
        .execute(
            "INSERT INTO groups(group_id, name, owner_device_id, revision, state,
                created_at_ms, updated_at_ms, disbanded_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(group_id) DO UPDATE SET name = excluded.name,
                owner_device_id = excluded.owner_device_id, revision = excluded.revision,
                state = excluded.state, updated_at_ms = excluded.updated_at_ms,
                disbanded_at_ms = excluded.disbanded_at_ms",
            params![
                snapshot.group_id.to_string(),
                snapshot.name,
                snapshot.owner_device_id.to_string(),
                i64::try_from(snapshot.revision).map_err(|_| StorageError::InvalidStoredId)?,
                snapshot.state,
                snapshot.created_at_ms,
                now,
                snapshot.disbanded_at_ms
            ],
        )
        .map_err(StorageError::Write)?;
    transaction
        .execute(
            "DELETE FROM group_members WHERE group_id = ?1",
            [snapshot.group_id.to_string()],
        )
        .map_err(StorageError::Write)?;
    for member in &snapshot.members {
        transaction
            .execute(
                "INSERT INTO group_members(group_id, device_id, role, membership,
                    joined_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 = 'joined' THEN ?5 ELSE NULL END, ?5)",
                params![
                    snapshot.group_id.to_string(),
                    member.device_id.to_string(),
                    member.role,
                    member.membership,
                    now
                ],
            )
            .map_err(StorageError::Write)?;
    }
    let local_membership = snapshot
        .members
        .iter()
        .find(|member| member.device_id == local_device_id)
        .map(|member| member.membership.as_str());
    let conversation_state = if snapshot.state == "disbanded" {
        "disbanded"
    } else if local_membership == Some("joined") {
        "active"
    } else {
        "left"
    };
    transaction
        .execute(
            "INSERT INTO conversations(conversation_id, kind, state, group_id, title_cache,
                last_activity_at_ms, unread_count, created_at_ms)
             VALUES (?1, 'group', ?2, ?1, ?3, ?4, 0, ?5)
             ON CONFLICT(conversation_id) DO UPDATE SET state = excluded.state,
                title_cache = excluded.title_cache, deleted_at_ms = NULL",
            params![
                snapshot.group_id.to_string(),
                conversation_state,
                snapshot.name,
                now,
                snapshot.created_at_ms
            ],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

fn current_group_revision(
    transaction: &Transaction<'_>,
    group_id: &GroupId,
) -> Result<u64, StorageError> {
    let revision = transaction
        .query_row(
            "SELECT revision FROM groups WHERE group_id = ?1",
            [group_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(StorageError::Write)?;
    u64::try_from(revision).map_err(|_| StorageError::InvalidStoredId)
}

fn joined_group_members(
    transaction: &Transaction<'_>,
    group_id: &GroupId,
    exclude: DeviceId,
) -> Result<Vec<DeviceId>, StorageError> {
    let mut statement = transaction
        .prepare(
            "SELECT device_id FROM group_members
             WHERE group_id = ?1 AND membership = 'joined' AND device_id != ?2
             ORDER BY device_id",
        )
        .map_err(StorageError::Write)?;
    statement
        .query_map(params![group_id.to_string(), exclude.to_string()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(StorageError::Write)?
        .map(|row| {
            row.map_err(StorageError::Write)?
                .parse()
                .map_err(|_| StorageError::InvalidStoredId)
        })
        .collect()
}

fn enqueue_group_update(
    transaction: &Transaction<'_>,
    profile: &LocalProfile,
    previous_revision: u64,
    snapshot: &GroupSnapshot,
    change_reason: &str,
    now: i64,
) -> Result<(), StorageError> {
    enqueue_group_update_with_extra(
        transaction,
        profile,
        previous_revision,
        snapshot,
        change_reason,
        &[],
        now,
    )
}

fn enqueue_group_update_with_extra(
    transaction: &Transaction<'_>,
    profile: &LocalProfile,
    previous_revision: u64,
    snapshot: &GroupSnapshot,
    change_reason: &str,
    extra_recipients: &[DeviceId],
    now: i64,
) -> Result<(), StorageError> {
    let mut recipients = joined_group_members(transaction, &snapshot.group_id, profile.device_id)?
        .into_iter()
        .collect::<HashSet<_>>();
    recipients.extend(
        extra_recipients
            .iter()
            .copied()
            .filter(|device_id| *device_id != profile.device_id),
    );
    for recipient in recipients {
        let event_id = EventId::generate();
        let payload = serde_json::json!({
            "version": 1,
            "type": "group_update",
            "event_id": event_id,
            "sender_device_id": profile.device_id,
            "sent_at_ms": now,
            "body": {
                "previous_revision": previous_revision,
                "group": snapshot,
                "change_reason": change_reason
            }
        })
        .to_string();
        insert_outbox_event(
            transaction,
            recipient,
            event_id,
            "group_update",
            &payload,
            now,
        )?;
    }
    Ok(())
}

fn insert_group_system_message(
    transaction: &Transaction<'_>,
    sender_device_id: DeviceId,
    group_id: GroupId,
    text: &str,
    now: i64,
) -> Result<(), StorageError> {
    let message_id = MessageId::generate();
    let sort_order = next_sort_order(transaction, &group_id.to_string())?;
    transaction
        .execute(
            "INSERT INTO messages(message_id, conversation_id, sender_device_id, kind,
                state, text_content, created_at_ms, local_sort_order)
             VALUES (?1, ?2, ?3, 'system', 'delivered', ?4, ?5, ?6)",
            params![
                message_id.to_string(),
                group_id.to_string(),
                sender_device_id.to_string(),
                text,
                now,
                sort_order
            ],
        )
        .map_err(StorageError::Write)?;
    transaction
        .execute(
            "UPDATE conversations SET last_message_id = ?2, last_activity_at_ms = ?3
             WHERE conversation_id = ?1",
            params![group_id.to_string(), message_id.to_string(), now],
        )
        .map_err(StorageError::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{EntryId, MessageKind, Platform, TransferEntryKind},
        transfer::{ManifestEntry, TransferManifest},
    };

    fn connect_peers(
        left: &mut Storage,
        left_profile: &LocalProfile,
        right: &mut Storage,
        right_profile: &LocalProfile,
    ) {
        left.upsert_nearby_peer(
            right_profile.device_id,
            &right_profile.device_name,
            right_profile.platform,
            "127.0.0.2",
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

    #[test]
    fn invitation_update_and_group_text_are_transactional_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let mut owner_storage = Storage::open(temp.path().join("owner.db")).unwrap();
        let owner = owner_storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let mut member_storage = Storage::open(temp.path().join("member.db")).unwrap();
        let member = member_storage
            .load_or_create_profile("Member", Platform::Android)
            .unwrap();
        connect_peers(&mut owner_storage, &owner, &mut member_storage, &member);

        let operation_id = ClientOperationId::generate();
        let created = owner_storage
            .create_group(&owner, operation_id, "家庭设备", &[member.device_id])
            .unwrap();
        let duplicate = owner_storage
            .create_group(&owner, operation_id, "不会覆盖", &[member.device_id])
            .unwrap();
        assert_eq!(created, duplicate);
        assert_eq!(created.group_id.local_sequence(), 1);

        let invite_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite")
            .unwrap();
        let invite: serde_json::Value = serde_json::from_str(&invite_outbox.payload_json).unwrap();
        let invite_id: InviteId = invite["body"]["invite_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let snapshot: GroupSnapshot =
            serde_json::from_value(invite["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_invite(
                member.device_id,
                owner.device_id,
                invite_outbox.event_id,
                invite_id,
                &snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(member_storage.list_group_invitations().unwrap().len(), 1);
        owner_storage
            .mark_control_event_stored(member.device_id, invite_outbox.event_id)
            .unwrap();

        let decide_operation = ClientOperationId::generate();
        assert_eq!(
            member_storage
                .decide_group_invite(&member, decide_operation, invite_id, true)
                .unwrap(),
            Some(created.group_id.clone())
        );
        assert_eq!(
            member_storage
                .decide_group_invite(&member, decide_operation, invite_id, false)
                .unwrap(),
            Some(created.group_id.clone())
        );
        assert_eq!(
            member_storage
                .pending_outbox()
                .unwrap()
                .iter()
                .filter(|entry| entry.event_type == "group_invite_reply")
                .count(),
            1
        );
        let reply_outbox = member_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite_reply")
            .unwrap();
        owner_storage
            .receive_group_invite_reply(
                &owner,
                member.device_id,
                reply_outbox.event_id,
                invite_id,
                created.group_id.clone(),
                true,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let owner_group = owner_storage
            .get_group(owner.device_id, created.group_id.clone())
            .unwrap();
        assert_eq!(owner_group.snapshot.revision, 2);
        assert!(
            owner_group.snapshot.members.iter().any(|entry| {
                entry.device_id == member.device_id && entry.membership == "joined"
            })
        );

        let update_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_update")
            .unwrap();
        let update: serde_json::Value = serde_json::from_str(&update_outbox.payload_json).unwrap();
        let update_snapshot: GroupSnapshot =
            serde_json::from_value(update["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_update(
                member.device_id,
                owner.device_id,
                update_outbox.event_id,
                update["body"]["previous_revision"].as_u64().unwrap(),
                &update_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let member_group = member_storage
            .get_group(member.device_id, created.group_id.clone())
            .unwrap();
        assert_eq!(member_group.snapshot, owner_group.snapshot);

        let message_operation = ClientOperationId::generate();
        let outgoing = owner_storage
            .create_outgoing_text_idempotent(
                &owner,
                message_operation,
                &created.conversation_id,
                "群消息",
            )
            .unwrap();
        let repeated = owner_storage
            .create_outgoing_text_idempotent(
                &owner,
                message_operation,
                &created.conversation_id,
                "重复调用不得覆盖",
            )
            .unwrap();
        assert_eq!(repeated, outgoing);
        let queued_deliveries = owner_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(queued_deliveries.len(), 1);
        assert_eq!(queued_deliveries[0].recipient_device_id, member.device_id);
        assert_eq!(queued_deliveries[0].state, "queued");
        assert!(!queued_deliveries[0].delivered);
        let text_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "text_message")
            .unwrap();
        let text: serde_json::Value = serde_json::from_str(&text_outbox.payload_json).unwrap();
        member_storage
            .receive_text(
                member.device_id,
                owner.device_id,
                text_outbox.event_id,
                outgoing.message_id,
                &created.conversation_id,
                "群消息",
                outgoing.created_at_ms,
                text["body"]["group_revision"].as_u64(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        owner_storage
            .mark_text_delivered(member.device_id, text_outbox.event_id, outgoing.message_id)
            .unwrap();
        let delivered = owner_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(delivered[0].state, "stored");
        assert!(delivered[0].delivered);
        assert_eq!(
            owner_storage
                .list_messages(&created.conversation_id, 100)
                .unwrap()
                .last()
                .unwrap()
                .state,
            "delivered"
        );
        assert_eq!(
            member_storage
                .list_messages(&created.conversation_id, 100)
                .unwrap()
                .last()
                .unwrap()
                .text,
            "群消息"
        );

        let clipboard = owner_storage
            .create_outgoing_clipboard_text(&owner, &created.conversation_id, "群剪贴板内容")
            .unwrap();
        let clipboard_message_id = clipboard.message_id.to_string();
        let clipboard_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| {
                if entry.event_type != "text_message" {
                    return false;
                }
                let payload: serde_json::Value = serde_json::from_str(&entry.payload_json).unwrap();
                payload["body"]["message_id"].as_str() == Some(clipboard_message_id.as_str())
            })
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&clipboard_outbox.payload_json).unwrap();
        assert_eq!(payload["body"]["message_kind"], "clipboard_text");
        member_storage
            .receive_text_message(
                member.device_id,
                owner.device_id,
                clipboard_outbox.event_id,
                clipboard.message_id,
                &created.conversation_id,
                "clipboard_text",
                "群剪贴板内容",
                clipboard.created_at_ms,
                payload["body"]["group_revision"].as_u64(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let received = member_storage
            .list_messages(&created.conversation_id, 100)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(received.kind, "clipboard_text");
        assert_eq!(received.text, "群剪贴板内容");
    }

    #[test]
    fn revision_gap_queues_persistent_sync_and_applies_full_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let mut owner_storage = Storage::open(temp.path().join("owner.db")).unwrap();
        let owner = owner_storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let mut member_storage = Storage::open(temp.path().join("member.db")).unwrap();
        let member = member_storage
            .load_or_create_profile("Member", Platform::Android)
            .unwrap();
        connect_peers(&mut owner_storage, &owner, &mut member_storage, &member);

        let created = owner_storage
            .create_group(
                &owner,
                ClientOperationId::generate(),
                "家庭设备",
                &[member.device_id],
            )
            .unwrap();
        let invite_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite")
            .unwrap();
        let invite: serde_json::Value = serde_json::from_str(&invite_outbox.payload_json).unwrap();
        let invite_id: InviteId = invite["body"]["invite_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let invite_snapshot: GroupSnapshot =
            serde_json::from_value(invite["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_invite(
                member.device_id,
                owner.device_id,
                invite_outbox.event_id,
                invite_id,
                &invite_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        owner_storage
            .mark_control_event_stored(member.device_id, invite_outbox.event_id)
            .unwrap();
        member_storage
            .decide_group_invite(&member, ClientOperationId::generate(), invite_id, true)
            .unwrap();
        let reply_outbox = member_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite_reply")
            .unwrap();
        owner_storage
            .receive_group_invite_reply(
                &owner,
                member.device_id,
                reply_outbox.event_id,
                invite_id,
                created.group_id.clone(),
                true,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let joined_update = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_update")
            .unwrap();
        let joined: serde_json::Value = serde_json::from_str(&joined_update.payload_json).unwrap();
        let joined_snapshot: GroupSnapshot =
            serde_json::from_value(joined["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_update(
                member.device_id,
                owner.device_id,
                joined_update.event_id,
                1,
                &joined_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        owner_storage
            .mark_control_event_stored(member.device_id, joined_update.event_id)
            .unwrap();

        owner_storage
            .update_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id.clone(),
                2,
                Some("家庭设备 v3"),
                &[],
                &[],
                None,
                false,
            )
            .unwrap();
        owner_storage
            .update_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id.clone(),
                3,
                Some("家庭设备 v4"),
                &[],
                &[],
                None,
                false,
            )
            .unwrap();
        let skipped_update =
            owner_storage
                .pending_outbox()
                .unwrap()
                .into_iter()
                .filter(|entry| entry.event_type == "group_update")
                .find(|entry| {
                    serde_json::from_str::<serde_json::Value>(&entry.payload_json).unwrap()["body"]
                        ["group"]["revision"]
                        == 4
                })
                .unwrap();
        let skipped: serde_json::Value =
            serde_json::from_str(&skipped_update.payload_json).unwrap();
        let skipped_snapshot: GroupSnapshot =
            serde_json::from_value(skipped["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_update(
                member.device_id,
                owner.device_id,
                skipped_update.event_id,
                3,
                &skipped_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert_eq!(
            member_storage
                .get_group(member.device_id, created.group_id.clone())
                .unwrap()
                .snapshot
                .revision,
            2
        );

        let sync_request = member_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_sync_request")
            .unwrap();
        let request: serde_json::Value = serde_json::from_str(&sync_request.payload_json).unwrap();
        owner_storage
            .receive_group_sync_request(
                &owner,
                member.device_id,
                sync_request.event_id,
                created.group_id.clone(),
                request["body"]["known_revision"].as_u64().unwrap(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let sync_response = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_sync_response")
            .unwrap();
        let response: serde_json::Value =
            serde_json::from_str(&sync_response.payload_json).unwrap();
        let response_snapshot: GroupSnapshot =
            serde_json::from_value(response["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_sync_response(
                member.device_id,
                owner.device_id,
                sync_response.event_id,
                response["body"]["requested_revision"].as_u64().unwrap(),
                &response_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let synchronized = member_storage
            .get_group(member.device_id, created.group_id)
            .unwrap();
        assert_eq!(synchronized.snapshot.revision, 4);
        assert_eq!(synchronized.snapshot.name, "家庭设备 v4");
    }

    #[test]
    fn management_permissions_leave_and_disband_follow_revisions() {
        let temp = tempfile::tempdir().unwrap();
        let mut owner_storage = Storage::open(temp.path().join("owner.db")).unwrap();
        let owner = owner_storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let mut member_storage = Storage::open(temp.path().join("member.db")).unwrap();
        let member = member_storage
            .load_or_create_profile("Member", Platform::Android)
            .unwrap();
        let mut third_storage = Storage::open(temp.path().join("third.db")).unwrap();
        let third = third_storage
            .load_or_create_profile("Third", Platform::Android)
            .unwrap();
        connect_peers(&mut owner_storage, &owner, &mut member_storage, &member);
        owner_storage
            .upsert_nearby_peer(
                third.device_id,
                &third.device_name,
                third.platform,
                "127.0.0.3",
                1,
            )
            .unwrap();

        let created = owner_storage
            .create_group(
                &owner,
                ClientOperationId::generate(),
                "家庭设备",
                &[member.device_id],
            )
            .unwrap();
        let invite_outbox = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite")
            .unwrap();
        let invite: serde_json::Value = serde_json::from_str(&invite_outbox.payload_json).unwrap();
        let invite_id: InviteId = invite["body"]["invite_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let invite_snapshot: GroupSnapshot =
            serde_json::from_value(invite["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_invite(
                member.device_id,
                owner.device_id,
                invite_outbox.event_id,
                invite_id,
                &invite_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        member_storage
            .decide_group_invite(&member, ClientOperationId::generate(), invite_id, true)
            .unwrap();
        let reply_outbox = member_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_invite_reply")
            .unwrap();
        owner_storage
            .receive_group_invite_reply(
                &owner,
                member.device_id,
                reply_outbox.event_id,
                invite_id,
                created.group_id.clone(),
                true,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let joined_update =
            owner_storage
                .pending_outbox()
                .unwrap()
                .into_iter()
                .filter(|entry| entry.event_type == "group_update")
                .find(|entry| {
                    serde_json::from_str::<serde_json::Value>(&entry.payload_json).unwrap()["body"]
                        ["group"]["revision"]
                        == 2
                })
                .unwrap();
        let joined: serde_json::Value = serde_json::from_str(&joined_update.payload_json).unwrap();
        let joined_snapshot: GroupSnapshot =
            serde_json::from_value(joined["body"]["group"].clone()).unwrap();
        member_storage
            .receive_group_update(
                member.device_id,
                owner.device_id,
                joined_update.event_id,
                1,
                &joined_snapshot,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();

        assert!(matches!(
            owner_storage.update_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id.clone(),
                1,
                Some("过期修改"),
                &[],
                &[],
                None,
                false,
            ),
            Err(StorageError::GroupRevisionConflict)
        ));
        assert_eq!(
            owner_storage
                .update_group(
                    &owner,
                    ClientOperationId::generate(),
                    created.group_id.clone(),
                    2,
                    Some("新群名"),
                    &[],
                    &[],
                    None,
                    false,
                )
                .unwrap()
                .revision,
            3
        );
        assert_eq!(
            owner_storage
                .update_group(
                    &owner,
                    ClientOperationId::generate(),
                    created.group_id.clone(),
                    3,
                    None,
                    &[third.device_id],
                    &[],
                    None,
                    false,
                )
                .unwrap()
                .revision,
            4
        );
        assert_eq!(
            owner_storage
                .update_group(
                    &owner,
                    ClientOperationId::generate(),
                    created.group_id.clone(),
                    4,
                    None,
                    &[],
                    &[third.device_id],
                    None,
                    false,
                )
                .unwrap()
                .revision,
            5
        );
        assert_eq!(
            owner_storage
                .update_group(
                    &owner,
                    ClientOperationId::generate(),
                    created.group_id.clone(),
                    5,
                    None,
                    &[],
                    &[],
                    Some(member.device_id),
                    false,
                )
                .unwrap()
                .revision,
            6
        );
        assert!(matches!(
            owner_storage.update_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id.clone(),
                6,
                Some("旧群主不能修改"),
                &[],
                &[],
                None,
                false,
            ),
            Err(StorageError::NotGroupOwner)
        ));

        let transferred = owner_storage
            .get_group(owner.device_id, created.group_id.clone())
            .unwrap()
            .snapshot;
        member_storage
            .receive_group_sync_response(
                member.device_id,
                owner.device_id,
                EventId::generate(),
                2,
                &transferred,
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        assert!(
            member_storage
                .get_group(member.device_id, created.group_id.clone())
                .unwrap()
                .local_role
                == "owner"
        );
        assert!(matches!(
            member_storage.leave_group(
                &member,
                ClientOperationId::generate(),
                created.group_id.clone(),
            ),
            Err(StorageError::GroupOwnerMustTransfer)
        ));

        owner_storage
            .leave_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id.clone(),
            )
            .unwrap();
        let leave_request = owner_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| entry.event_type == "group_leave_request")
            .unwrap();
        let leave: serde_json::Value = serde_json::from_str(&leave_request.payload_json).unwrap();
        member_storage
            .receive_group_leave_request(
                &member,
                owner.device_id,
                leave_request.event_id,
                created.group_id.clone(),
                leave["body"]["known_revision"].as_u64().unwrap(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        let after_leave = member_storage
            .get_group(member.device_id, created.group_id.clone())
            .unwrap();
        assert_eq!(after_leave.snapshot.revision, 7);
        assert!(
            after_leave
                .snapshot
                .members
                .iter()
                .any(|entry| { entry.device_id == owner.device_id && entry.membership == "left" })
        );

        assert_eq!(
            member_storage
                .update_group(
                    &member,
                    ClientOperationId::generate(),
                    created.group_id.clone(),
                    7,
                    None,
                    &[],
                    &[],
                    None,
                    true,
                )
                .unwrap()
                .revision,
            8
        );
        assert_eq!(
            member_storage
                .get_group(member.device_id, created.group_id)
                .unwrap()
                .snapshot
                .state,
            "disbanded"
        );
    }

    #[test]
    fn group_accepts_32_devices_and_rejects_the_33rd() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("groups.db")).unwrap();
        let owner = storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let mut members = Vec::new();
        for index in 0..32 {
            let device_id = DeviceId::generate();
            storage
                .upsert_nearby_peer(
                    device_id,
                    &format!("Device {index}"),
                    Platform::Android,
                    &format!("127.0.0.{}", index + 2),
                    1,
                )
                .unwrap();
            members.push(device_id);
        }
        let created = storage
            .create_group(
                &owner,
                ClientOperationId::generate(),
                "32 台设备",
                &members[..31],
            )
            .unwrap();
        assert_eq!(
            storage
                .get_group(owner.device_id, created.group_id.clone())
                .unwrap()
                .snapshot
                .members
                .len(),
            32
        );
        assert!(matches!(
            storage.update_group(
                &owner,
                ClientOperationId::generate(),
                created.group_id,
                1,
                None,
                &[members[31]],
                &[],
                None,
                false,
            ),
            Err(StorageError::GroupFull)
        ));
    }

    #[test]
    fn group_file_offer_fans_out_one_message_to_independent_transfers() {
        let temp = tempfile::tempdir().unwrap();
        let mut storage = Storage::open(temp.path().join("group-files.db")).unwrap();
        let owner = storage
            .load_or_create_profile("Owner", Platform::Windows)
            .unwrap();
        let first = DeviceId::generate();
        let second = DeviceId::generate();
        for (index, peer) in [first, second].into_iter().enumerate() {
            storage
                .upsert_nearby_peer(
                    peer,
                    &format!("Peer {index}"),
                    Platform::Android,
                    &format!("127.0.0.{}", index + 2),
                    1,
                )
                .unwrap();
        }
        let created = storage
            .create_group(
                &owner,
                ClientOperationId::generate(),
                "文件群",
                &[first, second],
            )
            .unwrap();
        for (invite_id, peer) in created.invite_ids.iter().copied().zip([first, second]) {
            storage
                .receive_group_invite_reply(
                    &owner,
                    peer,
                    EventId::generate(),
                    invite_id,
                    created.group_id.clone(),
                    true,
                    "{\"stage\":\"stored\"}",
                )
                .unwrap();
        }
        let source_entry_id = EntryId::generate();
        let manifest = TransferManifest {
            message_kind: MessageKind::File,
            display_name: "family.bin".to_owned(),
            total_size: 1024,
            entry_count: 1,
            entries: vec![ManifestEntry {
                entry_id: source_entry_id,
                entry_kind: TransferEntryKind::File,
                relative_path: "family.bin".to_owned(),
                size: 1024,
                modified_at_ms: 1,
                source_ref: Some("family.bin".to_owned()),
            }],
        };
        let offer = storage
            .create_outgoing_offer(&owner, &created.conversation_id, &manifest, None, false)
            .unwrap();
        let deliveries = storage.list_message_deliveries(offer.message_id).unwrap();
        assert_eq!(deliveries.len(), 2);
        assert!(deliveries.iter().all(|delivery| {
            delivery.state == "queued"
                && !delivery.delivered
                && delivery.recipient_name.starts_with("Peer ")
        }));
        let transfers = storage
            .list_transfers()
            .unwrap()
            .into_iter()
            .filter(|transfer| transfer.message_id == offer.message_id)
            .collect::<Vec<_>>();
        assert_eq!(transfers.len(), 2);
        assert_ne!(transfers[0].transfer_id, transfers[1].transfer_id);
        let first_entries = storage.get_transfer(transfers[0].transfer_id).unwrap().1;
        let second_entries = storage.get_transfer(transfers[1].transfer_id).unwrap().1;
        assert_ne!(first_entries[0].entry_id, second_entries[0].entry_id);
        assert_ne!(first_entries[0].entry_id, source_entry_id);
        assert_eq!(first_entries[0].source_ref, second_entries[0].source_ref);

        let offers = storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.event_type == "file_offer")
            .collect::<Vec<_>>();
        assert_eq!(offers.len(), 2);
        let payloads = offers
            .iter()
            .map(|entry| serde_json::from_str::<serde_json::Value>(&entry.payload_json).unwrap())
            .collect::<Vec<_>>();
        assert!(payloads.iter().all(|payload| {
            payload["body"]["message_id"] == offer.message_id.to_string()
                && payload["body"]["group_revision"] == 3
        }));
        assert_ne!(
            payloads[0]["body"]["transfer_id"],
            payloads[1]["body"]["transfer_id"]
        );
        assert_eq!(
            storage
                .list_messages(&created.conversation_id, 100)
                .unwrap()
                .into_iter()
                .filter(|message| message.message_id == offer.message_id)
                .count(),
            1
        );
    }
}
