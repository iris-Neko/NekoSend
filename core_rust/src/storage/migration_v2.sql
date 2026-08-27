ALTER TABLE transfer_entries RENAME TO transfer_entries_v1;
ALTER TABLE transfers RENAME TO transfers_v1;

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
    entry_count INTEGER NOT NULL CHECK (entry_count BETWEEN 1 AND 20001),
    persisted_bytes INTEGER NOT NULL DEFAULT 0 CHECK (persisted_bytes >= 0),
    receive_base_ref TEXT,
    paused_by_user INTEGER NOT NULL DEFAULT 0 CHECK (paused_by_user IN (0,1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE (message_id, peer_device_id, direction)
) STRICT;

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

INSERT INTO transfers SELECT * FROM transfers_v1;
INSERT INTO transfer_entries SELECT * FROM transfer_entries_v1;

DROP TABLE transfer_entries_v1;
DROP TABLE transfers_v1;

CREATE INDEX idx_transfers_schedulable
    ON transfers(direction, state, updated_at_ms);
CREATE INDEX idx_transfers_peer
    ON transfers(peer_device_id, state);
CREATE INDEX idx_transfer_entries_transfer
    ON transfer_entries(transfer_id, state, relative_path);

UPDATE schema_meta SET value = '2' WHERE key = 'schema_version';
