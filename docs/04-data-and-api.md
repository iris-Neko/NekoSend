# 数据、状态与 API 规范

## 目的

本文档是稳定标识、枚举、状态转换、SQLite DDL、Rust/Flutter 接口和错误码的唯一权威来源。其他文档引用本页，不得另行定义同名状态、字段或错误码。

## 前置知识

- 已阅读 [总体架构](02-architecture.md) 和 [网络协议](03-network-protocol.md)。
- 理解 SQLite 外键、事务、WAL、唯一索引和幂等键。
- 理解 FFI 命令成功表示“本地核心已接受”，不代表远端已经完成。

## 数据表示约定

- SQLite 布尔值使用 `INTEGER NOT NULL CHECK(value IN (0,1))`。
- SQLite 时间使用 Unix 毫秒 `INTEGER`。
- SQLite 字节数和偏移使用非负 `INTEGER`；SQLite 有符号 64 位上限足够覆盖 V1 文件大小。
- Rust 使用 `i64` 表示时间，使用 `u64` 表示大小、偏移和序号。
- JSON/FFI 枚举统一使用本文规定的小写 `snake_case` 字符串。
- 数据库正文与路径使用 UTF-8 `TEXT`；文件内容绝不进入数据库。
- 所有创建/更新时间由执行事务的本机生成，不依赖对端时钟作为排序真值。

## 稳定标识

| 类型 | 格式 | 生成与稳定性 |
|---|---|---|
| `DeviceId` | `d_` + 32 位小写十六进制 | 首次安装用系统随机源生成 16 字节并持久化；卸载清数据后才变化 |
| `PrivateConversationId` | `p:<较小DeviceId>:<较大DeviceId>` | 两个 DeviceId 按 ASCII 排序后确定，两端结果相同 |
| `GroupId` | `g:<creatorDeviceId>:<localSequence>` | 创建者在事务中递增本地 u64 序号，一次生成后永久不变 |
| `MessageId` | 标准小写 UUIDv7 | 每个逻辑消息一个；群聊向多成员发送时复用 |
| `TransferId` | 标准小写 UUIDv7 | 每个发送方到接收方的文件任务一个；群文件每位接收者不同 |
| `EntryId` | 标准小写 UUIDv7 | 每个 transfer 内的文件条目一个 |
| `EventId` | 标准小写 UUIDv7 | 每个网络业务事件一个；重发不得变化 |
| `InviteId` | 标准小写 UUIDv7 | 每次群邀请一个 |
| `BindingId` | 标准小写 UUIDv7 | 每次“我的设备”绑定请求一个 |
| `ClientOperationId` | 标准小写 UUIDv7 | Flutter 每次用户操作生成，用于防重复点击 |
| `PlatformRequestId` | 标准小写 UUIDv7 | Rust 请求平台打开文件/写剪贴板时生成 |

UUIDv7 只用于唯一性和大致时间排序；业务顺序仍以数据库接收序、群 revision 和明确状态转换为准。

## 权威枚举

### 平台与关系

```text
Platform         = windows | android
PeerRelation     = nearby | known | own_device
Presence         = offline | online
ReceivePolicy    = auto_accept | ask_every_time
ClipboardMode    = off | send_only | receive_only | bidirectional
BindingState     = pending_outbound | pending_inbound | active | rejected | removed
LogLevel         = normal | debug
```

`ReceivePolicy` 的全局默认值为 `auto_accept`，但未知设备无论默认值如何都需要首次确认。每设备覆盖为空时使用全局值。

### 会话、群和消息

```text
ConversationKind = private | group
ConversationState = active | left | disbanded
GroupRole         = owner | member
GroupMembership   = invited | joined | left | removed
MessageKind       = text | file | image | folder | clipboard_text | clipboard_image | system
MessageState      = queued | sending | partially_delivered | delivered | failed | cancelled
DeliveryState     = queued | sending | stored | accepted | rejected | completed | failed | cancelled
```

聚合规则：

- 私聊文字：远端 `stored` 后 MessageState 为 `delivered`。
- 私聊文件：传输 `completed` 后 MessageState 为 `delivered`。
- 群聊：全部目标 `completed/stored` 为 `delivered`；部分完成且仍有等待为 `partially_delivered`；全部等待为 `queued`。
- 任一目标失败但仍有可继续目标时保持 `partially_delivered`；所有目标均为最终失败/拒绝时为 `failed`。
- MessageState 是缓存字段，必须在更新 delivery 的同一事务中重新计算。

### 传输状态

```text
TransferDirection = send | receive
TransferState     = queued | offered | accepted | transferring | paused | verifying | completed | failed | cancelled
TransferEntryKind = file | directory
TransferEntryState = queued | transferring | completed | failed | cancelled
TransferFailureReason = peer_offline | rejected | source_changed | not_enough_space |
                        connection_error | invalid_path | permission_lost |
                        unsupported | user_cancelled
```

状态含义：

- `queued`：本地已创建，等待对端在线或等待调度；`peer_offline` 只记录为最近原因，任务仍为 queued。
- `offered`：对端已存储 Offer，可能等待用户确认。
- `accepted`：接收端已确认路径和偏移，等待数据连接。
- `transferring`：数据连接正在读写。
- `paused`：用户或系统明确暂停，重启后不自动继续用户暂停的任务。
- `verifying`：所有字节已读完，正在刷新、检查大小和发布最终文件。
- `completed`：最终文件已安全保存并提交完成状态。
- `failed`：需要用户处理或最终不可继续；是否可恢复由 failure reason 决定。
- `cancelled`：用户取消，不能原任务继续；重新发送会生成新 TransferId。

### 传输状态转换

| 当前状态 | 允许的下一状态 |
|---|---|
| `queued` | `offered`, `cancelled`, `failed` |
| `offered` | `accepted`, `queued`, `failed`, `cancelled` |
| `accepted` | `transferring`, `queued`, `paused`, `failed`, `cancelled` |
| `transferring` | `verifying`, `queued`, `paused`, `failed`, `cancelled` |
| `paused` | `queued`, `accepted`, `cancelled`, `failed` |
| `verifying` | `completed`, `failed` |
| `failed` | `queued`, `accepted`, `cancelled`，仅限可恢复原因 |
| `completed` | 无 |
| `cancelled` | 无 |

特殊规则：

- 网络中断从 `transferring` 回到 `queued`，最近原因记为 `connection_error`，不是最终 failed。
- 对端离线保持或回到 `queued`，最近原因记为 `peer_offline`。
- `source_changed`、`not_enough_space`、`permission_lost` 进入 `failed`，用户处理后才可恢复。
- `rejected` 和 `unsupported` 是最终 failed，不能原任务重试。
- 用户取消直接进入 `cancelled`，同时使用 `user_cancelled` 记录原因。

### Outbox 与邀请

```text
OutboxState      = pending | in_flight
InvitationState  = pending | accepted | rejected | cancelled
GroupState       = active | disbanded
OfferDecision    = accept | reject
InviteDecision   = accept | reject
BindingDecision  = accept | reject
TransferFilter   = all | active | waiting | completed | failed
CoreHealthState  = ready | degraded | failed
```

## SQLite 配置

每次打开连接后执行：

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
```

不得使用 `synchronous=OFF`。性能优化不得牺牲已确认偏移和完成状态的崩溃一致性。

## SQLite DDL（当前 Schema Version 3）

以下 SQL 是当前权威结构。实现时拆成 V1、V2、V3 迁移文件，但最终语义和约束必须保持一致。

```sql
CREATE TABLE schema_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

INSERT INTO schema_meta(key, value) VALUES ('schema_version', '3');

CREATE TABLE local_profile (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    device_id TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android')),
    group_sequence INTEGER NOT NULL DEFAULT 0 CHECK (group_sequence >= 0),
    clipboard_sequence INTEGER NOT NULL DEFAULT 0 CHECK (clipboard_sequence >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE app_settings (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    default_receive_policy TEXT NOT NULL DEFAULT 'auto_accept'
        CHECK (default_receive_policy IN ('auto_accept', 'ask_every_time')),
    default_receive_ref TEXT,
    notifications_enabled INTEGER NOT NULL DEFAULT 1 CHECK (notifications_enabled IN (0,1)),
    close_to_tray INTEGER NOT NULL DEFAULT 1 CHECK (close_to_tray IN (0,1)),
    start_on_boot INTEGER NOT NULL DEFAULT 0 CHECK (start_on_boot IN (0,1)),
    android_keep_online INTEGER NOT NULL DEFAULT 1 CHECK (android_keep_online IN (0,1)),
    log_level TEXT NOT NULL DEFAULT 'normal' CHECK (log_level IN ('normal', 'debug')),
    updated_at_ms INTEGER NOT NULL,
    auto_open_receive_directory INTEGER NOT NULL DEFAULT 0
        CHECK (auto_open_receive_directory IN (0,1))
) STRICT;

CREATE TABLE peers (
    device_id TEXT PRIMARY KEY,
    device_name TEXT NOT NULL CHECK (length(device_name) BETWEEN 1 AND 32),
    platform TEXT NOT NULL CHECK (platform IN ('windows', 'android')),
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

CREATE INDEX idx_peers_last_seen ON peers(last_seen_at_ms DESC);
CREATE INDEX idx_peers_relation ON peers(relation);

CREATE TABLE own_device_bindings (
    peer_device_id TEXT PRIMARY KEY REFERENCES peers(device_id) ON DELETE CASCADE,
    binding_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL
        CHECK (state IN ('pending_outbound', 'pending_inbound', 'active', 'rejected', 'removed')),
    clipboard_mode TEXT NOT NULL DEFAULT 'off'
        CHECK (clipboard_mode IN ('off', 'send_only', 'receive_only', 'bidirectional')),
    requested_by_device_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 50),
    owner_device_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'disbanded')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    disbanded_at_ms INTEGER
) STRICT;

CREATE TABLE group_members (
    group_id TEXT NOT NULL REFERENCES groups(group_id) ON DELETE CASCADE,
    device_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    membership TEXT NOT NULL CHECK (membership IN ('invited', 'joined', 'left', 'removed')),
    joined_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (group_id, device_id)
) STRICT;

CREATE INDEX idx_group_members_device ON group_members(device_id, membership);

CREATE TABLE conversations (
    conversation_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('private', 'group')),
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'left', 'disbanded')),
    peer_device_id TEXT REFERENCES peers(device_id),
    group_id TEXT REFERENCES groups(group_id),
    title_cache TEXT NOT NULL,
    last_message_id TEXT,
    last_activity_at_ms INTEGER NOT NULL,
    unread_count INTEGER NOT NULL DEFAULT 0 CHECK (unread_count >= 0),
    created_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    CHECK (
      (kind = 'private' AND peer_device_id IS NOT NULL AND group_id IS NULL) OR
      (kind = 'group' AND group_id IS NOT NULL AND peer_device_id IS NULL)
    )
) STRICT;

CREATE INDEX idx_conversations_activity
    ON conversations(deleted_at_ms, last_activity_at_ms DESC);

CREATE TABLE messages (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
    sender_device_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('text', 'file', 'image', 'folder', 'clipboard_text', 'clipboard_image', 'system')),
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'sending', 'partially_delivered', 'delivered', 'failed', 'cancelled')),
    text_content TEXT,
    display_name TEXT,
    total_size INTEGER CHECK (total_size IS NULL OR total_size >= 0),
    entry_count INTEGER CHECK (entry_count IS NULL OR entry_count >= 0),
    group_revision INTEGER,
    created_at_ms INTEGER NOT NULL,
    received_at_ms INTEGER,
    local_sort_order INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    CHECK (
      (kind IN ('text', 'clipboard_text', 'system') AND text_content IS NOT NULL) OR
      (kind IN ('file', 'image', 'folder', 'clipboard_image') AND display_name IS NOT NULL)
    )
) STRICT;

CREATE UNIQUE INDEX idx_messages_conversation_sort
    ON messages(conversation_id, local_sort_order);
CREATE INDEX idx_messages_conversation_time
    ON messages(conversation_id, deleted_at_ms, local_sort_order DESC);

CREATE TABLE message_deliveries (
    message_id TEXT NOT NULL REFERENCES messages(message_id) ON DELETE CASCADE,
    recipient_device_id TEXT NOT NULL,
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'sending', 'stored', 'accepted', 'rejected', 'completed', 'failed', 'cancelled')),
    last_event_id TEXT,
    failure_reason TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (message_id, recipient_device_id)
) STRICT;

CREATE INDEX idx_deliveries_recipient_state
    ON message_deliveries(recipient_device_id, state, updated_at_ms);

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

CREATE INDEX idx_transfers_schedulable
    ON transfers(direction, state, updated_at_ms);
CREATE INDEX idx_transfers_peer
    ON transfers(peer_device_id, state);

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

CREATE INDEX idx_transfer_entries_transfer
    ON transfer_entries(transfer_id, state, relative_path);

CREATE TABLE group_invitations (
    invite_id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    peer_device_id TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('send', 'receive')),
    state TEXT NOT NULL CHECK (state IN ('pending', 'accepted', 'rejected', 'cancelled')),
    snapshot_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (group_id, peer_device_id, direction, invite_id)
) STRICT;

CREATE TABLE outbox (
    outbox_id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_device_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    requires_receipt INTEGER NOT NULL CHECK (requires_receipt IN (0,1)),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'in_flight')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (peer_device_id, event_id)
) STRICT;

CREATE INDEX idx_outbox_due
    ON outbox(peer_device_id, state, next_attempt_at_ms, outbox_id);

CREATE TABLE processed_events (
    sender_device_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    receipt_json TEXT,
    processed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (sender_device_id, event_id)
) STRICT;

CREATE INDEX idx_processed_events_time ON processed_events(processed_at_ms);

CREATE TABLE clipboard_dedup (
    origin_device_id TEXT NOT NULL,
    clipboard_sequence INTEGER NOT NULL CHECK (clipboard_sequence >= 0),
    message_id TEXT NOT NULL,
    content_fingerprint TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (origin_device_id, clipboard_sequence)
) STRICT;

CREATE INDEX idx_clipboard_dedup_time ON clipboard_dedup(created_at_ms);

CREATE TABLE client_operations (
    client_operation_id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    result_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;
```

### DDL 后置校验

SQLite 不能用简单 CHECK 保证“每群恰好一个 owner”，因此 `GroupRepository` 在同一事务中执行：

1. 校验成员数 1-32。
2. 校验 DeviceId 无重复。
3. 校验恰好一个 `role=owner` 且等于 `groups.owner_device_id`。
4. 校验 revision 严格递增。
5. 更新 groups 和 group_members 后再提交。

`messages.total_size`、`transfers.total_size`、`transfers.persisted_bytes` 只统计 file entry 的字节；directory entry 始终 size=0，不进入速度、进度和完成字节数计算。`entry_count` 同时统计 file 与 directory。

## 数据保留与清理

- `messages`、`transfers` 和群资料永久保留，直到用户删除本机会话。
- 已完成文件位于用户接收目录，删除会话不得删除文件。
- 取消接收任务时删除对应 `.partial`；无法删除时记录日志并下次启动重试。
- `processed_events` 至少保留 90 天；仍被 outbox、message 或 transfer 引用的事件不得清理。
- `clipboard_dedup` 保留 30 天或最近 10,000 条，取范围更大者。
- `client_operations` 保留 7 天，足以处理 UI 重试；删除不影响业务记录。
- SQLite 每次启动执行轻量清理；`VACUUM` 只在用户手动“优化存储”时执行。

## Rust 对 Flutter 接口

以下为语言无关的 Rust 风格定义。实际由 `flutter_rust_bridge` 生成 Dart 类型，字段名和枚举值不得改变。

### 启动与查询

```rust
struct StartConfig {
    app_data_dir: String,
    platform: Platform,
    app_version: String,
    protocol_version: u32, // V1 固定 1
    platform_capabilities: PlatformCapabilities,
}

struct PlatformCapabilities {
    can_watch_clipboard_in_background: bool,
    can_minimize_to_tray: bool,
    uses_saf: bool,
}

async fn start(config: StartConfig) -> Result<AppSnapshot, CoreError>;
async fn shutdown() -> Result<(), CoreError>;
async fn get_app_snapshot() -> Result<AppSnapshot, CoreError>;
async fn list_conversations(page: PageRequest) -> Result<Page<ConversationSummary>, CoreError>;
async fn list_messages(conversation_id: String, before_sort_order: Option<i64>, limit: u32)
    -> Result<Vec<MessageView>, CoreError>;
async fn list_transfers(filter: TransferFilter, page: PageRequest)
    -> Result<Page<TransferView>, CoreError>;
```

分页 `limit` 范围 1-100，默认 50。查询返回数据库快照，不触发网络行为。

### 用户命令

所有会产生持久化变化的命令必须携带 `client_operation_id`。重复提交同一 ID 返回首次保存的结果，不重复创建记录。

```rust
async fn set_device_name(client_operation_id: String, name: String) -> Result<(), CoreError>;
async fn refresh_nearby() -> Result<(), CoreError>;
async fn open_private_conversation(client_operation_id: String, peer_device_id: String)
    -> Result<String, CoreError>; // conversation_id

async fn send_text(req: SendTextRequest) -> Result<SendMessageResult, CoreError>;
async fn send_sources(req: SendSourcesRequest) -> Result<SendMessageResult, CoreError>;
async fn decide_incoming_offer(req: OfferDecisionRequest) -> Result<(), CoreError>;
async fn pause_transfer(client_operation_id: String, transfer_id: String) -> Result<(), CoreError>;
async fn resume_transfer(client_operation_id: String, transfer_id: String) -> Result<(), CoreError>;
async fn cancel_transfer(client_operation_id: String, transfer_id: String) -> Result<(), CoreError>;
async fn clear_completed_transfers(client_operation_id: String) -> Result<u64, CoreError>;
async fn replace_transfer_sources(req: ReplaceSourcesRequest) -> Result<(), CoreError>;
async fn replace_receive_destination(req: ReplaceReceiveDestinationRequest)
    -> Result<(), CoreError>;

async fn create_group(req: CreateGroupRequest) -> Result<CreateGroupResult, CoreError>;
async fn decide_group_invite(req: GroupInviteDecisionRequest) -> Result<(), CoreError>;
async fn update_group(req: UpdateGroupRequest) -> Result<(), CoreError>;
async fn leave_group(client_operation_id: String, group_id: String) -> Result<(), CoreError>;

async fn request_own_device_binding(client_operation_id: String, peer_device_id: String)
    -> Result<String, CoreError>; // binding_id
async fn decide_own_device_binding(req: BindingDecisionRequest) -> Result<(), CoreError>;
async fn remove_own_device_binding(client_operation_id: String, peer_device_id: String)
    -> Result<(), CoreError>;
async fn set_clipboard_mode(client_operation_id: String, peer_device_id: String, mode: ClipboardMode)
    -> Result<(), CoreError>;
async fn submit_local_clipboard(req: SubmitClipboardRequest) -> Result<SendMessageResult, CoreError>;

async fn update_settings(client_operation_id: String, patch: SettingsPatch)
    -> Result<AppSettingsView, CoreError>;
async fn delete_conversation(client_operation_id: String, conversation_id: String)
    -> Result<(), CoreError>;
async fn mark_conversation_read(client_operation_id: String, conversation_id: String, through_sort_order: i64)
    -> Result<(), CoreError>;
```

### 主要命令 DTO

```rust
struct SendTextRequest {
    client_operation_id: String,
    conversation_id: String,
    text: String,
}

struct SendSourcesRequest {
    client_operation_id: String,
    conversation_id: String,
    message_kind: MessageKind, // file | image | folder | clipboard_image
    display_name: String,
    sources: Vec<SourceItem>,
}

struct SendClipboardImageRequest {
    conversation_id: String,
    display_name: String,
    sources: Vec<SourceItem>, // 只能包含一个 file，最大 20 MiB
    content_fingerprint: String, // 64 字符小写 SHA-256
    automatic: bool,
}

struct SourceItem {
    entry_kind: TransferEntryKind,
    source_ref: Option<String>, // file 必填；directory 可以为空
    relative_path: String,
    size: u64,
    modified_at_ms: i64,
}

struct OfferDecisionRequest {
    client_operation_id: String,
    transfer_id: String,
    decision: OfferDecision, // accept | reject
    receive_base_ref: Option<String>,
}

struct CreateGroupRequest {
    client_operation_id: String,
    name: String,
    member_device_ids: Vec<String>,
}

struct UpdateGroupRequest {
    client_operation_id: String,
    group_id: String,
    expected_revision: u64,
    name: Option<String>,
    add_member_ids: Vec<String>,
    remove_member_ids: Vec<String>,
    transfer_owner_to: Option<String>,
    disband: bool,
}

struct SubmitClipboardRequest {
    client_operation_id: String,
    peer_device_id: String,
    content: ClipboardContent, // Text(String) | Image(SourceItem)
    automatic: bool,
}

enum ClipboardContent {
    Text(String),
    Image(SourceItem),
}

enum OfferDecision { Accept, Reject }
enum InviteDecision { Accept, Reject }
enum BindingDecision { Accept, Reject }

struct ReplaceSourcesRequest {
    client_operation_id: String,
    transfer_id: String,
    sources: Vec<SourceItem>,
}

struct GroupInviteDecisionRequest {
    client_operation_id: String,
    invite_id: String,
    decision: InviteDecision,
}

struct BindingDecisionRequest {
    client_operation_id: String,
    binding_id: String,
    decision: BindingDecision,
}

struct SendMessageResult {
    message_id: String,
    transfer_ids: Vec<String>, // 文字为空；群文件按接收成员返回多个
}

struct CreateGroupResult {
    group_id: String,
    conversation_id: String,
    invite_ids: Vec<String>,
}

enum Patch<T> { Unchanged, Set(T), Clear }

struct SettingsPatch {
    default_receive_policy: Option<ReceivePolicy>,
    default_receive_ref: Patch<String>,
    notifications_enabled: Option<bool>,
    close_to_tray: Option<bool>,
    start_on_boot: Option<bool>,
    android_keep_online: Option<bool>,
    auto_open_receive_directory: Option<bool>,
    log_level: Option<LogLevel>,
}

struct PageRequest {
    offset: u64,
    limit: u32, // 1..=100
}

struct Page<T> {
    items: Vec<T>,
    total_count: u64,
    next_offset: Option<u64>,
}
```

`UpdateGroupRequest` 一次只能执行一种管理动作：资料修改、成员增删、转让或解散。出现互斥字段组合时同步返回 `INVALID_ARGUMENT`。

`SourceItem.entry_kind=file` 时 `source_ref` 必填且 size/mtime 必须来自实际源文件；`directory` 时 size 必须为 0，文件夹根目录应保存 `source_ref` 以支持发送完成后的“打开/显示位置”，其他目录可为空。文件夹消息必须显式包含根目录和所有空目录，父目录排在子项之前。

`clear_completed_transfers` 只删除状态为 completed 的传输详情，返回删除数量；不得删除消息记录、源文件或已接收文件。

`ReplaceSourcesRequest.sources` 必须与原 transfer 的 `relative_path`、size 和修改时间逐项匹配；只允许更新 `source_ref`。任何元数据不一致都返回 `TRANSFER_SOURCE_CHANGED`，用户需要创建新发送任务。

## CoreEvent 事件流

```rust
enum CoreEvent {
    CoreReady { snapshot: AppSnapshot },
    AppSnapshotInvalidated,
    PeerPresenceChanged { peer: PeerView },
    ConversationChanged { conversation: ConversationSummary },
    MessageChanged { message: MessageView },
    TransferProgress { progress: TransferProgressView },
    IncomingOfferRequiresDecision { offer: IncomingOfferView },
    GroupInviteReceived { invite: GroupInviteView },
    OwnDeviceBindingRequested { binding: BindingRequestView },
    PlatformRequest { request: PlatformRequest },
    NotificationRequested { notification: NotificationView },
    CoreErrorOccurred { error: CoreError, context: ErrorContext },
}
```

进度事件字段：

```rust
struct TransferProgressView {
    transfer_id: String,
    state: TransferState,
    persisted_bytes: u64,
    total_size: u64,
    bytes_per_second: u64,
    eta_seconds: Option<u64>,
    active_entry_relative_path: Option<String>,
}
```

UI 不根据瞬时进度自行改变权威状态；状态变化必须来自 `MessageChanged`/`TransferProgress.state` 或重新查询快照。

## 平台请求接口

Rust 通过 `PlatformRequest` 请求一次平台能力，Dart/Kotlin/Windows 适配器完成后调用统一结果接口：

```rust
enum PlatformRequest {
    OpenSourceFile {
        request_id: String,
        source_ref: String,
        expected_size: u64,
        expected_modified_at_ms: i64,
    },
    CreateReceiveFile {
        request_id: String,
        receive_base_ref: String,
        relative_path: String,
        expected_size: u64,
    },
    CreateReceiveDirectory {
        request_id: String,
        receive_base_ref: String,
        relative_path: String,
        modified_at_ms: i64,
    },
    CommitReceiveFile {
        request_id: String,
        partial_ref: String,
        desired_relative_path: String,
        modified_at_ms: i64,
    },
    DeletePartialFile {
        request_id: String,
        partial_ref: String,
    },
    ApplyClipboardText {
        request_id: String,
        text: String,
        origin_device_id: String,
        clipboard_sequence: u64,
    },
    ApplyClipboardImage {
        request_id: String,
        file_ref: String,
        origin_device_id: String,
        clipboard_sequence: u64,
    },
}

enum PlatformRequestResult {
    SourceFileOpened {
        owned_fd: i64,
        actual_size: u64,
        actual_modified_at_ms: i64,
    },
    ReceiveFileCreated {
        owned_fd: i64,
        partial_ref: String,
        actual_relative_path: String,
    },
    ReceiveDirectoryCreated {
        destination_ref: String,
        actual_relative_path: String,
    },
    ReceiveFileCommitted {
        final_ref: String,
        final_display_name: String,
    },
    PartialFileDeleted,
    ClipboardApplied,
    Failed { error: CoreError },
}

async fn complete_platform_request(
    request_id: String,
    result: PlatformRequestResult,
) -> Result<(), CoreError>;
```

文件打开成功结果包含由平台 `detach` 后交给 Rust 所有权的整数 FD、可选最终引用和实际元数据。Rust 收到成功结果后负责关闭 FD；平台不得二次关闭。平台请求默认 15 秒超时，等待用户选择目录的请求不使用该超时，由 Offer 流程单独保持。

## 视图 DTO

### AppSnapshot

```rust
struct AppSnapshot {
    local_profile: LocalProfileView,
    settings: AppSettingsView,
    nearby_peers: Vec<PeerView>,
    conversation_summaries: Vec<ConversationSummary>,
    active_transfer_count: u32,
    pending_user_action_count: u32,
    core_health: CoreHealthView,
}

struct LocalProfileView {
    device_id: String,
    device_name: String,
    platform: Platform,
}

struct AppSettingsView {
    default_receive_policy: ReceivePolicy,
    default_receive_ref: Option<String>,
    notifications_enabled: bool,
    close_to_tray: bool,
    start_on_boot: bool,
    android_keep_online: bool,
    auto_open_receive_directory: bool,
    log_level: LogLevel,
}

struct CoreHealthView {
    state: CoreHealthState,
    discovery_available: bool,
    tcp_listener_available: bool,
    last_error: Option<CoreError>,
}
```

### MessageView

```rust
struct MessageView {
    message_id: String,
    conversation_id: String,
    sender_device_id: String,
    kind: MessageKind,
    state: MessageState,
    text_content: Option<String>,
    display_name: Option<String>,
    total_size: Option<u64>,
    entry_count: Option<u32>,
    local_file_ref: Option<String>, // completed 后才返回；文件夹指向根目录
    created_at_ms: i64,
    local_sort_order: i64,
    deliveries: Vec<DeliveryView>,
    transfer_summaries: Vec<TransferSummaryView>,
}

struct DeliveryView {
    recipient_device_id: String,
    state: DeliveryState,
    failure_reason: Option<TransferFailureReason>,
    updated_at_ms: i64,
}

struct TransferSummaryView {
    transfer_id: String,
    peer_device_id: String,
    direction: TransferDirection,
    state: TransferState,
    persisted_bytes: u64,
    total_size: u64,
    failure_reason: Option<TransferFailureReason>,
}
```

### PeerView

```rust
struct PeerView {
    device_id: String,
    device_name: String,
    platform: Platform,
    relation: PeerRelation,
    presence: Presence,
    last_seen_at_ms: Option<i64>,
    effective_receive_policy: ReceivePolicy,
    clipboard_mode: ClipboardMode,
}

struct ConversationSummary {
    conversation_id: String,
    kind: ConversationKind,
    state: ConversationState,
    title: String,
    peer_device_id: Option<String>,
    group_id: Option<String>,
    last_message_id: Option<String>,
    last_message_preview: Option<String>,
    last_activity_at_ms: i64,
    unread_count: u32,
    active_transfer_count: u32,
}

struct TransferView {
    transfer_id: String,
    message_id: String,
    conversation_id: String,
    peer_device_id: String,
    direction: TransferDirection,
    state: TransferState,
    failure_reason: Option<TransferFailureReason>,
    display_name: String,
    total_size: u64,
    persisted_bytes: u64,
    entry_count: u32,
    receive_base_ref: Option<String>,
    local_file_ref: Option<String>, // completed 后才返回；文件夹指向根目录
    paused_by_user: bool,
}

struct IncomingOfferView {
    transfer: TransferView,
    sender: PeerView,
    suggested_receive_ref: Option<String>,
}

struct GroupInviteView {
    invite_id: String,
    group_id: String,
    group_name: String,
    inviter: PeerView,
    member_device_ids: Vec<String>,
    created_at_ms: i64,
}

struct BindingRequestView {
    binding_id: String,
    peer: PeerView,
    created_at_ms: i64,
}

struct NotificationView {
    notification_id: String,
    title: String,
    body: String,
    conversation_id: Option<String>,
    transfer_id: Option<String>,
    action_ids: Vec<String>,
}

struct ErrorContext {
    operation: String,
    conversation_id: Option<String>,
    message_id: Option<String>,
    transfer_id: Option<String>,
    peer_device_id: Option<String>,
}
```

任何 UI 必需字段若不在 DTO 中，应先修改本文档再实现，不允许页面直接查询 SQLite 补字段。

## 错误码

`CoreError` 结构：

```rust
struct CoreError {
    code: ErrorCode,
    message: String,             // 面向开发者的短说明
    user_message: String,        // 默认中文用户文案
    recoverable: bool,
    related_id: Option<String>,
}
```

权威错误码：

| Code | 默认用户文案 | 可恢复 |
|---|---|:---:|
| `INVALID_ARGUMENT` | 输入内容不正确 | 否 |
| `NOT_READY` | 服务仍在启动，请稍后重试 | 是 |
| `ALREADY_EXISTS` | 该项目已经存在 | 否 |
| `NOT_FOUND` | 找不到对应记录 | 否 |
| `CONFIG_DIRECTORY_UNWRITABLE` | 应用数据目录不可写 | 是 |
| `STORAGE_OPEN_FAILED` | 无法打开本地数据库 | 是 |
| `STORAGE_MIGRATION_FAILED` | 本地数据升级失败 | 是 |
| `STORAGE_WRITE_FAILED` | 无法保存本地状态 | 是 |
| `NETWORK_UDP_BIND_FAILED` | 无法启动附近设备发现 | 是 |
| `NETWORK_TCP_BIND_FAILED` | 无法启动接收服务 | 是 |
| `NETWORK_CONNECT_FAILED` | 暂时无法连接设备 | 是 |
| `NETWORK_CONNECTION_LOST` | 连接已中断，将自动重试 | 是 |
| `UNSUPPORTED_PROTOCOL` | 对方应用版本不兼容 | 否 |
| `PROTOCOL_INVALID_FRAME` | 收到无法识别的数据 | 否 |
| `PROTOCOL_LIMIT_EXCEEDED` | 发送内容超过协议限制 | 否 |
| `PEER_OFFLINE` | 设备离线，将在上线后继续 | 是 |
| `PEER_UNKNOWN` | 找不到该设备 | 是 |
| `MESSAGE_TOO_LARGE` | 消息内容过大 | 否 |
| `GROUP_FULL` | 群成员已达到 32 台设备 | 否 |
| `GROUP_NOT_FOUND` | 群聊不存在 | 否 |
| `NOT_GROUP_OWNER` | 只有群主可以执行此操作 | 否 |
| `GROUP_OWNER_MUST_TRANSFER` | 群主需要先转让群主或解散群聊 | 是 |
| `GROUP_REVISION_CONFLICT` | 群资料正在同步，请稍后重试 | 是 |
| `TRANSFER_REJECTED` | 对方拒绝了此次传输 | 否 |
| `TRANSFER_SOURCE_CHANGED` | 源文件已变化，请重新选择 | 是 |
| `TRANSFER_CANCELLED` | 传输已取消 | 否 |
| `FILE_NOT_FOUND` | 找不到源文件 | 是 |
| `FILE_OPEN_FAILED` | 无法打开文件 | 是 |
| `FILE_WRITE_FAILED` | 无法写入接收文件 | 是 |
| `FILE_NOT_ENOUGH_SPACE` | 存储空间不足 | 是 |
| `FILE_INVALID_PATH` | 文件路径无效 | 否 |
| `FILE_PERMISSION_LOST` | 文件访问权限已失效，请重新选择 | 是 |
| `PLATFORM_REQUEST_TIMEOUT` | 系统操作超时，请重试 | 是 |
| `PLATFORM_UNSUPPORTED` | 当前系统不支持此操作 | 否 |
| `CLIPBOARD_EMPTY` | 剪贴板中没有可发送的内容 | 是 |
| `CLIPBOARD_TOO_LARGE` | 剪贴板内容过大，请改用文件发送 | 否 |
| `CLIPBOARD_BINDING_REQUIRED` | 请先将对方设为“我的设备” | 是 |
| `CLIPBOARD_BACKGROUND_READ_BLOCKED` | Android 后台不能自动读取剪贴板 | 否 |
| `INTERNAL_ERROR` | 发生内部错误，请重试 | 是 |

`INTERNAL_ERROR` 只能用于真正无法分类的错误；新增可预期失败时必须先在本文档增加具体错误码和测试。

## 事务与幂等示例

### 发送文字

```text
BEGIN IMMEDIATE
  如果 client_operation_id 已存在：返回已保存 result_json
  分配 message_id 和 local_sort_order
  INSERT messages(state=queued)
  为每个目标 INSERT message_deliveries(state=queued)
  为每个目标构造精确 JSON 并 INSERT outbox
  UPDATE conversations last_message_id/last_activity_at_ms
  INSERT client_operations(result_json={message_id})
COMMIT
发布 MessageChanged
```

### 收到文件完成

```text
数据读完
  -> flush/平台 commit/原子改名
BEGIN IMMEDIATE
  UPDATE transfer_entries = completed
  UPDATE transfers = completed, persisted_bytes = total_size
  UPDATE message_deliveries = completed
  重新计算 messages.state
  INSERT processed_events / 完成回执 outbox
COMMIT
发布 TransferProgress(completed) 与 MessageChanged
```

文件发布失败时不得提交 completed。

### 接收失败与更换目录

接收数据路径必须按操作所属一侧分类错误：写入或 `sync` 返回磁盘满时为 `failed/not_enough_space`；SAF/文件系统授权失效时为 `failed/permission_lost`；目标路径消失时为 `failed/invalid_path`。TCP 读写失败才回到 `queued/connection_error`。

`replace_receive_destination` 只接受上述三种可恢复的接收失败。核心重新准备所有目标和 partial，以新目标中实际存在的安全长度生成 `transfer_resume.entries`，在同一事务内更新路径、偏移、状态和 outbox。新目标没有 partial 时偏移必须归零。完成所有文件的 `flush/commit/rename` 前不得写 `completed` 或发送完成回执。

文件夹逐项发布时，每个成功发布的文件立即清除其 `partial_ref`。进程在部分发布后退出，重启应识别“目标文件存在、长度正确且 partial 不存在”的条目并继续剩余条目，不得另取重名文件名或重复发布。

## 迁移规则

- 使用单调 schema version；每个版本一个事务迁移。
- 迁移前备份数据库文件头信息和当前版本，不复制大型 WAL 到用户目录。
- 迁移失败必须回滚并停止核心启动，不得部分使用新旧表。
- 不允许在应用运行中执行破坏性 DDL。
- 开发阶段可以重建测试数据库，正式版本禁止“删除数据库解决迁移问题”。

## 异常情况

- 本地 group sequence 达到 SQLite 整数上限：返回 `INTERNAL_ERROR` 并禁止新建群，现有群仍可用。
- UUID 生成器失败：命令失败且不写数据库，不使用时间戳字符串代替。
- client operation 已存在但命令参数不同：返回首次结果并记录诊断警告，不执行第二次。
- 消息删除时仍有活动 transfer：同一事务先取消任务和 outbox，再软删除消息。
- 数据库 offset 与 partial 大小不一致：按协议规则修正，不直接标记完成。
- Android URI token 仍在数据库但系统授权丢失：映射为 `FILE_PERMISSION_LOST`。
- UI 收到未知枚举字符串：视为客户端/核心版本错误，记录并刷新快照，不猜测显示。

## 实现检查表

- [ ] 所有稳定 ID 严格按本文格式生成和比较。
- [ ] 所有枚举只有本文列出的值。
- [ ] 非法传输状态转换被核心拒绝并记录。
- [ ] 当前 Schema Version 3 DDL 可在空数据库一次执行成功，历史 V1/V2 数据可无损迁移。
- [ ] 每次数据库连接都启用 foreign keys、WAL、NORMAL synchronous 和 busy timeout。
- [ ] 消息、delivery 和 outbox 在同一事务创建。
- [ ] 远端事件落库后才发送 stored 回执。
- [ ] 所有有副作用 FFI 命令通过 ClientOperationId 幂等。
- [ ] 页面只消费 DTO，不直接访问 SQLite。
- [ ] Android FD 所有权只转移一次并最终关闭。
- [ ] UI 错误提示由权威错误码映射，不直接显示协议 error 文案。
- [ ] completed 只在最终文件发布成功后提交。
