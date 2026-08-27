CREATE TABLE schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

INSERT INTO schema_meta(key, value) VALUES ('schema_version', '1');

CREATE TABLE local_profile (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    device_id TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android')),
    group_sequence INTEGER NOT NULL DEFAULT 0 CHECK (group_sequence >= 0),
    clipboard_sequence INTEGER NOT NULL DEFAULT 0 CHECK (clipboard_sequence >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE app_settings (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    default_receive_policy TEXT NOT NULL DEFAULT 'auto_accept'
        CHECK (default_receive_policy IN ('auto_accept', 'ask_every_time')),
    default_receive_ref TEXT,
    notifications_enabled INTEGER NOT NULL DEFAULT 1 CHECK (notifications_enabled IN (0,1)),
    close_to_tray INTEGER NOT NULL DEFAULT 1 CHECK (close_to_tray IN (0,1)),
    start_on_boot INTEGER NOT NULL DEFAULT 0 CHECK (start_on_boot IN (0,1)),
    android_keep_online INTEGER NOT NULL DEFAULT 1 CHECK (android_keep_online IN (0,1)),
    log_level TEXT NOT NULL DEFAULT 'normal' CHECK (log_level IN ('normal', 'debug')),
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE peers (
    device_id TEXT PRIMARY KEY,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android')),
    relation TEXT NOT NULL DEFAULT 'nearby'
        CHECK (relation IN ('nearby', 'known', 'own_device')),
    receive_policy_override TEXT
        CHECK (receive_policy_override IS NULL OR receive_policy_override IN ('auto_accept', 'ask_every_time')),
    last_ip TEXT,
    last_seen_at_ms INTEGER,
    known_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_peers_last_seen ON peers(last_seen_at_ms DESC);
CREATE INDEX idx_peers_relation ON peers(relation);

CREATE TABLE own_device_bindings (
    peer_device_id TEXT PRIMARY KEY REFERENCES peers(device_id) ON DELETE CASCADE,
    binding_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL
        CHECK (state IN ('pending_outbound', 'pending_inbound', 'active', 'rejected', 'removed')),
    clipboard_mode TEXT NOT NULL DEFAULT 'off'
        CHECK (clipboard_mode IN ('off', 'send_only', 'receive_only', 'bidirectional')),
    requested_by_device_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 50),
    owner_device_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'disbanded')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    disbanded_at_ms INTEGER
) STRICT;

CREATE TABLE group_members (
    group_id TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    membership TEXT NOT NULL CHECK (membership IN ('invited', 'joined', 'left', 'removed')),
    joined_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (group_id, device_id)
) STRICT;

CREATE INDEX idx_group_members_device ON group_members(device_id, membership);

CREATE TABLE conversations (
    conversation_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('private', 'group')),
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'left', 'disbanded')),
    peer_device_id TEXT REFERENCES peers(device_id),
    group_id TEXT REFERENCES groups(group_id),
    title_cache TEXT NOT NULL,
    last_message_id TEXT,
    last_activity_at_ms INTEGER NOT NULL,
    unread_count INTEGER NOT NULL DEFAULT 0 CHECK (unread_count >= 0),
    created_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    CHECK (
      (kind = 'private' AND peer_device_id IS NOT NULL AND group_id IS NULL) OR
      (kind = 'group' AND group_id IS NOT NULL AND peer_device_id IS NULL)
    )
) STRICT;

CREATE INDEX idx_conversations_activity
    ON conversations(deleted_at_ms, last_activity_at_ms DESC);

CREATE TABLE messages (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_device_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('text', 'file', 'image', 'folder', 'clipboard_text', 'clipboard_image', 'system')),
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'sending', 'partially_delivered', 'delivered', 'failed', 'cancelled')),
    text_content TEXT,
    display_name TEXT,
    total_size INTEGER CHECK (total_size IS NULL OR total_size >= 0),
    entry_count INTEGER CHECK (entry_count IS NULL OR entry_count >= 0),
    group_revision INTEGER,
    created_at_ms INTEGER NOT NULL,
    received_at_ms INTEGER,
    local_sort_order INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    CHECK (
      (kind IN ('text', 'clipboard_text', 'system') AND text_content IS NOT NULL) OR
      (kind IN ('file', 'image', 'folder', 'clipboard_image') AND display_name IS NOT NULL)
    )
) STRICT;

CREATE UNIQUE INDEX idx_messages_conversation_sort
    ON messages(conversation_id, local_sort_order);
CREATE INDEX idx_messages_conversation_time
    ON messages(conversation_id, deleted_at_ms, local_sort_order DESC);

CREATE TABLE message_deliveries (
    message_id TEXT NOT NULL REFERENCES messages(message_id) ON DELETE CASCADE,
    recipient_device_id TEXT NOT NULL,
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'sending', 'stored', 'accepted', 'rejected', 'completed', 'failed', 'cancelled')),
    last_event_id TEXT,
    failure_reason TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (message_id, recipient_device_id)
) STRICT;

CREATE INDEX idx_deliveries_recipient_state
    ON message_deliveries(recipient_device_id, state, updated_at_ms);

CREATE TABLE transfers (
    transfer_id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(message_id) ON DELETE CASCADE,
    peer_device_id TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('send', 'receive')),
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'offered', 'accepted', 'transferring', 'paused', 'verifying', 'completed', 'failed', 'cancelled')),
    failure_reason TEXT
        CHECK (failure_reason IS NULL OR failure_reason IN (
          'peer_offline', 'rejected', 'source_changed', 'not_enough_space',
          'connection_error', 'invalid_path', 'permission_lost',
          'unsupported', 'user_cancelled'
        )),
    display_name TEXT NOT NULL,
    total_size INTEGER NOT NULL CHECK (total_size >= 0),
    entry_count INTEGER NOT NULL CHECK (entry_count BETWEEN 1 AND 10000),
    persisted_bytes INTEGER NOT NULL DEFAULT 0 CHECK (persisted_bytes >= 0),
    receive_base_ref TEXT,
    paused_by_user INTEGER NOT NULL DEFAULT 0 CHECK (paused_by_user IN (0,1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE (message_id, peer_device_id, direction)
) STRICT;

CREATE INDEX idx_transfers_schedulable
    ON transfers(direction, state, updated_at_ms);
CREATE INDEX idx_transfers_peer
    ON transfers(peer_device_id, state);

CREATE TABLE transfer_entries (
    entry_id TEXT PRIMARY KEY,
    transfer_id TEXT NOT NULL REFERENCES transfers(transfer_id) ON DELETE CASCADE,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('file', 'directory')),
    relative_path TEXT NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    modified_at_ms INTEGER NOT NULL,
    source_ref TEXT,
    destination_ref TEXT,
    partial_ref TEXT,
    final_display_name TEXT,
    persisted_offset INTEGER NOT NULL DEFAULT 0 CHECK (persisted_offset >= 0),
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'transferring', 'completed', 'failed', 'cancelled')),
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (transfer_id, relative_path),
    CHECK (persisted_offset <= size),
    CHECK (entry_kind = 'file' OR size = 0)
) STRICT;

CREATE INDEX idx_transfer_entries_transfer
    ON transfer_entries(transfer_id, state, relative_path);

CREATE TABLE group_invitations (
    invite_id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    peer_device_id TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('send', 'receive')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'accepted', 'rejected', 'cancelled')),
    snapshot_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (group_id, peer_device_id, direction, invite_id)
) STRICT;

CREATE TABLE outbox (
    outbox_id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_device_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    requires_receipt INTEGER NOT NULL CHECK (requires_receipt IN (0,1)),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'in_flight')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (peer_device_id, event_id)
) STRICT;

CREATE INDEX idx_outbox_due
    ON outbox(peer_device_id, state, next_attempt_at_ms, outbox_id);

CREATE TABLE processed_events (
    sender_device_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    receipt_json TEXT,
    processed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (sender_device_id, event_id)
) STRICT;

CREATE INDEX idx_processed_events_time ON processed_events(processed_at_ms);

CREATE TABLE clipboard_dedup (
    origin_device_id TEXT NOT NULL,
    clipboard_sequence INTEGER NOT NULL CHECK (clipboard_sequence >= 0),
    message_id TEXT NOT NULL,
    content_fingerprint TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (origin_device_id, clipboard_sequence)
) STRICT;

CREATE INDEX idx_clipboard_dedup_time ON clipboard_dedup(created_at_ms);

CREATE TABLE client_operations (
    client_operation_id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    result_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;
