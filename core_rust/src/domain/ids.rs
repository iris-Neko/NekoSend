use std::{fmt, str::FromStr};

use rand::RngCore;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use uuid::Uuid;

macro_rules! impl_string_serde {
    ($name:ident) => {
        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(de::Error::custom)
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind}: {message}")]
pub struct IdParseError {
    kind: &'static str,
    message: &'static str,
}

impl IdParseError {
    fn new(kind: &'static str, message: &'static str) -> Self {
        Self { kind, message }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId([u8; 16]);

impl DeviceId {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 16];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("d_")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for DeviceId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value
            .strip_prefix("d_")
            .ok_or_else(|| IdParseError::new("DeviceId", "missing d_ prefix"))?;
        if hex.len() != 32 {
            return Err(IdParseError::new("DeviceId", "expected 32 hex digits"));
        }
        if !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(IdParseError::new("DeviceId", "hex must be lowercase"));
        }

        let mut bytes = [0_u8; 16];
        for (index, output) in bytes.iter_mut().enumerate() {
            *output = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                .map_err(|_| IdParseError::new("DeviceId", "invalid hex"))?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for DeviceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrivateConversationId {
    first: DeviceId,
    second: DeviceId,
}

impl PrivateConversationId {
    pub fn new(left: DeviceId, right: DeviceId) -> Result<Self, IdParseError> {
        if left == right {
            return Err(IdParseError::new(
                "PrivateConversationId",
                "devices must differ",
            ));
        }
        let (first, second) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        Ok(Self { first, second })
    }

    pub const fn devices(&self) -> (DeviceId, DeviceId) {
        (self.first, self.second)
    }
}

impl fmt::Display for PrivateConversationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "p:{}:{}", self.first, self.second)
    }
}

impl FromStr for PrivateConversationId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let body = value
            .strip_prefix("p:")
            .ok_or_else(|| IdParseError::new("PrivateConversationId", "missing p: prefix"))?;
        let (left, right) = body
            .split_once(':')
            .ok_or_else(|| IdParseError::new("PrivateConversationId", "missing device pair"))?;
        let parsed = Self::new(left.parse()?, right.parse()?)?;
        if parsed.to_string() != value {
            return Err(IdParseError::new(
                "PrivateConversationId",
                "devices are not in canonical order",
            ));
        }
        Ok(parsed)
    }
}

impl_string_serde!(PrivateConversationId);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId {
    creator: DeviceId,
    local_sequence: u64,
}

impl GroupId {
    pub fn new(creator: DeviceId, local_sequence: u64) -> Result<Self, IdParseError> {
        if local_sequence == 0 {
            return Err(IdParseError::new("GroupId", "sequence must be positive"));
        }
        Ok(Self {
            creator,
            local_sequence,
        })
    }

    pub const fn creator(&self) -> DeviceId {
        self.creator
    }

    pub const fn local_sequence(&self) -> u64 {
        self.local_sequence
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "g:{}:{}", self.creator, self.local_sequence)
    }
}

impl FromStr for GroupId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let body = value
            .strip_prefix("g:")
            .ok_or_else(|| IdParseError::new("GroupId", "missing g: prefix"))?;
        let (creator, sequence) = body
            .rsplit_once(':')
            .ok_or_else(|| IdParseError::new("GroupId", "missing sequence"))?;
        let sequence = sequence
            .parse::<u64>()
            .map_err(|_| IdParseError::new("GroupId", "invalid sequence"))?;
        let parsed = Self::new(creator.parse()?, sequence)?;
        if parsed.to_string() != value {
            return Err(IdParseError::new("GroupId", "non-canonical form"));
        }
        Ok(parsed)
    }
}

impl_string_serde!(GroupId);

macro_rules! uuid_v7_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(value)
                    .map_err(|_| IdParseError::new(stringify!($name), "invalid UUID"))?;
                if uuid.get_version_num() != 7 || uuid.to_string() != value {
                    return Err(IdParseError::new(
                        stringify!($name),
                        "expected canonical lowercase UUIDv7",
                    ));
                }
                Ok(Self(uuid))
            }
        }

        impl_string_serde!($name);
    };
}

uuid_v7_id!(MessageId);
uuid_v7_id!(TransferId);
uuid_v7_id!(EntryId);
uuid_v7_id!(EventId);
uuid_v7_id!(InviteId);
uuid_v7_id!(BindingId);
uuid_v7_id!(ClientOperationId);
uuid_v7_id!(PlatformRequestId);

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_A: &str = "d_00000000000000000000000000000001";
    const DEVICE_B: &str = "d_00000000000000000000000000000002";

    #[test]
    fn device_id_round_trips_and_rejects_non_canonical_input() {
        let id: DeviceId = DEVICE_A.parse().unwrap();
        assert_eq!(id.to_string(), DEVICE_A);
        assert!(
            "D_00000000000000000000000000000001"
                .parse::<DeviceId>()
                .is_err()
        );
        assert!(
            "d_ABCDEF00000000000000000000000000"
                .parse::<DeviceId>()
                .is_err()
        );
        assert!("d_01".parse::<DeviceId>().is_err());
    }

    #[test]
    fn private_conversation_is_stably_sorted() {
        let a: DeviceId = DEVICE_A.parse().unwrap();
        let b: DeviceId = DEVICE_B.parse().unwrap();
        let left = PrivateConversationId::new(a, b).unwrap();
        let right = PrivateConversationId::new(b, a).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.to_string(), format!("p:{DEVICE_A}:{DEVICE_B}"));
        assert!(
            format!("p:{DEVICE_B}:{DEVICE_A}")
                .parse::<PrivateConversationId>()
                .is_err()
        );
    }

    #[test]
    fn group_id_is_stable_and_requires_positive_sequence() {
        let creator: DeviceId = DEVICE_A.parse().unwrap();
        let group = GroupId::new(creator, 42).unwrap();
        assert_eq!(group.to_string(), format!("g:{DEVICE_A}:42"));
        assert_eq!(group.to_string().parse::<GroupId>().unwrap(), group);
        assert!(GroupId::new(creator, 0).is_err());
        assert!(format!("g:{DEVICE_A}:042").parse::<GroupId>().is_err());
    }

    #[test]
    fn generated_uuid_ids_are_canonical_v7() {
        let id = MessageId::generate();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert_eq!(id.to_string().parse::<MessageId>().unwrap(), id);
        assert!(Uuid::new_v4().to_string().parse::<MessageId>().is_err());
    }

    #[test]
    fn ids_serialize_as_strings() {
        let id: DeviceId = DEVICE_A.parse().unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{DEVICE_A}\""));
        assert_eq!(serde_json::from_str::<DeviceId>(&json).unwrap(), id);
    }
}
