CREATE TABLE local_profile_v4 (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    device_id TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android', 'linux')),
    group_sequence INTEGER NOT NULL DEFAULT 0 CHECK (group_sequence >= 0),
    clipboard_sequence INTEGER NOT NULL DEFAULT 0 CHECK (clipboard_sequence >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

INSERT INTO local_profile_v4 SELECT * FROM local_profile;
DROP TABLE local_profile;
ALTER TABLE local_profile_v4 RENAME TO local_profile;

CREATE TABLE peers_v4 (
    device_id TEXT PRIMARY KEY,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android', 'linux')),
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

INSERT INTO peers_v4 SELECT * FROM peers;
DROP TABLE peers;
ALTER TABLE peers_v4 RENAME TO peers;

CREATE INDEX idx_peers_last_seen ON peers(last_seen_at_ms DESC);
CREATE INDEX idx_peers_relation ON peers(relation);
UPDATE schema_meta SET value = '4' WHERE key = 'schema_version';
