ALTER TABLE app_settings ADD COLUMN auto_open_receive_directory INTEGER NOT NULL DEFAULT 0
    CHECK (auto_open_receive_directory IN (0,1));

UPDATE schema_meta SET value = '3' WHERE key = 'schema_version';
