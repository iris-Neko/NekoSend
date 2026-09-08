use std::{
    collections::{HashMap, HashSet},
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use if_addrs::IfAddr;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use thiserror::Error;

use crate::{
    CORE_VERSION, PROTOCOL_VERSION,
    domain::{DeviceId, EventId, Platform},
    events::{self, CoreEventKind},
    storage::{LocalProfile, Storage, StorageError},
};

pub const DISCOVERY_PORT: u16 = 53_317;
pub const MAX_DISCOVERY_PACKET_BYTES: usize = 1_200;
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(2);
const OFFLINE_AFTER: Duration = Duration::from_secs(7);
const RECEIVE_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyPeer {
    pub device_id: DeviceId,
    pub device_name: String,
    pub platform: Platform,
    pub source_ip: Ipv4Addr,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("failed to bind UDP discovery socket: {0}")]
    Bind(#[source] io::Error),
    #[error("failed to open discovery database: {0}")]
    Storage(#[from] StorageError),
}

pub struct DiscoveryService {
    peers: Arc<Mutex<HashMap<DeviceId, PeerEntry>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DiscoveryService {
    pub fn start(profile: LocalProfile, database_path: &Path) -> Result<Self, DiscoveryError> {
        let socket = create_socket().map_err(DiscoveryError::Bind)?;
        let storage = Storage::open(database_path)?;
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_peers = Arc::clone(&peers);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("lan-chat-discovery".to_owned())
            .spawn(move || run_loop(socket, storage, profile, worker_peers, worker_stop))
            .map_err(DiscoveryError::Bind)?;

        Ok(Self {
            peers,
            stop,
            worker: Some(worker),
        })
    }

    pub fn nearby_peers(&self) -> Vec<NearbyPeer> {
        let now = Instant::now();
        let mut peers = self.peers.lock().unwrap_or_else(|error| error.into_inner());
        peers.retain(|_, entry| now.duration_since(entry.observed_at) <= OFFLINE_AFTER);
        let mut result = peers
            .values()
            .map(|entry| entry.peer.clone())
            .collect::<Vec<_>>();
        result.sort_by(|left, right| {
            right
                .last_seen_at_ms
                .cmp(&left.last_seen_at_ms)
                .then_with(|| left.device_id.cmp(&right.device_id))
        });
        result
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone)]
struct PeerEntry {
    peer: NearbyPeer,
    observed_at: Instant,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiscoverPacket {
    version: u16,
    #[serde(rename = "type")]
    packet_type: String,
    request_id: String,
    sender_device_id: DeviceId,
    sent_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnnouncePacket {
    version: u16,
    #[serde(rename = "type")]
    packet_type: String,
    device_id: DeviceId,
    device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    avatar_id: Option<String>,
    platform: Platform,
    app_version: String,
    protocol_min: u16,
    protocol_max: u16,
    tcp_port: u16,
    capabilities: Vec<String>,
    sent_at_ms: i64,
}

fn create_socket() -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(RECEIVE_TIMEOUT))?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT).into())?;
    Ok(socket.into())
}

fn run_loop(
    socket: UdpSocket,
    storage: Storage,
    profile: LocalProfile,
    peers: Arc<Mutex<HashMap<DeviceId, PeerEntry>>>,
    stop: Arc<AtomicBool>,
) {
    let mut announce = announce_with_avatar(&profile, &storage);
    send_discover(&socket, profile.device_id);
    send_broadcast(&socket, &announce);
    let mut last_announce = Instant::now();
    let mut buffer = [0_u8; MAX_DISCOVERY_PACKET_BYTES + 1];

    while !stop.load(Ordering::Acquire) {
        if last_announce.elapsed() >= ANNOUNCE_INTERVAL {
            announce = announce_with_avatar(&profile, &storage);
            send_discover(&socket, profile.device_id);
            send_broadcast(&socket, &announce);
            last_announce = Instant::now();
        }

        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => handle_packet(
                &socket,
                &storage,
                &profile,
                &peers,
                &announce,
                &buffer[..length],
                source,
            ),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
        expire_offline_peers(&peers);
    }
}

fn handle_packet(
    socket: &UdpSocket,
    storage: &Storage,
    profile: &LocalProfile,
    peers: &Mutex<HashMap<DeviceId, PeerEntry>>,
    own_announce: &[u8],
    bytes: &[u8],
    source: SocketAddr,
) {
    if bytes.is_empty() || bytes.len() > MAX_DISCOVERY_PACKET_BYTES {
        return;
    }
    let source_ip = match source.ip() {
        IpAddr::V4(ip) => ip,
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(ip) => ip,
            None => return,
        },
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("discover") => {
            let Ok(packet) = serde_json::from_value::<DiscoverPacket>(value) else {
                return;
            };
            if packet.version != PROTOCOL_VERSION || packet.sender_device_id == profile.device_id {
                return;
            }
            let destination = SocketAddrV4::new(source_ip, DISCOVERY_PORT);
            let _ = socket.send_to(own_announce, destination);
        }
        Some("announce") => {
            let Ok(packet) = serde_json::from_value::<AnnouncePacket>(value) else {
                return;
            };
            if packet.version != PROTOCOL_VERSION
                || packet.device_id == profile.device_id
                || packet.protocol_min > PROTOCOL_VERSION
                || packet.protocol_max < PROTOCOL_VERSION
                || packet.tcp_port != 53_318
                || !valid_device_name(&packet.device_name)
            {
                return;
            }
            let seen_at_ms = unix_time_ms();
            let peer = NearbyPeer {
                device_id: packet.device_id,
                device_name: packet.device_name,
                platform: packet.platform,
                source_ip,
                last_seen_at_ms: seen_at_ms,
            };
            let _ = storage.upsert_nearby_peer(
                peer.device_id,
                &peer.device_name,
                peer.platform,
                &source_ip.to_string(),
                seen_at_ms,
            );
            let avatar_changed = storage
                .store_peer_avatar(peer.device_id, packet.avatar_id.as_deref())
                .unwrap_or(false);
            let device_id = peer.device_id;
            let changed = {
                let mut peers = peers.lock().unwrap_or_else(|error| error.into_inner());
                let changed = peers.get(&device_id).is_none_or(|entry| {
                    entry.peer.device_name != peer.device_name
                        || entry.peer.platform != peer.platform
                        || entry.peer.source_ip != peer.source_ip
                });
                peers.insert(
                    device_id,
                    PeerEntry {
                        peer,
                        observed_at: Instant::now(),
                    },
                );
                changed
            };
            if changed || avatar_changed {
                events::publish(
                    CoreEventKind::PeerPresenceChanged,
                    Some(device_id.to_string()),
                );
            }
        }
        _ => {}
    }
}

fn expire_offline_peers(peers: &Mutex<HashMap<DeviceId, PeerEntry>>) {
    let now = Instant::now();
    let expired = {
        let mut peers = peers.lock().unwrap_or_else(|error| error.into_inner());
        let expired = peers
            .iter()
            .filter_map(|(device_id, entry)| {
                (now.duration_since(entry.observed_at) > OFFLINE_AFTER).then_some(*device_id)
            })
            .collect::<Vec<_>>();
        for device_id in &expired {
            peers.remove(device_id);
        }
        expired
    };
    for device_id in expired {
        events::publish(
            CoreEventKind::PeerPresenceChanged,
            Some(device_id.to_string()),
        );
    }
}

fn send_discover(socket: &UdpSocket, device_id: DeviceId) {
    let packet = DiscoverPacket {
        version: PROTOCOL_VERSION,
        packet_type: "discover".to_owned(),
        request_id: EventId::generate().to_string(),
        sender_device_id: device_id,
        sent_at_ms: unix_time_ms(),
    };
    if let Ok(bytes) = serde_json::to_vec(&packet) {
        send_broadcast(socket, &bytes);
    }
}

fn announce_bytes(profile: &LocalProfile) -> Vec<u8> {
    serde_json::to_vec(&AnnouncePacket {
        version: PROTOCOL_VERSION,
        packet_type: "announce".to_owned(),
        device_id: profile.device_id,
        device_name: profile.device_name.clone(),
        avatar_id: Some(crate::storage::default_avatar_id(profile.device_id).to_owned()),
        platform: profile.platform,
        app_version: CORE_VERSION.to_owned(),
        protocol_min: PROTOCOL_VERSION,
        protocol_max: PROTOCOL_VERSION,
        tcp_port: 53_318,
        capabilities: vec![
            "text".to_owned(),
            "file".to_owned(),
            "folder".to_owned(),
            "clipboard".to_owned(),
        ],
        sent_at_ms: unix_time_ms(),
    })
    .expect("the fixed announce packet is serializable")
}

fn announce_with_avatar(profile: &LocalProfile, storage: &Storage) -> Vec<u8> {
    let mut packet: AnnouncePacket = serde_json::from_slice(&announce_bytes(profile))
        .expect("locally serialized announce must be valid");
    packet.avatar_id = storage.avatar_id(profile.device_id).ok();
    serde_json::to_vec(&packet).expect("announce with built-in avatar is serializable")
}

fn send_broadcast(socket: &UdpSocket, bytes: &[u8]) {
    if bytes.len() <= MAX_DISCOVERY_PACKET_BYTES {
        for address in broadcast_addresses() {
            let destination = SocketAddrV4::new(address, DISCOVERY_PORT);
            let _ = socket.send_to(bytes, destination);
        }
    }
}

fn broadcast_addresses() -> HashSet<Ipv4Addr> {
    let mut addresses = HashSet::from([Ipv4Addr::BROADCAST]);
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            if !interface.is_oper_up()
                || interface.is_loopback()
                || interface.is_link_local()
                || interface.is_p2p()
            {
                continue;
            }
            if let IfAddr::V4(address) = interface.addr
                && let Some(broadcast) = address.broadcast
            {
                addresses.insert(broadcast);
            }
        }
    }
    addresses
}

fn valid_device_name(name: &str) -> bool {
    (1..=32).contains(&name.chars().count()) && name.len() <= 128
}

fn unix_time_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> LocalProfile {
        LocalProfile {
            device_id: DeviceId::from_bytes([1; 16]),
            device_name: "测试电脑".to_owned(),
            platform: Platform::Windows,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn announce_has_required_protocol_fields_and_fits_limit() {
        let bytes = announce_bytes(&profile());
        assert!(bytes.len() <= MAX_DISCOVERY_PACKET_BYTES);
        let packet: AnnouncePacket = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(packet.packet_type, "announce");
        assert_eq!(packet.protocol_min, PROTOCOL_VERSION);
        assert_eq!(packet.protocol_max, PROTOCOL_VERSION);
        assert_eq!(packet.tcp_port, 53_318);
    }

    #[test]
    fn avatar_changes_are_announced_and_cached_without_overwriting_old_client_data() {
        let temp = tempfile::tempdir().unwrap();
        let mut local_store = Storage::open(temp.path().join("local.db")).unwrap();
        let local = local_store
            .load_or_create_profile("PC", Platform::Windows)
            .unwrap();
        let mut remote_store = Storage::open(temp.path().join("remote.db")).unwrap();
        let remote = remote_store
            .load_or_create_profile("Phone", Platform::Android)
            .unwrap();
        remote_store
            .set_local_avatar(
                crate::domain::ClientOperationId::generate(),
                remote.device_id,
                "rocket",
            )
            .unwrap();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peers = Mutex::new(HashMap::new());
        let packet = announce_with_avatar(&remote, &remote_store);
        assert!(packet.len() <= MAX_DISCOVERY_PACKET_BYTES);
        let source = SocketAddr::from((Ipv4Addr::LOCALHOST, 12345));
        handle_packet(
            &socket,
            &local_store,
            &local,
            &peers,
            &announce_with_avatar(&local, &local_store),
            &packet,
            source,
        );
        assert_eq!(local_store.avatar_id(remote.device_id).unwrap(), "rocket");
        remote_store
            .set_local_avatar(
                crate::domain::ClientOperationId::generate(),
                remote.device_id,
                "moon",
            )
            .unwrap();
        handle_packet(
            &socket,
            &local_store,
            &local,
            &peers,
            &[],
            &announce_with_avatar(&remote, &remote_store),
            source,
        );
        assert_eq!(local_store.avatar_id(remote.device_id).unwrap(), "moon");
        let mut old_packet: serde_json::Value = serde_json::from_slice(&packet).unwrap();
        old_packet.as_object_mut().unwrap().remove("avatar_id");
        handle_packet(
            &socket,
            &local_store,
            &local,
            &peers,
            &[],
            &serde_json::to_vec(&old_packet).unwrap(),
            source,
        );
        assert_eq!(local_store.avatar_id(remote.device_id).unwrap(), "moon");
        expire_offline_peers(&peers);
        assert_eq!(
            local_store
                .list_device_identities()
                .unwrap()
                .iter()
                .find(|item| item.device_id == remote.device_id.to_string())
                .unwrap()
                .device_name,
            "Phone"
        );
    }

    #[test]
    fn device_name_limits_use_characters_and_utf8_bytes() {
        assert!(valid_device_name("手机"));
        assert!(!valid_device_name(""));
        assert!(!valid_device_name(&"a".repeat(33)));
        assert!(!valid_device_name(&"😀".repeat(33)));
    }

    #[test]
    fn oversized_packets_are_ignored_and_source_ip_is_authoritative() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().join("discovery.db")).unwrap();
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peers = Mutex::new(HashMap::new());
        let local = profile();
        let own_announce = announce_bytes(&local);

        handle_packet(
            &socket,
            &storage,
            &local,
            &peers,
            &own_announce,
            &vec![b'x'; MAX_DISCOVERY_PACKET_BYTES + 1],
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 2, 3, 4), 40_000)),
        );
        assert!(peers.lock().unwrap().is_empty());

        let remote = LocalProfile {
            device_id: DeviceId::from_bytes([2; 16]),
            device_name: "手机".to_owned(),
            platform: Platform::Android,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let mut value: serde_json::Value =
            serde_json::from_slice(&announce_bytes(&remote)).unwrap();
        value["source_ip"] = serde_json::json!("203.0.113.9");
        handle_packet(
            &socket,
            &storage,
            &local,
            &peers,
            &own_announce,
            &serde_json::to_vec(&value).unwrap(),
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 2, 3, 4), 40_000)),
        );
        assert_eq!(
            peers.lock().unwrap()[&remote.device_id].peer.source_ip,
            Ipv4Addr::new(10, 2, 3, 4)
        );
    }

    #[test]
    fn stale_peers_expire_without_waiting_for_a_ui_query() {
        let device_id = DeviceId::from_bytes([2; 16]);
        let peers = Mutex::new(HashMap::from([(
            device_id,
            PeerEntry {
                peer: NearbyPeer {
                    device_id,
                    device_name: "手机".to_owned(),
                    platform: Platform::Android,
                    source_ip: Ipv4Addr::LOCALHOST,
                    last_seen_at_ms: 1,
                },
                observed_at: Instant::now() - OFFLINE_AFTER - Duration::from_millis(1),
            },
        )]));

        expire_offline_peers(&peers);

        assert!(peers.lock().unwrap().is_empty());
    }
}
