use crate::{CoreHealthState, health_check};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreHealthDto {
    pub status: String,
    pub core_version: String,
    pub protocol_version: u16,
}

#[flutter_rust_bridge::frb(sync)]
pub fn get_core_health() -> CoreHealthDto {
    let health = health_check();
    CoreHealthDto {
        status: match health.status {
            CoreHealthState::Ready => "ready",
            CoreHealthState::Degraded => "degraded",
            CoreHealthState::Failed => "failed",
        }
        .to_owned(),
        core_version: health.core_version,
        protocol_version: health.protocol_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_health_uses_stable_string_values() {
        let health = get_core_health();
        assert_eq!(health.status, "ready");
        assert_eq!(health.protocol_version, 1);
    }
}
