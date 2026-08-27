pub mod api;
pub mod domain;
pub mod events;
mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */
pub mod network;
pub mod platform_io;
pub mod protocol;
pub mod storage;
pub mod transfer;

use serde::{Deserialize, Serialize};

pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreHealth {
    pub status: CoreHealthState,
    pub core_version: String,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreHealthState {
    Ready,
    Degraded,
    Failed,
}

pub fn health_check() -> CoreHealth {
    CoreHealth {
        status: CoreHealthState::Ready,
        core_version: CORE_VERSION.to_owned(),
        protocol_version: PROTOCOL_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_check_reports_compile_time_versions() {
        let health = health_check();
        assert_eq!(health.status, CoreHealthState::Ready);
        assert_eq!(health.core_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(health.protocol_version, 1);
    }
}
