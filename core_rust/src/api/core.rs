use std::{path::Path, str::FromStr, sync::Mutex};

use flutter_rust_bridge::frb;

use crate::{
    domain::{
        BindingId, ClientOperationId, DeviceId, EntryId, GroupId, InviteId, MessageId, MessageKind,
        Platform, TransferEntryKind, TransferId,
    },
    network::{ControlService, DiscoveryService, stop_active_transfer, stop_all_active_transfers},
    platform_io::{PlatformRequestDto, complete_platform_request as complete_platform_io_request},
    storage::{
        ConversationRecord, LocalProfile, MessageDeliveryRecord, MessageRecord, Storage,
        TransferRecord,
    },
    transfer::{
        PreparedReceiveEntry, SourceItem, build_manifest, enumerate_path, prepare_receive_paths,
    },
};
use crate::{
    events::{self, CoreEventDto},
    frb_generated::StreamSink,
};

pub fn subscribe_core_events(sink: StreamSink<CoreEventDto>) {
    events::subscribe(sink);
}

static CORE_RUNTIME: Mutex<Option<CoreRuntime>> = Mutex::new(None);

struct CoreRuntime {
    profile: LocalProfile,
    database_path: String,
    discovery: Option<DiscoveryService>,
    control: Option<ControlService>,
}

fn start_network_pair<C, D>(
    start_control: impl FnOnce() -> Result<C, String>,
    start_discovery: impl FnOnce() -> Result<D, String>,
) -> Result<(C, Option<D>, Option<String>), String> {
    let control = start_control()?;
    Ok(match start_discovery() {
        Ok(discovery) => (control, Some(discovery), None),
        Err(error) => (control, None, Some(error)),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartCoreDto {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub discovery_available: bool,
    pub discovery_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreErrorDto {
    pub code: String,
    pub message: String,
    pub user_message: String,
    pub recoverable: bool,
    pub related_id: Option<String>,
}

#[frb(sync)]
pub fn describe_core_error(message: String) -> CoreErrorDto {
    let normalized = message.to_ascii_lowercase();
    let (code, user_message, recoverable) = if normalized.contains("application data directory") {
        ("CONFIG_DIRECTORY_UNWRITABLE", "应用数据目录不可写", true)
    } else if normalized.contains("failed to open database") {
        ("STORAGE_OPEN_FAILED", "无法打开本地数据库", true)
    } else if normalized.contains("database migration") || normalized.contains("schema version") {
        ("STORAGE_MIGRATION_FAILED", "本地数据升级失败", true)
    } else if normalized.contains("database write")
        || normalized.contains("failed to serialize persisted")
    {
        ("STORAGE_WRITE_FAILED", "无法保存本地状态", true)
    } else if normalized.contains("failed to bind udp") {
        ("NETWORK_UDP_BIND_FAILED", "无法启动附近设备发现", true)
    } else if normalized.contains("failed to bind tcp") {
        ("NETWORK_TCP_BIND_FAILED", "无法启动接收服务", true)
    } else if normalized.contains("unsupported protocol")
        || normalized.contains("no mutually supported protocol")
    {
        ("UNSUPPORTED_PROTOCOL", "对方应用版本不兼容", false)
    } else if normalized.contains("frame length") || normalized.contains("limit exceeded") {
        ("PROTOCOL_LIMIT_EXCEEDED", "发送内容超过协议限制", false)
    } else if normalized.contains("invalid json")
        || normalized.contains("invalid utf-8")
        || normalized.contains("protocol validation")
    {
        ("PROTOCOL_INVALID_FRAME", "收到无法识别的数据", false)
    } else if normalized.contains("insufficient free space")
        || normalized.contains("not_enough_space")
    {
        ("FILE_NOT_ENOUGH_SPACE", "存储空间不足", true)
    } else if normalized.contains("permission was lost")
        || normalized.contains("permission_lost")
        || normalized.contains("permission denied")
    {
        (
            "FILE_PERMISSION_LOST",
            "文件访问权限已失效，请重新选择",
            true,
        )
    } else if normalized.contains("source file changed")
        || normalized.contains("replacement sources do not match")
        || normalized.contains("source_changed")
    {
        ("TRANSFER_SOURCE_CHANGED", "源文件已变化，请重新选择", true)
    } else if normalized.contains("source does not exist") || normalized.contains("file not found")
    {
        ("FILE_NOT_FOUND", "找不到源文件", true)
    } else if normalized.contains("invalid relative path")
        || normalized.contains("receive path cannot")
    {
        ("FILE_INVALID_PATH", "文件路径无效", false)
    } else if normalized.contains("failed to open file") {
        ("FILE_OPEN_FAILED", "无法打开文件", true)
    } else if normalized.contains("failed to prepare receive")
        || normalized.contains("receive directory is unavailable")
        || normalized.contains("failed to write")
    {
        ("FILE_WRITE_FAILED", "无法写入接收文件", true)
    } else if normalized.contains("group already contains 32") {
        ("GROUP_FULL", "群成员已达到 32 台设备", false)
    } else if normalized.contains("group owner must transfer") {
        (
            "GROUP_OWNER_MUST_TRANSFER",
            "群主需要先转让群主或解散群聊",
            true,
        )
    } else if normalized.contains("only the current group owner") {
        ("NOT_GROUP_OWNER", "只有群主可以执行此操作", false)
    } else if normalized.contains("group revision") {
        (
            "GROUP_REVISION_CONFLICT",
            "群资料正在同步，请稍后重试",
            true,
        )
    } else if normalized.contains("group does not exist") {
        ("GROUP_NOT_FOUND", "群聊不存在", false)
    } else if normalized.contains("automatic clipboard sync requires") {
        (
            "CLIPBOARD_BINDING_REQUIRED",
            "请先将对方设为“我的设备”",
            true,
        )
    } else if normalized.contains("clipboard text must") || normalized.contains("clipboard image") {
        (
            "CLIPBOARD_TOO_LARGE",
            "剪贴板内容过大，请改用文件发送",
            false,
        )
    } else if normalized.contains("notification permission") {
        (
            "NOTIFICATION_PERMISSION_REQUIRED",
            "需要通知权限才能继续后台传输",
            true,
        )
    } else if normalized.contains("peer or private conversation") {
        ("PEER_UNKNOWN", "找不到该设备", true)
    } else if normalized.contains("conversation does not exist")
        || normalized.contains("invitation does not exist")
        || normalized.contains("binding does not exist")
        || normalized.contains("transfer does not exist")
    {
        ("NOT_FOUND", "找不到对应记录", false)
    } else if normalized.contains("text message must") {
        ("INVALID_ARGUMENT", "消息必须为 1 至 20000 个字符", false)
    } else if normalized.contains("not ready") || normalized.contains("not started") {
        ("NOT_READY", "服务仍在启动，请稍后重试", true)
    } else if normalized.contains("connection") || normalized.contains("peer offline") {
        ("NETWORK_CONNECTION_LOST", "连接已中断，将自动重试", true)
    } else if normalized.contains("invalid")
        || normalized.contains("must contain")
        || normalized.contains("must be")
        || normalized.contains("requires")
    {
        ("INVALID_ARGUMENT", "输入内容不正确", false)
    } else {
        ("INTERNAL_ERROR", "发生内部错误，请重试", true)
    };
    CoreErrorDto {
        code: code.to_owned(),
        message: message.chars().take(512).collect(),
        user_message: user_message.to_owned(),
        recoverable,
        related_id: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettingsDto {
    pub default_receive_policy: String,
    pub default_receive_ref: Option<String>,
    pub notifications_enabled: bool,
    pub close_to_tray: bool,
    pub start_on_boot: bool,
    pub android_keep_online: bool,
    pub auto_open_receive_directory: bool,
    pub log_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerReceivePolicyDto {
    pub device_id: String,
    pub device_name: String,
    pub relation: String,
    pub receive_policy_override: Option<String>,
    pub effective_receive_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyPeerDto {
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub source_ip: String,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationDto {
    pub conversation_id: String,
    pub title: String,
    pub peer_device_id: Option<String>,
    pub group_id: Option<String>,
    pub is_group: bool,
    pub member_count: u32,
    pub online_count: u32,
    pub last_message_preview: Option<String>,
    pub last_activity_at_ms: i64,
    pub unread_count: u32,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDto {
    pub message_id: String,
    pub conversation_id: String,
    pub sender_device_id: String,
    pub kind: String,
    pub state: String,
    pub text: String,
    pub total_size: Option<u64>,
    pub entry_count: Option<u32>,
    pub transfer_id: Option<String>,
    pub transfer_state: Option<String>,
    pub persisted_bytes: Option<u64>,
    pub local_file_ref: Option<String>,
    pub created_at_ms: i64,
    pub local_sort_order: i64,
    pub delivered_count: u32,
    pub delivery_count: u32,
    pub outgoing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDeliveryDto {
    pub recipient_device_id: String,
    pub recipient_name: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub updated_at_ms: i64,
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendSourceDto {
    pub message_id: String,
    pub transfer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItemDto {
    pub entry_kind: String,
    pub source_ref: Option<String>,
    pub relative_path: String,
    pub size: u64,
    pub modified_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEntryDto {
    pub entry_id: String,
    pub entry_kind: String,
    pub relative_path: String,
    pub size: u64,
    pub persisted_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedReceiveEntryDto {
    pub entry_id: String,
    pub destination_ref: String,
    pub partial_ref: Option<String>,
    pub persisted_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDto {
    pub transfer_id: String,
    pub message_id: String,
    pub conversation_id: String,
    pub peer_device_id: String,
    pub direction: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub display_name: String,
    pub total_size: u64,
    pub entry_count: u32,
    pub persisted_bytes: u64,
    pub receive_base_ref: Option<String>,
    pub local_file_ref: Option<String>,
    pub paused_by_user: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGroupDto {
    pub group_id: String,
    pub conversation_id: String,
    pub invite_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInvitationDto {
    pub invite_id: String,
    pub group_id: String,
    pub group_name: String,
    pub inviter_device_id: String,
    pub inviter_name: String,
    pub member_device_ids: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMemberDto {
    pub device_id: String,
    pub role: String,
    pub membership: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDto {
    pub group_id: String,
    pub name: String,
    pub owner_device_id: String,
    pub revision: u64,
    pub state: String,
    pub local_role: String,
    pub local_membership: String,
    pub members: Vec<GroupMemberDto>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateGroupDto {
    pub group_id: String,
    pub revision: u64,
    pub invite_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnDeviceBindingDto {
    pub peer_device_id: String,
    pub peer_name: String,
    pub binding_id: String,
    pub state: String,
    pub clipboard_mode: String,
    pub requested_by_device_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSendDto {
    pub message_id: String,
    pub event_id: String,
    pub clipboard_sequence: u64,
}

#[frb(sync)]
pub fn generate_client_operation_id() -> String {
    ClientOperationId::generate().to_string()
}

#[frb(sync)]
pub fn start_core(
    database_path: String,
    device_name: String,
    platform: String,
    enable_discovery: bool,
) -> Result<StartCoreDto, String> {
    shutdown_core();
    let platform = parse_platform(&platform)?;
    let mut storage = Storage::open(&database_path).map_err(|error| error.to_string())?;
    let profile = storage
        .load_or_create_profile(&device_name, platform)
        .map_err(|error| error.to_string())?;
    storage
        .reconcile_interrupted_transfers(&profile)
        .map_err(|error| error.to_string())?;
    drop(storage);

    let (discovery, discovery_error) = if enable_discovery {
        match DiscoveryService::start(profile.clone(), Path::new(&database_path)) {
            Ok(service) => (Some(service), None),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };
    let discovery_available = discovery.is_some();
    let control = if enable_discovery {
        Some(
            ControlService::start(profile.clone(), Path::new(&database_path))
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };

    let mut runtime = CORE_RUNTIME
        .lock()
        .map_err(|_| "core runtime lock is poisoned".to_owned())?;
    *runtime = Some(CoreRuntime {
        profile: profile.clone(),
        database_path,
        discovery,
        control,
    });

    Ok(StartCoreDto {
        device_id: profile.device_id.to_string(),
        device_name: profile.device_name,
        platform: platform_name(profile.platform).to_owned(),
        discovery_available,
        discovery_error,
    })
}

#[frb(sync)]
pub fn open_private_conversation(
    client_operation_id: String,
    peer_device_id: String,
) -> Result<ConversationDto, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer = DeviceId::from_str(&peer_device_id).map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let record = storage
            .open_private_conversation(operation_id, runtime.profile.device_id, peer)
            .map_err(|error| error.to_string())?;
        Ok(conversation_dto(runtime, record))
    })
}

#[frb(sync)]
pub fn create_group(
    client_operation_id: String,
    name: String,
    member_device_ids: Vec<String>,
) -> Result<CreateGroupDto, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let members = member_device_ids
            .into_iter()
            .map(|value| value.parse::<DeviceId>().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let result = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .create_group(&runtime.profile, operation_id, &name, &members)
            .map_err(|error| error.to_string())?;
        Ok(CreateGroupDto {
            group_id: result.group_id.to_string(),
            conversation_id: result.conversation_id,
            invite_ids: result
                .invite_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        })
    })
}

#[frb(sync)]
pub fn list_group_invitations() -> Result<Vec<GroupInvitationDto>, String> {
    with_runtime(|runtime| {
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .list_group_invitations()
            .map_err(|error| error.to_string())
            .map(|invitations| {
                invitations
                    .into_iter()
                    .map(|invitation| GroupInvitationDto {
                        invite_id: invitation.invite_id.to_string(),
                        group_id: invitation.group_id.to_string(),
                        group_name: invitation.group_name,
                        inviter_device_id: invitation.inviter_device_id.to_string(),
                        inviter_name: invitation.inviter_name,
                        member_device_ids: invitation
                            .member_device_ids
                            .into_iter()
                            .map(|id| id.to_string())
                            .collect(),
                        created_at_ms: invitation.created_at_ms,
                    })
                    .collect()
            })
    })
}

#[frb(sync)]
pub fn decide_group_invite(
    client_operation_id: String,
    invite_id: String,
    accept: bool,
) -> Result<Option<String>, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let invite_id = invite_id
            .parse::<InviteId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .decide_group_invite(&runtime.profile, operation_id, invite_id, accept)
            .map_err(|error| error.to_string())
            .map(|group| group.map(|id| id.to_string()))
    })
}

#[frb(sync)]
pub fn get_group(group_id: String) -> Result<GroupDto, String> {
    with_runtime(|runtime| {
        let group_id = group_id
            .parse::<GroupId>()
            .map_err(|error| error.to_string())?;
        let group = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .get_group(runtime.profile.device_id, group_id)
            .map_err(|error| error.to_string())?;
        let nearby = runtime
            .discovery
            .as_ref()
            .map(DiscoveryService::nearby_peers)
            .unwrap_or_default();
        Ok(GroupDto {
            group_id: group.snapshot.group_id.to_string(),
            name: group.snapshot.name,
            owner_device_id: group.snapshot.owner_device_id.to_string(),
            revision: group.snapshot.revision,
            state: group.snapshot.state,
            local_role: group.local_role,
            local_membership: group.local_membership,
            members: group
                .snapshot
                .members
                .into_iter()
                .map(|member| GroupMemberDto {
                    device_id: member.device_id.to_string(),
                    role: member.role,
                    membership: member.membership,
                    online: member.device_id == runtime.profile.device_id
                        || nearby.iter().any(|peer| peer.device_id == member.device_id),
                })
                .collect(),
        })
    })
}

#[frb(sync)]
#[allow(clippy::too_many_arguments)]
pub fn update_group(
    client_operation_id: String,
    group_id: String,
    expected_revision: u64,
    name: Option<String>,
    add_member_ids: Vec<String>,
    remove_member_ids: Vec<String>,
    transfer_owner_to: Option<String>,
    disband: bool,
) -> Result<UpdateGroupDto, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let group_id = group_id
            .parse::<GroupId>()
            .map_err(|error| error.to_string())?;
        let add_members = add_member_ids
            .into_iter()
            .map(|id| id.parse::<DeviceId>().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let remove_members = remove_member_ids
            .into_iter()
            .map(|id| id.parse::<DeviceId>().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let transfer_owner = transfer_owner_to
            .map(|id| id.parse::<DeviceId>().map_err(|error| error.to_string()))
            .transpose()?;
        if let Some(target) = transfer_owner {
            let online = runtime.discovery.as_ref().is_some_and(|discovery| {
                discovery
                    .nearby_peers()
                    .iter()
                    .any(|peer| peer.device_id == target)
            });
            if !online {
                return Err("new group owner must currently be online".to_owned());
            }
        }
        let result = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .update_group(
                &runtime.profile,
                operation_id,
                group_id,
                expected_revision,
                name.as_deref(),
                &add_members,
                &remove_members,
                transfer_owner,
                disband,
            )
            .map_err(|error| error.to_string())?;
        Ok(UpdateGroupDto {
            group_id: result.group_id.to_string(),
            revision: result.revision,
            invite_ids: result
                .invite_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
        })
    })
}

#[frb(sync)]
pub fn leave_group(client_operation_id: String, group_id: String) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let group_id = group_id
            .parse::<GroupId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .leave_group(&runtime.profile, operation_id, group_id)
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn list_own_device_bindings(state: Option<String>) -> Result<Vec<OwnDeviceBindingDto>, String> {
    with_runtime(|runtime| {
        let records = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .list_own_device_bindings(state.as_deref())
            .map_err(|error| error.to_string())?;
        let nearby = runtime
            .discovery
            .as_ref()
            .map(DiscoveryService::nearby_peers)
            .unwrap_or_default();
        Ok(records
            .into_iter()
            .map(|record| OwnDeviceBindingDto {
                peer_device_id: record.peer_device_id.to_string(),
                peer_name: record.peer_name,
                binding_id: record.binding_id.to_string(),
                state: record.state,
                clipboard_mode: record.clipboard_mode,
                requested_by_device_id: record.requested_by_device_id.to_string(),
                created_at_ms: record.created_at_ms,
                updated_at_ms: record.updated_at_ms,
                online: nearby
                    .iter()
                    .any(|peer| peer.device_id == record.peer_device_id),
            })
            .collect())
    })
}

#[frb(sync)]
pub fn request_own_device_binding(
    client_operation_id: String,
    peer_device_id: String,
) -> Result<String, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer = peer_device_id
            .parse::<DeviceId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .request_own_device_binding(&runtime.profile, operation_id, peer)
            .map(|binding_id| binding_id.to_string())
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn decide_own_device_binding(
    client_operation_id: String,
    binding_id: String,
    accept: bool,
) -> Result<String, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let binding = binding_id
            .parse::<BindingId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .decide_own_device_binding(&runtime.profile, operation_id, binding, accept)
            .map(|peer| peer.to_string())
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn remove_own_device_binding(
    client_operation_id: String,
    peer_device_id: String,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer = peer_device_id
            .parse::<DeviceId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .remove_own_device_binding(&runtime.profile, operation_id, peer)
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn set_clipboard_mode(
    client_operation_id: String,
    peer_device_id: String,
    mode: String,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer = peer_device_id
            .parse::<DeviceId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .set_clipboard_mode(operation_id, peer, &mode)
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn set_default_receive_ref(
    client_operation_id: String,
    receive_ref: Option<String>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .set_default_receive_ref(operation_id, receive_ref.as_deref())
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn get_app_settings() -> Result<AppSettingsDto, String> {
    with_runtime(|runtime| {
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .app_settings()
            .map(app_settings_dto)
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
#[allow(clippy::too_many_arguments)]
pub fn update_app_settings(
    client_operation_id: String,
    default_receive_policy: Option<String>,
    default_receive_ref: Option<String>,
    clear_default_receive_ref: bool,
    notifications_enabled: Option<bool>,
    close_to_tray: Option<bool>,
    start_on_boot: Option<bool>,
    android_keep_online: Option<bool>,
    auto_open_receive_directory: Option<bool>,
    log_level: Option<String>,
) -> Result<AppSettingsDto, String> {
    let result = with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .update_app_settings(
                operation_id,
                default_receive_policy.as_deref(),
                default_receive_ref.as_deref(),
                clear_default_receive_ref,
                notifications_enabled,
                close_to_tray,
                start_on_boot,
                android_keep_online,
                auto_open_receive_directory,
                log_level.as_deref(),
            )
            .map(app_settings_dto)
            .map_err(|error| error.to_string())
    });
    if result.is_ok() {
        events::invalidate();
    }
    result
}

#[frb(sync)]
pub fn list_peer_receive_policies() -> Result<Vec<PeerReceivePolicyDto>, String> {
    with_runtime(|runtime| {
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .list_peer_receive_policies()
            .map_err(|error| error.to_string())
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| PeerReceivePolicyDto {
                        device_id: record.device_id.to_string(),
                        device_name: record.device_name,
                        relation: record.relation,
                        receive_policy_override: record.receive_policy_override,
                        effective_receive_policy: record.effective_receive_policy,
                    })
                    .collect()
            })
    })
}

#[frb(sync)]
pub fn set_peer_receive_policy(
    client_operation_id: String,
    peer_device_id: String,
    receive_policy_override: Option<String>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer_device_id = peer_device_id
            .parse::<DeviceId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .set_peer_receive_policy(
                operation_id,
                peer_device_id,
                receive_policy_override.as_deref(),
            )
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn update_device_name(
    client_operation_id: String,
    device_name: String,
) -> Result<StartCoreDto, String> {
    let operation_id = client_operation_id
        .parse::<ClientOperationId>()
        .map_err(|error| error.to_string())?;
    let mut guard = CORE_RUNTIME
        .lock()
        .map_err(|_| "core runtime lock is poisoned".to_owned())?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| "core is not started".to_owned())?;
    let network_enabled = runtime.discovery.is_some() || runtime.control.is_some();
    if let Some(mut control) = runtime.control.take() {
        control.shutdown();
    }
    if let Some(mut discovery) = runtime.discovery.take() {
        discovery.shutdown();
    }
    let mut storage = Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
    let profile = storage
        .update_device_name(operation_id, &device_name)
        .map_err(|error| error.to_string())?;
    drop(storage);
    runtime.profile = profile.clone();

    let mut discovery_error = None;
    let mut next_control = None;
    let mut next_discovery = None;
    if network_enabled {
        let (control, discovery, error) = start_network_pair(
            || {
                ControlService::start(profile.clone(), Path::new(&runtime.database_path))
                    .map_err(|error| error.to_string())
            },
            || {
                DiscoveryService::start(profile.clone(), Path::new(&runtime.database_path))
                    .map_err(|error| error.to_string())
            },
        )?;
        next_control = Some(control);
        next_discovery = discovery;
        discovery_error = error;
    }
    runtime.control = next_control;
    runtime.discovery = next_discovery;
    Ok(StartCoreDto {
        device_id: profile.device_id.to_string(),
        device_name: profile.device_name,
        platform: platform_name(profile.platform).to_owned(),
        discovery_available: runtime.discovery.is_some(),
        discovery_error,
    })
}

#[frb(sync)]
pub fn submit_local_clipboard_text(
    client_operation_id: String,
    peer_device_id: String,
    text: String,
    automatic: bool,
) -> Result<ClipboardSendDto, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let peer = peer_device_id
            .parse::<DeviceId>()
            .map_err(|error| error.to_string())?;
        let sent = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .submit_local_clipboard_text(&runtime.profile, operation_id, peer, &text, automatic)
            .map_err(|error| error.to_string())?;
        Ok(ClipboardSendDto {
            message_id: sent.message_id.to_string(),
            event_id: sent.event_id.to_string(),
            clipboard_sequence: sent.clipboard_sequence,
        })
    })
}

#[frb(sync)]
pub fn list_conversations() -> Result<Vec<ConversationDto>, String> {
    with_runtime(|runtime| {
        let storage = Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .list_conversations()
            .map_err(|error| error.to_string())
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| conversation_dto(runtime, record))
                    .collect()
            })
    })
}

#[frb(sync)]
pub fn mark_conversation_read(
    client_operation_id: String,
    conversation_id: String,
    through_sort_order: i64,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .mark_conversation_read(
                operation_id,
                runtime.profile.device_id,
                &conversation_id,
                through_sort_order,
            )
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn delete_conversation(
    client_operation_id: String,
    conversation_id: String,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let result = storage
            .delete_conversation(operation_id, &conversation_id)
            .map_err(|error| error.to_string())?;
        for transfer_id in result.cancelled_transfer_ids {
            stop_active_transfer(transfer_id);
        }
        Ok(())
    })
}

#[frb(sync)]
pub fn list_messages(conversation_id: String, limit: u32) -> Result<Vec<MessageDto>, String> {
    with_runtime(|runtime| {
        let storage = Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .list_messages(&conversation_id, limit)
            .map_err(|error| error.to_string())
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| message_dto(runtime, record))
                    .collect()
            })
    })
}

#[frb(sync)]
pub fn list_message_deliveries(message_id: String) -> Result<Vec<MessageDeliveryDto>, String> {
    let message_id = MessageId::from_str(&message_id).map_err(|error| error.to_string())?;
    with_runtime(|runtime| {
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .list_message_deliveries(message_id)
            .map_err(|error| error.to_string())
            .map(|records| records.into_iter().map(message_delivery_dto).collect())
    })
}

#[frb(sync)]
pub fn send_text_message(
    client_operation_id: String,
    conversation_id: String,
    text: String,
) -> Result<MessageDto, String> {
    let result = with_runtime(|runtime| {
        let client_operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .create_outgoing_text_idempotent(
                &runtime.profile,
                client_operation_id,
                &conversation_id,
                &text,
            )
            .map_err(|error| error.to_string())
            .map(|record| message_dto(runtime, record))
    });
    publish_message_result(&result);
    result
}

#[frb(sync)]
pub fn send_clipboard_text_message(
    client_operation_id: String,
    conversation_id: String,
    text: String,
) -> Result<MessageDto, String> {
    let result = with_runtime(|runtime| {
        let client_operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .create_outgoing_clipboard_text_idempotent(
                &runtime.profile,
                client_operation_id,
                &conversation_id,
                &text,
            )
            .map_err(|error| error.to_string())
            .map(|record| message_dto(runtime, record))
    });
    publish_message_result(&result);
    result
}

#[frb(sync)]
pub fn send_source_path(
    client_operation_id: String,
    conversation_id: String,
    source_path: String,
    message_kind: String,
) -> Result<SendSourceDto, String> {
    let result = with_runtime(|runtime| {
        let client_operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let message_kind = parse_source_message_kind(&message_kind)?;
        let manifest =
            enumerate_path(&source_path, message_kind).map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let outgoing = storage
            .create_outgoing_offer_idempotent(
                &runtime.profile,
                client_operation_id,
                &conversation_id,
                &manifest,
                None,
                false,
            )
            .map_err(|error| error.to_string())?;
        Ok(SendSourceDto {
            message_id: outgoing.message_id.to_string(),
            transfer_id: outgoing.transfer_id.to_string(),
        })
    });
    publish_source_result(&result);
    result
}

#[frb(sync)]
pub fn send_source_items(
    client_operation_id: String,
    conversation_id: String,
    display_name: String,
    message_kind: String,
    sources: Vec<SourceItemDto>,
) -> Result<SendSourceDto, String> {
    let result = with_runtime(|runtime| {
        let client_operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let message_kind = parse_source_message_kind(&message_kind)?;
        let sources = sources
            .into_iter()
            .map(|source| {
                Ok(SourceItem {
                    entry_kind: match source.entry_kind.as_str() {
                        "file" => TransferEntryKind::File,
                        "directory" => TransferEntryKind::Directory,
                        _ => return Err("entry_kind must be file or directory".to_owned()),
                    },
                    source_ref: source.source_ref,
                    relative_path: source.relative_path,
                    size: source.size,
                    modified_at_ms: source.modified_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let manifest = build_manifest(message_kind, display_name, sources)
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let outgoing = storage
            .create_outgoing_offer_idempotent(
                &runtime.profile,
                client_operation_id,
                &conversation_id,
                &manifest,
                None,
                false,
            )
            .map_err(|error| error.to_string())?;
        Ok(SendSourceDto {
            message_id: outgoing.message_id.to_string(),
            transfer_id: outgoing.transfer_id.to_string(),
        })
    });
    publish_source_result(&result);
    result
}

#[frb(sync)]
pub fn send_clipboard_image_items(
    client_operation_id: String,
    conversation_id: String,
    display_name: String,
    sources: Vec<SourceItemDto>,
    content_fingerprint: String,
    automatic: bool,
) -> Result<SendSourceDto, String> {
    let result = with_runtime(|runtime| {
        let client_operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let sources = sources
            .into_iter()
            .map(|source| {
                Ok(SourceItem {
                    entry_kind: match source.entry_kind.as_str() {
                        "file" => TransferEntryKind::File,
                        "directory" => TransferEntryKind::Directory,
                        _ => return Err("entry_kind must be file or directory".to_owned()),
                    },
                    source_ref: source.source_ref,
                    relative_path: source.relative_path,
                    size: source.size,
                    modified_at_ms: source.modified_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let manifest = build_manifest(MessageKind::ClipboardImage, display_name, sources)
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let outgoing = storage
            .create_outgoing_offer_idempotent(
                &runtime.profile,
                client_operation_id,
                &conversation_id,
                &manifest,
                Some(&content_fingerprint),
                automatic,
            )
            .map_err(|error| error.to_string())?;
        Ok(SendSourceDto {
            message_id: outgoing.message_id.to_string(),
            transfer_id: outgoing.transfer_id.to_string(),
        })
    });
    publish_source_result(&result);
    result
}

fn publish_message_result(result: &Result<MessageDto, String>) {
    if let Ok(message) = result {
        events::publish(
            crate::events::CoreEventKind::MessageChanged,
            Some(message.message_id.clone()),
        );
    }
}

fn publish_source_result(result: &Result<SendSourceDto, String>) {
    if let Ok(source) = result {
        events::publish(
            crate::events::CoreEventKind::MessageChanged,
            Some(source.message_id.clone()),
        );
        events::publish(
            crate::events::CoreEventKind::TransferProgress,
            Some(source.transfer_id.clone()),
        );
    }
}

#[frb(sync)]
pub fn replace_transfer_source_path(
    client_operation_id: String,
    transfer_id: String,
    source_path: String,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let source = Path::new(&source_path);
        let message_kind = if source.is_dir() {
            MessageKind::Folder
        } else {
            MessageKind::File
        };
        let manifest = enumerate_path(source, message_kind).map_err(|error| error.to_string())?;
        let sources = manifest
            .entries
            .into_iter()
            .map(|entry| SourceItem {
                entry_kind: entry.entry_kind,
                source_ref: entry.source_ref,
                relative_path: entry.relative_path,
                size: entry.size,
                modified_at_ms: entry.modified_at_ms,
            })
            .collect::<Vec<_>>();
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .replace_transfer_sources(&runtime.profile, operation_id, transfer_id, &sources)
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn replace_transfer_source_items(
    client_operation_id: String,
    transfer_id: String,
    sources: Vec<SourceItemDto>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let sources = sources
            .into_iter()
            .map(|source| {
                Ok(SourceItem {
                    entry_kind: match source.entry_kind.as_str() {
                        "file" => TransferEntryKind::File,
                        "directory" => TransferEntryKind::Directory,
                        _ => return Err("entry_kind must be file or directory".to_owned()),
                    },
                    source_ref: source.source_ref,
                    relative_path: source.relative_path,
                    size: source.size,
                    modified_at_ms: source.modified_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .replace_transfer_sources(&runtime.profile, operation_id, transfer_id, &sources)
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn replace_receive_destination_path(
    client_operation_id: String,
    transfer_id: String,
    receive_base_ref: String,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        let (transfer, entries) = storage
            .get_transfer(transfer_id)
            .map_err(|error| error.to_string())?;
        let prepared = prepare_receive_paths(
            &receive_base_ref,
            transfer_id,
            &transfer.display_name,
            &entries,
        )
        .map_err(|error| error.to_string())?;
        storage
            .replace_receive_destination(
                &runtime.profile,
                operation_id,
                transfer_id,
                &receive_base_ref,
                &prepared,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn replace_receive_destination_prepared(
    client_operation_id: String,
    transfer_id: String,
    receive_base_ref: String,
    prepared: Vec<PreparedReceiveEntryDto>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let prepared = prepared
            .into_iter()
            .map(|entry| {
                Ok(PreparedReceiveEntry {
                    entry_id: entry
                        .entry_id
                        .parse::<EntryId>()
                        .map_err(|error| error.to_string())?,
                    destination_ref: entry.destination_ref,
                    partial_ref: entry.partial_ref,
                    persisted_offset: entry.persisted_offset,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .replace_receive_destination(
                &runtime.profile,
                operation_id,
                transfer_id,
                &receive_base_ref,
                &prepared,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn list_transfers() -> Result<Vec<TransferDto>, String> {
    with_runtime(|runtime| {
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .list_transfers()
            .map_err(|error| error.to_string())
            .map(|transfers| transfers.into_iter().map(transfer_dto).collect())
    })
}

#[frb(sync)]
pub fn clear_completed_transfers(client_operation_id: String) -> Result<u64, String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .clear_completed_transfers(operation_id)
            .map_err(|error| error.to_string())
    })
}

#[frb(sync)]
pub fn list_transfer_entries(transfer_id: String) -> Result<Vec<TransferEntryDto>, String> {
    with_runtime(|runtime| {
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let (_, entries) = Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .get_transfer(transfer_id)
            .map_err(|error| error.to_string())?;
        Ok(entries
            .into_iter()
            .map(|entry| TransferEntryDto {
                entry_id: entry.entry_id.to_string(),
                entry_kind: match entry.entry_kind {
                    TransferEntryKind::File => "file",
                    TransferEntryKind::Directory => "directory",
                }
                .to_owned(),
                relative_path: entry.relative_path,
                size: entry.size,
                persisted_offset: entry.persisted_offset,
            })
            .collect())
    })
}

#[frb(sync)]
pub fn decide_incoming_offer(
    client_operation_id: String,
    transfer_id: String,
    accept: bool,
    receive_base_ref: Option<String>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        if !accept {
            storage
                .reject_incoming_offer(&runtime.profile, operation_id, transfer_id, "user_rejected")
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
        let receive_base_ref = receive_base_ref
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| "receive_base_ref is required when accepting an offer".to_owned())?;
        let (transfer, entries) = storage
            .get_transfer(transfer_id)
            .map_err(|error| error.to_string())?;
        let prepared = prepare_receive_paths(
            &receive_base_ref,
            transfer_id,
            &transfer.display_name,
            &entries,
        )
        .map_err(|error| error.to_string())?;
        storage
            .accept_incoming_offer(
                &runtime.profile,
                operation_id,
                transfer_id,
                &receive_base_ref,
                &prepared,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn decide_incoming_offer_prepared(
    client_operation_id: String,
    transfer_id: String,
    receive_base_ref: String,
    prepared: Vec<PreparedReceiveEntryDto>,
) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let prepared = prepared
            .into_iter()
            .map(|entry| {
                Ok(PreparedReceiveEntry {
                    entry_id: entry
                        .entry_id
                        .parse::<EntryId>()
                        .map_err(|error| error.to_string())?,
                    destination_ref: entry.destination_ref,
                    partial_ref: entry.partial_ref,
                    persisted_offset: entry.persisted_offset,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .accept_incoming_offer(
                &runtime.profile,
                operation_id,
                transfer_id,
                &receive_base_ref,
                &prepared,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn pause_transfer(client_operation_id: String, transfer_id: String) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        let mut storage =
            Storage::open(&runtime.database_path).map_err(|error| error.to_string())?;
        storage
            .pause_transfer(&runtime.profile, operation_id, transfer_id)
            .map_err(|error| error.to_string())?;
        stop_active_transfer(transfer_id);
        Ok(())
    })
}

#[frb(sync)]
pub fn resume_transfer(client_operation_id: String, transfer_id: String) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .resume_transfer(&runtime.profile, operation_id, transfer_id)
            .map_err(|error| error.to_string())?;
        Ok(())
    })
}

#[frb(sync)]
pub fn cancel_transfer(client_operation_id: String, transfer_id: String) -> Result<(), String> {
    with_runtime(|runtime| {
        let operation_id = client_operation_id
            .parse::<ClientOperationId>()
            .map_err(|error| error.to_string())?;
        let transfer_id = transfer_id
            .parse::<TransferId>()
            .map_err(|error| error.to_string())?;
        Storage::open(&runtime.database_path)
            .map_err(|error| error.to_string())?
            .cancel_transfer(&runtime.profile, operation_id, transfer_id)
            .map_err(|error| error.to_string())?;
        stop_active_transfer(transfer_id);
        Ok(())
    })
}

#[frb(sync)]
pub fn poll_platform_requests() -> Vec<PlatformRequestDto> {
    crate::platform_io::poll_platform_requests()
}

#[frb(sync)]
pub fn complete_platform_request(
    request_id: String,
    fd: Option<i32>,
    value: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    complete_platform_io_request(request_id, fd, value, error)
}

#[frb(sync)]
pub fn get_nearby_peers() -> Vec<NearbyPeerDto> {
    let runtime = CORE_RUNTIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    runtime
        .as_ref()
        .and_then(|runtime| runtime.discovery.as_ref())
        .map(DiscoveryService::nearby_peers)
        .unwrap_or_default()
        .into_iter()
        .map(|peer| NearbyPeerDto {
            device_id: peer.device_id.to_string(),
            device_name: peer.device_name,
            platform: platform_name(peer.platform).to_owned(),
            source_ip: peer.source_ip.to_string(),
            last_seen_at_ms: peer.last_seen_at_ms,
        })
        .collect()
}

#[frb(sync)]
pub fn get_local_profile() -> Option<StartCoreDto> {
    let runtime = CORE_RUNTIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    runtime.as_ref().map(|runtime| StartCoreDto {
        device_id: runtime.profile.device_id.to_string(),
        device_name: runtime.profile.device_name.clone(),
        platform: platform_name(runtime.profile.platform).to_owned(),
        discovery_available: runtime.discovery.is_some(),
        discovery_error: None,
    })
}

#[frb(sync)]
pub fn shutdown_core() {
    events::disconnect();
    let mut runtime = CORE_RUNTIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(mut current) = runtime.take() {
        stop_all_active_transfers();
        if let Some(mut control) = current.control.take() {
            control.shutdown();
        }
        if let Some(mut discovery) = current.discovery.take() {
            discovery.shutdown();
        }
        if let Ok(mut storage) = Storage::open(&current.database_path) {
            let _ = storage.suspend_network_transfers();
        }
    }
}

fn with_runtime<T>(operation: impl FnOnce(&CoreRuntime) -> Result<T, String>) -> Result<T, String> {
    let runtime = CORE_RUNTIME
        .lock()
        .map_err(|_| "core runtime lock is poisoned".to_owned())?;
    operation(
        runtime
            .as_ref()
            .ok_or_else(|| "core is not started".to_owned())?,
    )
}

fn app_settings_dto(record: crate::storage::AppSettingsRecord) -> AppSettingsDto {
    AppSettingsDto {
        default_receive_policy: record.default_receive_policy,
        default_receive_ref: record.default_receive_ref,
        notifications_enabled: record.notifications_enabled,
        close_to_tray: record.close_to_tray,
        start_on_boot: record.start_on_boot,
        android_keep_online: record.android_keep_online,
        auto_open_receive_directory: record.auto_open_receive_directory,
        log_level: record.log_level,
    }
}

fn conversation_dto(runtime: &CoreRuntime, record: ConversationRecord) -> ConversationDto {
    let nearby = runtime
        .discovery
        .as_ref()
        .map(DiscoveryService::nearby_peers)
        .unwrap_or_default();
    let (online, online_count) = if let Some(peer_device_id) = record.peer_device_id {
        let peer_online = nearby.iter().any(|peer| peer.device_id == peer_device_id);
        (peer_online, u32::from(peer_online) + 1)
    } else if let Some(group_id) = record.group_id.as_ref() {
        let joined = Storage::open(&runtime.database_path)
            .ok()
            .and_then(|storage| {
                storage
                    .get_group(runtime.profile.device_id, group_id.clone())
                    .ok()
            })
            .map(|group| {
                group
                    .snapshot
                    .members
                    .iter()
                    .filter(|member| {
                        member.membership == "joined"
                            && (member.device_id == runtime.profile.device_id
                                || nearby.iter().any(|peer| peer.device_id == member.device_id))
                    })
                    .count()
            })
            .unwrap_or(1);
        let count = u32::try_from(joined).unwrap_or(u32::MAX);
        (count > 1, count)
    } else {
        (false, 0)
    };
    ConversationDto {
        conversation_id: record.conversation_id,
        title: record.title,
        peer_device_id: record.peer_device_id.map(|id| id.to_string()),
        group_id: record.group_id.map(|id| id.to_string()),
        is_group: record.is_group,
        member_count: record.member_count,
        online_count,
        last_message_preview: record.last_message_preview,
        last_activity_at_ms: record.last_activity_at_ms,
        unread_count: record.unread_count,
        online,
    }
}

fn message_dto(runtime: &CoreRuntime, record: MessageRecord) -> MessageDto {
    MessageDto {
        message_id: record.message_id.to_string(),
        conversation_id: record.conversation_id,
        sender_device_id: record.sender_device_id.to_string(),
        kind: record.kind,
        state: record.state,
        text: record.text,
        total_size: record.total_size,
        entry_count: record.entry_count,
        transfer_id: record.transfer_id.map(|id| id.to_string()),
        transfer_state: record.transfer_state,
        persisted_bytes: record.persisted_bytes,
        local_file_ref: record.local_file_ref,
        created_at_ms: record.created_at_ms,
        local_sort_order: record.local_sort_order,
        delivered_count: record.delivered_count,
        delivery_count: record.delivery_count,
        outgoing: record.sender_device_id == runtime.profile.device_id,
    }
}

fn message_delivery_dto(record: MessageDeliveryRecord) -> MessageDeliveryDto {
    MessageDeliveryDto {
        recipient_device_id: record.recipient_device_id.to_string(),
        recipient_name: record.recipient_name,
        state: record.state,
        failure_reason: record.failure_reason,
        updated_at_ms: record.updated_at_ms,
        delivered: record.delivered,
    }
}

fn transfer_dto(record: TransferRecord) -> TransferDto {
    TransferDto {
        transfer_id: record.transfer_id.to_string(),
        message_id: record.message_id.to_string(),
        conversation_id: record.conversation_id,
        peer_device_id: record.peer_device_id.to_string(),
        direction: record.direction,
        state: record.state,
        failure_reason: record.failure_reason,
        display_name: record.display_name,
        total_size: record.total_size,
        entry_count: record.entry_count,
        persisted_bytes: record.persisted_bytes,
        receive_base_ref: record.receive_base_ref,
        local_file_ref: record.local_file_ref,
        paused_by_user: record.paused_by_user,
    }
}

fn parse_platform(value: &str) -> Result<Platform, String> {
    match value {
        "windows" => Ok(Platform::Windows),
        "android" => Ok(Platform::Android),
        _ => Err("platform must be windows or android".to_owned()),
    }
}

fn parse_source_message_kind(value: &str) -> Result<MessageKind, String> {
    match value {
        "file" => Ok(MessageKind::File),
        "image" => Ok(MessageKind::Image),
        "folder" => Ok(MessageKind::Folder),
        "clipboard_image" => Ok(MessageKind::ClipboardImage),
        _ => Err("message_kind must be file, image, folder, or clipboard_image".to_owned()),
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "windows",
        Platform::Android => "android",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_errors_have_stable_codes_and_user_messages() {
        let cases = [
            (
                "failed to open database: disk I/O error",
                "STORAGE_OPEN_FAILED",
                "无法打开本地数据库",
                true,
            ),
            (
                "group revision does not follow the locally stored revision",
                "GROUP_REVISION_CONFLICT",
                "群资料正在同步，请稍后重试",
                true,
            ),
            (
                "source file changed after the offer",
                "TRANSFER_SOURCE_CHANGED",
                "源文件已变化，请重新选择",
                true,
            ),
            (
                "only the current group owner can perform this operation",
                "NOT_GROUP_OWNER",
                "只有群主可以执行此操作",
                false,
            ),
        ];
        for (message, code, user_message, recoverable) in cases {
            let error = describe_core_error(message.to_owned());
            assert_eq!(error.code, code);
            assert_eq!(error.user_message, user_message);
            assert_eq!(error.recoverable, recoverable);
            assert_eq!(error.message, message);
            assert_eq!(error.related_id, None);
        }
    }

    #[test]
    fn unknown_core_error_does_not_expose_internal_details_as_user_copy() {
        let error = describe_core_error("secret internal detail".to_owned());
        assert_eq!(error.code, "INTERNAL_ERROR");
        assert_eq!(error.user_message, "发生内部错误，请重试");
        assert_ne!(error.message, error.user_message);
    }

    #[test]
    fn start_core_persists_identity_without_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lan_chat.db").display().to_string();
        let first = start_core(
            path.clone(),
            "测试电脑".to_owned(),
            "windows".to_owned(),
            false,
        )
        .unwrap();
        shutdown_core();
        let second = start_core(path, "新名称".to_owned(), "windows".to_owned(), false).unwrap();
        assert_eq!(first.device_id, second.device_id);
        assert_eq!(second.device_name, "测试电脑");
        let renamed = update_device_name(
            ClientOperationId::generate().to_string(),
            "新名称".to_owned(),
        )
        .unwrap();
        assert_eq!(renamed.device_name, "新名称");
        assert!(!second.discovery_available);
        shutdown_core();
    }

    #[test]
    fn network_restart_never_publishes_discovery_when_control_bind_fails() {
        use std::cell::Cell;

        let discovery_started = Cell::new(false);
        let result = start_network_pair(
            || Err::<(), _>("TCP bind failed".to_owned()),
            || {
                discovery_started.set(true);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "TCP bind failed");
        assert!(!discovery_started.get());
    }
}
