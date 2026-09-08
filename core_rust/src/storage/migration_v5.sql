CREATE TABLE device_avatars (
    device_id TEXT PRIMARY KEY,
    avatar_id TEXT NOT NULL CHECK (avatar_id IN (
        'cat', 'dog', 'rabbit', 'bird', 'fish', 'bot',
        'rocket', 'flower', 'mountain', 'coffee', 'moon', 'sun'
    )),
    updated_at_ms INTEGER NOT NULL
) STRICT;

UPDATE schema_meta SET value = '5' WHERE key = 'schema_version';
