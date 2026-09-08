use super::*;

pub const BUILTIN_AVATAR_IDS: [&str; 12] = [
    "cat", "dog", "rabbit", "bird", "fish", "bot", "rocket", "flower", "mountain", "coffee",
    "moon", "sun",
];

pub fn default_avatar_id(device_id: DeviceId) -> &'static str {
    let slot = device_id.as_bytes().iter().fold(0_usize, |hash, byte| {
        (hash * 31 + usize::from(*byte)) % BUILTIN_AVATAR_IDS.len()
    });
    BUILTIN_AVATAR_IDS[slot]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentityRecord {
    pub device_id: String,
    pub device_name: String,
    pub avatar_id: String,
    pub is_local: bool,
}

impl Storage {
    pub fn avatar_id(&self, device_id: DeviceId) -> Result<String, StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT avatar_id FROM device_avatars WHERE device_id = ?1",
                [device_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::Write)?
            .unwrap_or_else(|| default_avatar_id(device_id).to_owned()))
    }

    pub fn store_peer_avatar(
        &self,
        device_id: DeviceId,
        avatar_id: Option<&str>,
    ) -> Result<bool, StorageError> {
        // Missing fields from old clients and unknown future IDs preserve our cache.
        let Some(avatar) = avatar_id.filter(|value| BUILTIN_AVATAR_IDS.contains(value)) else {
            return Ok(false);
        };
        let is_local: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_profile WHERE device_id = ?1)",
                [device_id.to_string()],
                |row| row.get(0),
            )
            .map_err(StorageError::Write)?;
        if is_local {
            return Ok(false);
        }
        self.connection.execute(
            "INSERT INTO device_avatars(device_id, avatar_id, updated_at_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id) DO UPDATE SET avatar_id=excluded.avatar_id, updated_at_ms=excluded.updated_at_ms
             WHERE device_avatars.avatar_id <> excluded.avatar_id",
            params![device_id.to_string(), avatar, unix_time_ms()],
        ).map(|changed| changed != 0).map_err(StorageError::Write)
    }

    pub fn set_local_avatar(
        &mut self,
        operation: ClientOperationId,
        local_id: DeviceId,
        avatar_id: &str,
    ) -> Result<String, StorageError> {
        if !BUILTIN_AVATAR_IDS.contains(&avatar_id) {
            return Err(StorageError::InvalidAvatar);
        }
        run_client_operation(
            &mut self.connection,
            operation,
            "set_device_avatar",
            |transaction, now| {
                transaction.execute(
                "INSERT INTO device_avatars(device_id, avatar_id, updated_at_ms)
                 SELECT device_id, ?2, ?3 FROM local_profile WHERE device_id = ?1
                 ON CONFLICT(device_id) DO UPDATE SET avatar_id=excluded.avatar_id, updated_at_ms=excluded.updated_at_ms",
                params![local_id.to_string(), avatar_id, now],
            ).map_err(StorageError::Write)?;
                Ok(avatar_id.to_owned())
            },
        )
    }

    pub fn list_device_identities(&self) -> Result<Vec<DeviceIdentityRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT p.device_id, p.device_name, a.avatar_id, p.is_local FROM (
                SELECT device_id, device_name, 1 AS is_local FROM local_profile
                UNION ALL
                SELECT device_id, device_name, 0 AS is_local FROM peers
                WHERE device_id NOT IN (SELECT device_id FROM local_profile)
             ) p LEFT JOIN device_avatars a ON a.device_id = p.device_id ORDER BY p.is_local DESC, p.device_id",
        ).map_err(StorageError::Write)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })
            .map_err(StorageError::Write)?;
        rows.map(|row| {
            let (device_id, device_name, avatar, is_local) = row.map_err(StorageError::Write)?;
            let id = device_id
                .parse()
                .map_err(|_| StorageError::InvalidStoredDeviceId)?;
            Ok(DeviceIdentityRecord {
                device_id,
                device_name,
                avatar_id: avatar.unwrap_or_else(|| default_avatar_id(id).to_owned()),
                is_local,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_persist_and_old_peers_cannot_reset_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identities.db");
        let mut storage = Storage::open(&path).unwrap();
        let local = storage
            .load_or_create_profile("My PC", Platform::Windows)
            .unwrap();
        let peer = DeviceId::from_bytes([7; 16]);
        storage
            .upsert_nearby_peer(peer, "Phone", Platform::Android, "127.0.0.1", 1)
            .unwrap();
        let operation = ClientOperationId::generate();
        storage
            .set_local_avatar(operation, local.device_id, "cat")
            .unwrap();
        storage
            .set_local_avatar(operation, local.device_id, "dog")
            .unwrap();
        assert_eq!(storage.avatar_id(local.device_id).unwrap(), "cat");
        assert!(
            !storage
                .store_peer_avatar(local.device_id, Some("dog"))
                .unwrap()
        );
        assert!(
            storage
                .set_local_avatar(ClientOperationId::generate(), local.device_id, "../bad")
                .is_err()
        );
        assert!(storage.store_peer_avatar(peer, Some("rocket")).unwrap());
        assert!(!storage.store_peer_avatar(peer, Some("rocket")).unwrap());
        assert!(!storage.store_peer_avatar(peer, None).unwrap());
        assert!(
            !storage
                .store_peer_avatar(peer, Some("future-avatar"))
                .unwrap()
        );
        drop(storage);
        let restored = Storage::open(&path).unwrap();
        let identities = restored.list_device_identities().unwrap();
        assert_eq!(identities[0].device_name, "My PC");
        assert_eq!(identities[0].avatar_id, "cat");
        assert_eq!(identities[1].device_name, "Phone");
        assert_eq!(identities[1].avatar_id, "rocket");
        assert_eq!(restored.avatar_id(peer).unwrap(), "rocket");
    }

    #[test]
    fn default_avatars_are_stable_and_distributed() {
        let avatars = (0..12)
            .map(|n| {
                let mut bytes = [0; 16];
                bytes[15] = n;
                default_avatar_id(DeviceId::from_bytes(bytes))
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(avatars.len(), 12);
        assert_eq!(default_avatar_id(DeviceId::from_bytes([0; 16])), "cat");
    }
}
