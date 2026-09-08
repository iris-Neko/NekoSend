//! Cross-core protocol and fault-injection tests live in this crate.

pub fn core_version() -> &'static str {
    lan_chat_core::CORE_VERSION
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
        thread,
    };

    use lan_chat_core::{
        domain::{ClientOperationId, EventId, InviteId, Platform},
        storage::{GroupSnapshot, LocalProfile, Storage},
    };
    use perf_harness::{DEFAULT_BUFFER_SIZE, receive_once, send_file};

    fn connect(
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

    #[test]
    fn loopback_transfer_waits_for_receiver_flush_and_preserves_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.bin");
        let destination = temp.path().join("destination.bin");
        let mut source_file = fs::File::create(&source).unwrap();
        for index in 0..(5 * 1024) {
            source_file
                .write_all(&(index as u64).to_le_bytes())
                .unwrap();
        }
        source_file.sync_all().unwrap();

        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = thread::spawn({
            let destination = destination.clone();
            move || receive_once(listener, &destination, DEFAULT_BUFFER_SIZE).unwrap()
        });

        let sent = send_file(address, &source, DEFAULT_BUFFER_SIZE).unwrap();
        let received = receiver.join().unwrap();
        assert_eq!(sent.bytes, received.bytes);
        assert_eq!(fs::read(source).unwrap(), fs::read(destination).unwrap());
    }

    #[test]
    fn group_owner_offline_member_delivery_survives_sender_restart() {
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
            .load_or_create_profile("Member B", Platform::Macos)
            .unwrap();
        let mut member_c_storage = Storage::open(&member_c_path).unwrap();
        let member_c = member_c_storage
            .load_or_create_profile("Member C", Platform::Android)
            .unwrap();
        connect(&mut owner_storage, &owner, &mut member_b_storage, &member_b);
        connect(&mut owner_storage, &owner, &mut member_c_storage, &member_c);
        connect(
            &mut member_b_storage,
            &member_b,
            &mut member_c_storage,
            &member_c,
        );

        let created = owner_storage
            .create_group(
                &owner,
                ClientOperationId::generate(),
                "Family",
                &[member_b.device_id, member_c.device_id],
            )
            .unwrap();
        let mut invitations = Vec::new();
        for target in [member_b.device_id, member_c.device_id] {
            let invite = owner_storage
                .pending_outbox()
                .unwrap()
                .into_iter()
                .find(|entry| entry.event_type == "group_invite" && entry.peer_device_id == target)
                .unwrap();
            let payload: serde_json::Value = serde_json::from_str(&invite.payload_json).unwrap();
            let invite_id: InviteId = payload["body"]["invite_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            let snapshot: GroupSnapshot =
                serde_json::from_value(payload["body"]["group"].clone()).unwrap();
            let target_storage = if target == member_b.device_id {
                &mut member_b_storage
            } else {
                &mut member_c_storage
            };
            target_storage
                .receive_group_invite(
                    target,
                    owner.device_id,
                    invite.event_id,
                    invite_id,
                    &snapshot,
                    "{\"stage\":\"stored\"}",
                )
                .unwrap();
            owner_storage
                .mark_control_event_stored(target, invite.event_id)
                .unwrap();
            invitations.push((target, invite_id));
        }

        for (target, invite_id) in invitations {
            let target_storage = if target == member_b.device_id {
                &mut member_b_storage
            } else {
                &mut member_c_storage
            };
            target_storage
                .decide_group_invite(
                    if target == member_b.device_id {
                        &member_b
                    } else {
                        &member_c
                    },
                    ClientOperationId::generate(),
                    invite_id,
                    true,
                )
                .unwrap();
            let reply = target_storage
                .pending_outbox()
                .unwrap()
                .into_iter()
                .find(|entry| entry.event_type == "group_invite_reply")
                .unwrap();
            owner_storage
                .receive_group_invite_reply(
                    &owner,
                    target,
                    reply.event_id,
                    invite_id,
                    created.group_id.clone(),
                    true,
                    "{\"stage\":\"stored\"}",
                )
                .unwrap();
            target_storage
                .mark_control_event_stored(owner.device_id, reply.event_id)
                .unwrap();
        }

        let final_snapshot = owner_storage
            .get_group(owner.device_id, created.group_id.clone())
            .unwrap()
            .snapshot;
        assert_eq!(final_snapshot.revision, 3);
        for (storage, profile) in [
            (&mut member_b_storage, &member_b),
            (&mut member_c_storage, &member_c),
        ] {
            storage
                .receive_group_sync_response(
                    profile.device_id,
                    owner.device_id,
                    EventId::generate(),
                    1,
                    &final_snapshot,
                    "{\"stage\":\"stored\"}",
                )
                .unwrap();
        }

        let outgoing = member_b_storage
            .create_outgoing_text(&member_b, &created.conversation_id, "owner is offline")
            .unwrap();
        let to_member_c = member_b_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| {
                entry.event_type == "text_message" && entry.peer_device_id == member_c.device_id
            })
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&to_member_c.payload_json).unwrap();
        member_c_storage
            .receive_text_message(
                member_c.device_id,
                member_b.device_id,
                to_member_c.event_id,
                outgoing.message_id,
                &created.conversation_id,
                "text",
                "owner is offline",
                outgoing.created_at_ms,
                payload["body"]["group_revision"].as_u64(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        member_b_storage
            .mark_text_delivered(
                member_c.device_id,
                to_member_c.event_id,
                outgoing.message_id,
            )
            .unwrap();
        let partial = member_b_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(partial.iter().filter(|item| item.delivered).count(), 1);
        assert_eq!(partial.len(), 2);

        drop(member_b_storage);
        let mut member_b_storage = Storage::open(&member_b_path).unwrap();
        let to_owner = member_b_storage
            .pending_outbox()
            .unwrap()
            .into_iter()
            .find(|entry| {
                entry.event_type == "text_message" && entry.peer_device_id == owner.device_id
            })
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&to_owner.payload_json).unwrap();
        owner_storage
            .receive_text_message(
                owner.device_id,
                member_b.device_id,
                to_owner.event_id,
                outgoing.message_id,
                &created.conversation_id,
                "text",
                "owner is offline",
                outgoing.created_at_ms,
                payload["body"]["group_revision"].as_u64(),
                "{\"stage\":\"stored\"}",
            )
            .unwrap();
        member_b_storage
            .mark_text_delivered(owner.device_id, to_owner.event_id, outgoing.message_id)
            .unwrap();
        let completed = member_b_storage
            .list_message_deliveries(outgoing.message_id)
            .unwrap();
        assert_eq!(completed.iter().filter(|item| item.delivered).count(), 2);
        assert!(member_b_storage.pending_outbox().unwrap().is_empty());
    }
}
