# 网络协议规范

## 目的

本文档是 V1 网络报文、字段、时序、超时、限制和兼容策略的唯一权威来源。发送端和接收端必须严格使用相同定义，不得根据 UI 状态自行增加网络消息。

## 前置知识

- 已阅读 [总体架构](02-architecture.md)。
- 理解 TCP 是连续字节流，一次 `read` 不等于一个完整消息。
- 理解所有业务事件必须先持久化，再发送确认回执。
- ID、数据库状态和错误码定义见 [数据与 API](04-data-and-api.md)。

## 协议范围

V1 只支持：

- 同一 IPv4 子网。
- UDP 广播发现。
- TCP 明文控制连接。
- TCP 明文文件数据连接。
- 协议版本 `1`。

固定端口：

| 用途 | 协议 | 端口 |
|---|---|---:|
| 设备发现 | UDP/IPv4 | 53317 |
| 控制与数据监听 | TCP/IPv4 | 53318 |

应用必须使用单实例锁，避免本机多个进程争用固定端口。端口无法绑定时按 [错误码](04-data-and-api.md#错误码) 报错，不自动换随机端口，否则附近设备无法使用统一发现规则。

## 通用编码规则

- JSON 使用 UTF-8，不带 BOM。
- 字段名使用 `snake_case`。
- ID 使用 [数据文档](04-data-and-api.md#稳定标识) 规定的字符串格式。
- 时间使用 UTC Unix 毫秒整数 `i64`，字段后缀 `_at_ms`。
- 字节数和文件偏移使用非负 `u64` JSON 整数；V1 的 Dart/Rust 模型不得转成浮点数。
- 未知 JSON 字段必须忽略，以便小版本向前兼容。
- 缺少必填字段、类型错误、数值越界或未知 `type` 必须拒绝该消息。
- 不使用 `null` 表示缺省；可选字段不存在时直接省略。
- 字符串按 Unicode 原样传输，比较 ID 和协议枚举时只接受规定的小写 ASCII。

## UDP 设备发现

### Socket 行为

- 绑定 `0.0.0.0:53317`，开启 `SO_REUSEADDR` 和 `SO_BROADCAST`。
- 向每个活动 IPv4 网卡的定向广播地址发送，同时向 `255.255.255.255:53317` 发送一次兜底广播。
- 不向回环、断开、VPN 和无广播能力的接口发送。
- UDP 报文上限 1,200 字节；超过直接丢弃。
- 收到来自本机 `DeviceId` 的报文直接忽略。

### 广播节奏

- 启动、用户点击刷新、网络接口变化时立即发送 `discover` 和 `announce`。
- 应用可被发现期间每 2 秒发送一次 `announce`。
- 设备连续 7 秒没有 `announce` 且没有存活控制连接时标记离线。
- Windows 托盘运行时持续可被发现。
- Android 仅在应用前台或前台服务活动期间可被发现；没有活动服务时视为离线。

### discover

```json
{
  "version": 1,
  "type": "discover",
  "request_id": "01954185-a0b0-7b21-9e85-b74251454b34",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779926400123
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `version` | 是 | 固定为 1 |
| `type` | 是 | 固定为 `discover` |
| `request_id` | 是 | 本次扫描 UUIDv7 |
| `sender_device_id` | 是 | 发送设备 ID |
| `sent_at_ms` | 是 | 发送时间，仅用于日志，不用于安全判断 |

收到合法 `discover` 后，在 0-200ms 随机延迟内向来源 IP:53317 单播一个 `announce`，避免多设备同时响应造成突发。

### announce

```json
{
  "version": 1,
  "type": "announce",
  "device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "device_name": "书房电脑",
  "platform": "windows",
  "app_version": "0.1.0",
  "protocol_min": 1,
  "protocol_max": 1,
  "tcp_port": 53318,
  "capabilities": ["text", "file", "folder", "clipboard"],
  "sent_at_ms": 1779926400123
}
```

约束：

- `device_name`：1-32 个 Unicode 字符，UTF-8 最多 128 字节。
- `platform`：`windows`、`android` 或 `linux`。Linux 标识从应用 0.2.0 起支持；旧客户端会忽略无法解析的平台值，跨 Linux 通信要求各端更新到 0.2.0 或以上。帧格式和协议版本仍为 1。
- `tcp_port`：V1 必须为 53318。
- `capabilities`：未知值忽略；缺少某能力时 UI 禁用对应入口。
- 来源 IP 以 UDP 数据报真实来源为准，不接受 JSON 中提供 IP。

## TCP 连接分类

所有入站 TCP 连接先读取一个长度前缀 JSON 首帧。首帧 `type` 只能是：

- `hello`：长期控制连接。
- `data_hello`：短期文件数据连接。

连接建立超时 3 秒，首帧读取超时 3 秒。超时或首帧非法立即关闭连接。

## 长度前缀 JSON 帧

控制连接和数据连接的 JSON 头使用同一帧格式：

```text
+----------------------+-----------------------------+
| 4 字节 u32 大端长度 N | N 字节 UTF-8 JSON          |
+----------------------+-----------------------------+
```

规则：

- 长度不包含自身 4 字节。
- 普通控制帧最大 1MiB。
- `file_offer` 和群全量同步帧最大 16MiB。
- 长度为 0、超过上限或 JSON 非法时关闭连接并记录协议错误。
- 读取必须使用 `read_exact(4)` 后再 `read_exact(N)`；禁止假定一次 Socket read 返回完整帧。
- 写入端必须把长度和 JSON 放入同一有序写队列，禁止多个任务并发直接写同一 Socket。

## 控制连接握手

### hello

连接发起方建立 TCP 后立即发送：

```json
{
  "version": 1,
  "type": "hello",
  "connection_id": "01954185-a0b0-7b21-9e85-b74251454b34",
  "initiator_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "device_name": "书房电脑",
  "platform": "windows",
  "app_version": "0.1.0",
  "protocol_min": 1,
  "protocol_max": 1,
  "sent_at_ms": 1779926400123
}
```

接收方校验存在共同版本后返回：

```json
{
  "version": 1,
  "type": "hello_ack",
  "connection_id": "01954185-a0b0-7b21-9e85-b74251454b34",
  "device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "device_name": "小米手机",
  "platform": "android",
  "selected_protocol": 1,
  "received_at_ms": 1779926400150
}
```

若没有共同版本，返回 `protocol_error`，其中 `code` 为 `UNSUPPORTED_PROTOCOL`，然后关闭连接。

### 重复控制连接选择

同一对 DeviceId 可能同时建立多条连接。双方对所有已完成 Hello 的连接计算相同排序键：

```text
(initiator_device_id 按 ASCII 升序, connection_id 按 ASCII 升序)
```

保留排序键最小的一条，向其他连接发送 `duplicate_connection` 后关闭。若当前只有一条连接，即使发起方 ID 较大也先使用；第二条出现后再统一选择。业务 outbox 属于设备，不属于某个 Socket，切换连接不得丢消息。

### 保活与离线

- 空闲控制连接每 10 秒发送 `ping`。
- `ping` 包含 `ping_id` 和 `sent_at_ms`；对端立即返回同 ID 的 `pong`。
- 任意合法控制帧都视为活动。
- 30 秒没有收到任何合法帧则关闭连接并标记网络不可达。
- TCP 关闭后，设备在线状态仍可由最近 7 秒 UDP announce 维持；需要发送时立即重连。

保活帧的完整结构：

```json
{
  "version": 1,
  "type": "ping",
  "ping_id": "01954185-a0b0-7b21-9e85-b74251454b34",
  "sent_at_ms": 1779926400123
}
```

`pong` 使用相同字段，把 `type` 改为 `pong`；不得生成新的 `ping_id`。

关闭重复连接前发送：

```json
{
  "version": 1,
  "type": "duplicate_connection",
  "kept_connection_id": "01954185-a0b0-7b21-9e85-b74251454b34"
}
```

该帧不需要回执，写入后立即关闭被淘汰连接。

## 控制消息信封

除握手、ping/pong 和纯错误帧外，业务消息统一使用：

```json
{
  "version": 1,
  "type": "text_message",
  "event_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779926402123,
  "body": {}
}
```

- `event_id` 是网络幂等键，UUIDv7，永不复用。
- 每个接收设备在数据库中对 `(sender_device_id, event_id)` 建唯一约束。
- 接收顺序固定为：解析 → 基本校验 → 查重 → 事务落库 → 发送 `delivery_receipt`。
- 若事件已存在，不能重复执行业务，但必须重新发送原有结果对应的回执。
- 发送方在收到 `stored` 或最终回执前保留 outbox，重连后重发相同 `event_id` 和完全相同正文。

## 文本消息

### text_message

```json
{
  "version": 1,
  "type": "text_message",
  "event_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779926402123,
  "body": {
    "message_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
    "conversation_id": "p:d_12d4b0835e2a4f77a3b133b23a383f30:d_fa4abdf087224388808abb80852cb1d3",
    "message_kind": "text",
    "text": "今晚把照片传一下",
    "created_at_ms": 1779926402100,
    "group_revision": 4
  }
}
```

约束：

- `message_kind`：这里只允许 `text` 或 `clipboard_text`。
- `text`：1-20,000 个 Unicode 字符；`clipboard_text` 允许最多 1MiB UTF-8。
- 私聊不发送 `group_revision`；群聊必须发送创建消息时本机已知的群版本。
- 群发送方为每个其他有效成员创建独立 outbox，但复用同一个 `message_id`，每份网络事件有独立 `event_id`。

## 文件 Offer

文件、图片、文件夹和剪贴板图片都使用 `file_offer`。文件清单不包含内容哈希。

```json
{
  "version": 1,
  "type": "file_offer",
  "event_id": "01954187-24f0-7777-a476-acde909b55be",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779926405000,
  "body": {
    "message_id": "01954187-24f0-7777-a476-acde909b55be",
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "conversation_id": "g:d_12d4b0835e2a4f77a3b133b23a383f30:7",
    "message_kind": "folder",
    "display_name": "家庭相册",
    "total_size": 15516245,
    "entry_count": 4,
    "created_at_ms": 1779926404900,
    "group_revision": 4,
    "entries": [
      {
        "entry_id": "01954187-2520-76cc-8fd1-f6d4faf5b1d0",
        "entry_kind": "directory",
        "relative_path": "家庭相册",
        "size": 0,
        "modified_at_ms": 1767225600000
      },
      {
        "entry_id": "01954187-2525-70aa-b78c-f6ab2cb587b8",
        "entry_kind": "directory",
        "relative_path": "家庭相册/2026",
        "size": 0,
        "modified_at_ms": 1767225600000
      },
      {
        "entry_id": "01954187-2530-7e6e-91bc-cdb9e27b48ac",
        "entry_kind": "file",
        "relative_path": "家庭相册/2026/01.jpg",
        "size": 6388123,
        "modified_at_ms": 1767225600000
      },
      {
        "entry_id": "01954187-2540-7b0e-a7d6-ad256a4daf44",
        "entry_kind": "file",
        "relative_path": "家庭相册/2026/02.jpg",
        "size": 9128122,
        "modified_at_ms": 1767225610000
      }
    ]
  }
}
```

约束：

- `message_kind`：`file`、`image`、`folder` 或 `clipboard_image`。
- `entry_count` 必须等于 `entries.length`，范围 1-20,001，文件和目录都计数；其中 file entry 最多 10,000 个，directory entry 另计。
- `entry_kind` 只允许 `file` 或 `directory`。
- `directory` 的 `size` 必须为 0；`total_size` 必须等于所有 `file` entry 的 size 之和，使用检查溢出的 u64 加法。
- 单个 `relative_path` UTF-8 不超过 1,024 字节，必须使用 `/` 分隔。
- 禁止空路径、绝对路径、盘符、`.`、`..`、NUL 和平台保留路径片段。
- `file`、`image`、`clipboard_image` 必须只有一个 `file` entry。
- `folder` 必须包含一个代表根目录的 `directory` entry；所有父目录必须显式列出并排在子项之前，第一段路径等于 `display_name` 清理后的根目录名。
- Offer JSON 超过 16MiB或条目超过 10,000 时，发送命令在本机失败，不上网。
- 群聊必须带 `group_revision`；私聊省略。
- `clipboard_image` 必须额外携带 `origin_device_id`、`clipboard_sequence`、`content_fingerprint` 和 `automatic`；其他消息类型必须省略这四个字段。
- `origin_device_id` 必须等于 `sender_device_id`，`clipboard_sequence` 必须大于 0，`content_fingerprint` 是 64 字符小写十六进制 SHA-256。
- SHA-256 只对不超过 20 MiB 的剪贴板图片计算，用于跨重启去重和回环抑制；普通文件、普通图片和文件夹仍不计算整体或分块哈希。
- 同一条群剪贴板图片消息发给多个成员时，所有成员复用同一个 origin、sequence 和 fingerprint，但各自拥有独立 event 和 transfer。

接收方先落库，再根据设备策略：

- 自动接收：发送 `file_accept`。
- 每次询问或未知设备：发送 `delivery_receipt(stage="stored")`，等待用户决定。
- 明确拒绝：发送 `file_reject`。

## 接受与拒绝

### file_accept

```json
{
  "version": 1,
  "type": "file_accept",
  "event_id": "01954188-1100-7659-b7ba-5f6c8fef872a",
  "sender_device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "sent_at_ms": 1779926410000,
  "body": {
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "entries": [
      {"entry_id": "01954187-2530-7e6e-91bc-cdb9e27b48ac", "offset": 0},
      {"entry_id": "01954187-2540-7b0e-a7d6-ad256a4daf44", "offset": 0}
    ]
  }
}
```

`file_accept.entries` 只列 `file` entry。`offset` 是接收端已经安全写入并持久化的连续前缀长度，不得大于 entry size。首次接受通常为 0；若之前已有合法 `.partial` 和数据库记录，可以非零。接收端必须在发送 accept 前创建清单中的目录；纯空目录任务创建完成后可直接发送 completed 回执，不建立数据连接。

### file_reject

```json
{
  "version": 1,
  "type": "file_reject",
  "event_id": "01954188-2200-7c23-88df-1ae21954681c",
  "sender_device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "sent_at_ms": 1779926410200,
  "body": {
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "reason": "user_rejected"
  }
}
```

`reason` 只允许：`user_rejected`、`not_enough_space`、`invalid_path`、`permission_lost`、`unsupported`。详细本机错误不通过网络泄露。

`file_reject` 不仅用于接受前拒绝。接收端已经发送 `file_accept` 后，如果写入中途发生磁盘满、目标路径失效或 SAF 权限丢失，也必须用同一消息可靠通知发送端。发送端收到后立即停止该任务的数据连接，并把任务置为对应的 `failed` 终态；不得继续按 `connection_error` 无限重连。用户在接收端更换目录或恢复权限后，由接收端发送带真实安全偏移的 `transfer_resume`，双方再回到 `accepted` 并继续传输。

## 数据连接

### data_hello

发送端收到 `file_accept` 后建立新的 TCP 连接，首帧为：

```json
{
  "version": 1,
  "type": "data_hello",
  "connection_id": "01954189-3000-71d9-8328-649feace2118",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
  "protocol": 1
}
```

接收方必须确认：

- 发送 DeviceId 与 Offer 一致。
- transfer 存在且已经接受。
- 当前没有另一条活动数据连接占用该 transfer。

合法时回复 `data_hello_ack`；非法时回复 `protocol_error` 并关闭。

```json
{
  "version": 1,
  "type": "data_hello_ack",
  "connection_id": "01954189-3000-71d9-8328-649feace2118",
  "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
  "accepted_at_ms": 1779926410300
}
```

### 文件记录

握手后，发送方按 `file_accept.entries` 顺序对每个尚未完成的 entry 发送：

```text
[4 字节 JSON 长度]
[data_entry JSON]
[data_length 个原始文件字节]
[下一条 data_entry JSON]
[下一段原始字节]
...
```

`data_entry` 示例：

```json
{
  "version": 1,
  "type": "data_entry",
  "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
  "entry_id": "01954187-2530-7e6e-91bc-cdb9e27b48ac",
  "offset": 1048576,
  "data_length": 5339547
}
```

其中：

- 只有 `file` entry 可以发送 `data_entry`。`data_length = entry.size - offset`，V1 一次发送该文件的全部剩余连续字节。
- 接收方读取原始数据时严格计数；读满 `data_length` 后才能把后续 4 字节解释为新 JSON 长度。
- TCP EOF 出现在数据未读满时属于可恢复中断，保留已持久化连续偏移。
- 发送完成后发送一个长度前缀 `data_end` JSON 帧，然后发送方半关闭写方向。
- 接收方完成刷新和改名后返回长度前缀 `data_complete`，再关闭连接。

### data_end

```json
{
  "version": 1,
  "type": "data_end",
  "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
  "entry_count": 2,
  "total_bytes_sent": 14467669
}
```

`data_end.entry_count` 是本次数据连接实际发送的 file entry 数，`total_bytes_sent` 是本次连接发送的原始字节数；两者不包含已经续传完成的前缀和 directory entry。发送方在发出 `data_end` 前再次读取所有源文件 size/mtime，发现变化时改发 `transfer_source_changed` 并关闭数据连接，接收端不得发布 partial。

### data_complete

```json
{
  "version": 1,
  "type": "data_complete",
  "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
  "total_bytes_saved": 15516245,
  "completed_at_ms": 1779926500000
}
```

接收端发送 `data_complete` 后，还需通过控制连接发送最终 `delivery_receipt(stage="completed")`，用于控制连接重连后的可靠状态同步。

## 暂停、继续、取消

### transfer_pause

任一端用户暂停时：

1. 本地先持久化暂停状态。
2. 发送 `transfer_pause`。
3. 停止调度并关闭数据连接。
4. 接收方刷新当前 `.partial`，保存连续偏移。

正文只包含 `transfer_id` 和 `reason`，其中 `reason` 为 `user` 或 `system`。

### transfer_resume

继续时由需要接收数据的一端发送当前偏移：

```json
{
  "version": 1,
  "type": "transfer_resume",
  "event_id": "01954190-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "sent_at_ms": 1779926600000,
  "body": {
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "entries": [
      {"entry_id": "01954187-2530-7e6e-91bc-cdb9e27b48ac", "offset": 104857600},
      {"entry_id": "01954187-2540-7b0e-a7d6-ad256a4daf44", "offset": 0}
    ]
  }
}
```

发送方在打开数据连接前重新检查源文件的大小和修改时间。任一 entry 变化则发送 `transfer_source_changed`，不发送新数据。

### transfer_cancel

取消是最终操作。发送方或接收方先持久化取消，再发送 `transfer_cancel`，正文包含 `transfer_id` 和 `cancelled_by`。收到后停止数据连接并删除接收端 `.partial`；已完成文件不删除。

## 投递回执

```json
{
  "version": 1,
  "type": "delivery_receipt",
  "event_id": "01954191-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "sent_at_ms": 1779926700000,
  "body": {
    "original_event_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
    "message_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "stage": "stored",
    "at_ms": 1779926699900
  }
}
```

字段按场景省略：文字没有 `transfer_id`，纯传输控制可没有 `message_id`。`stage` 允许：

- `stored`：事件已事务落库，发送方可以从 outbox 删除该事件。
- `accepted`：Offer 已接受。
- `rejected`：Offer 被拒绝。
- `completed`：文件已经安全保存或消息完成目标业务动作。
- `cancelled`：任务被对端取消。

回执本身也按 `event_id` 去重，但无需为回执再发送回执，避免无限确认。

## 群资料同步

### 群模型

- `GroupId` 创建后不变。
- `revision` 从 1 开始，每次群资料或成员变化加 1。
- 当前群主是唯一可以产生下一版本全量快照的设备。
- 普通群消息携带发送者当时已知 revision；接收者只要发送者存在于当前或该 revision 可恢复的成员表中即可接收。

### group_invite

邀请包含完整群快照：

```json
{
  "version": 1,
  "type": "group_invite",
  "event_id": "01954200-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779927000000,
  "body": {
    "invite_id": "01954200-1000-7aab-90b6-356329ce1b45",
    "group": {
      "group_id": "g:d_12d4b0835e2a4f77a3b133b23a383f30:7",
      "name": "家里设备",
      "owner_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
      "revision": 1,
      "created_at_ms": 1779926999000,
      "members": [
        {"device_id": "d_12d4b0835e2a4f77a3b133b23a383f30", "role": "owner", "membership": "joined"},
        {"device_id": "d_fa4abdf087224388808abb80852cb1d3", "role": "member", "membership": "invited"}
      ]
    }
  }
}
```

成员数组 2-32 项，DeviceId 不得重复，必须恰好一个 owner 且与 `owner_device_id` 相同。

### group_invite_reply

包含 `invite_id`、`group_id` 和 `decision`，其中 `decision` 为 `accepted` 或 `rejected`。接受后群主产生 revision+1 的 `group_update`，把该成员 membership 改为 `joined`。新成员只加入后续消息的目标列表，不向其创建历史消息或历史文件 outbox。

### group_update

每次发送完整快照，而不是补丁：

```json
{
  "version": 1,
  "type": "group_update",
  "event_id": "01954201-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779927100000,
  "body": {
    "previous_revision": 3,
    "group": {
      "group_id": "g:d_12d4b0835e2a4f77a3b133b23a383f30:7",
      "name": "家庭设备",
      "owner_device_id": "d_fa4abdf087224388808abb80852cb1d3",
      "revision": 4,
      "created_at_ms": 1779926999000,
      "members": [
        {"device_id": "d_12d4b0835e2a4f77a3b133b23a383f30", "role": "member", "membership": "joined"},
        {"device_id": "d_fa4abdf087224388808abb80852cb1d3", "role": "owner", "membership": "joined"}
      ]
    },
    "change_reason": "owner_transferred"
  }
}
```

应用规则：

1. 本地无该群：仅当本机在 members 中且 membership 为 `joined` 时保存，否则拒绝。
2. `revision <= local_revision`：视为重复，重新发送 stored 回执，不覆盖。
3. `previous_revision == local_revision` 且发送者是本地记录的当前 owner：原子应用快照。
4. 转让群主时，发送者必须是旧 owner，快照中恰好一个新 owner。
5. `previous_revision > local_revision`：发送 `group_sync_request`，暂存当前事件但不应用。
6. `previous_revision < local_revision < revision`：同样请求全量同步，避免跳过中间成员变更。
7. 非 owner 的更新返回 `protocol_error(code="NOT_GROUP_OWNER")`。

### group_sync_request / group_sync_response

- request 包含 `group_id` 和 `known_revision`。
- response 由当前群主返回最新完整快照。
- 接收方仅在快照 revision 大于本地且 owner 链可以从已知更新验证时应用。
- 群主离线时等待，不阻塞普通群消息。

### 退出、移除和解散

- 退出：成员发送 `group_leave_request` 给群主；群主生成新快照移除该成员。
- 移除：群主直接生成新快照。
- 解散：`group_update.change_reason = "disbanded"`，快照包含 `disbanded_at_ms`；成员将会话只读。
- 已移除成员发来的新群消息，接收端按本地最新成员表拒绝。

V1 没有加密，以上规则保证正常客户端功能一致，不用于抵御恶意客户端伪造。

## 我的设备绑定

### own_device_bind

```json
{
  "version": 1,
  "type": "own_device_bind",
  "event_id": "01954210-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779927200000,
  "body": {
    "binding_id": "01954210-1000-7aab-90b6-356329ce1b45",
    "requester_name": "书房电脑"
  }
}
```

对端落库并提示用户，使用 `own_device_bind_reply` 返回 `accepted` 或 `rejected`。只有 accepted 后双方将关系标记为 active。任一端使用 `own_device_unbind` 解除，另一端收到后关闭自动剪贴板。

绑定消息不能自动接受，重复 binding ID 不重复弹窗。

## 剪贴板协议

### clipboard_update

剪贴板文字使用：

```json
{
  "version": 1,
  "type": "clipboard_update",
  "event_id": "01954220-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
  "sent_at_ms": 1779927300000,
  "body": {
    "message_id": "01954220-1000-7aab-90b6-356329ce1b45",
    "conversation_id": "p:d_12d4b0835e2a4f77a3b133b23a383f30:d_fa4abdf087224388808abb80852cb1d3",
    "origin_device_id": "d_12d4b0835e2a4f77a3b133b23a383f30",
    "clipboard_sequence": 81,
    "content_type": "text",
    "text": "192.168.1.20",
    "automatic": true
  }
}
```

约束：

- 仅 active 的“我的设备”可使用 `automatic=true`。
- `clipboard_sequence` 是 origin 设备本地永久递增 u64。
- 接收端对 `(origin_device_id, clipboard_sequence)` 去重。
- 接收端写入系统剪贴板后保存“最近远端写入指纹”，平台监听器检测到相同内容时不得产生新的 sequence。
- 图片不内联 JSON，使用 `file_offer(message_kind="clipboard_image")`，并在 body 中携带 origin、sequence、fingerprint 和 automatic。

## 协议错误

```json
{
  "version": 1,
  "type": "protocol_error",
  "code": "PROTOCOL_INVALID_FRAME",
  "related_event_id": "01954186-1d40-7bc6-a587-5f85a4cac626",
  "message": "missing body.conversation_id"
}
```

- `message` 仅用于诊断，最长 256 字节，不显示给普通用户。
- 可恢复的单条业务错误发送 error 后保持控制连接。
- 非法长度、无效 UTF-8、非法 JSON 帧或首帧错误立即关闭当前连接；V1 不维护连续错误计数。
- 对端错误映射到本地权威错误码，不直接信任任意 error 文案。

## 补充业务消息字段

以下消息均使用“控制消息信封”，表中字段位于 `body`。除明确标注外，均需要 event 去重和 stored 回执。

| `type` | body 必填字段 | 字段约束与结果 |
|---|---|---|
| `transfer_pause` | `transfer_id`, `reason` | `reason=user/system`；落库暂停后关闭数据连接 |
| `transfer_cancel` | `transfer_id`, `cancelled_by` | `cancelled_by=sender/receiver`；最终取消并删除未完成 partial |
| `transfer_source_changed` | `transfer_id`, `changed_entry_ids` | entry ID 数组 1-10,000；接收端进入需要重选源文件状态 |
| `group_invite_reply` | `invite_id`, `group_id`, `decision` | `decision=accepted/rejected` |
| `group_sync_request` | `group_id`, `known_revision` | 发送给本地记录的当前 owner |
| `group_sync_response` | `group`, `requested_revision` | `group` 是最新完整快照；`requested_revision` 回显请求值 |
| `group_leave_request` | `group_id`, `known_revision` | 仅普通成员可发送；群主发送时返回 `GROUP_OWNER_MUST_TRANSFER` |
| `own_device_bind_reply` | `binding_id`, `decision` | `decision=accepted/rejected` |
| `own_device_unbind` | `binding_id` | 双方关闭自动剪贴板并把绑定标记 removed |

`transfer_pause` 示例：

```json
{
  "version": 1,
  "type": "transfer_pause",
  "event_id": "01954230-1000-7aab-90b6-356329ce1b45",
  "sender_device_id": "d_fa4abdf087224388808abb80852cb1d3",
  "sent_at_ms": 1779927400000,
  "body": {
    "transfer_id": "01954187-2510-7e20-8d46-52960393fb2e",
    "reason": "user"
  }
}
```

群快照结构严格复用 `group_invite.body.group`。`group_sync_response` 不允许返回部分成员或仅差异字段。以上消息如果引用不存在的实体，返回权威 `NOT_FOUND`/`GROUP_NOT_FOUND` 协议错误；不得新建占位实体猜测恢复。

## 超时与重试

| 操作 | 超时 | 重试策略 |
|---|---:|---|
| TCP connect | 3 秒 | 1、2、4、8、15、30 秒，之后每 30 秒 |
| Hello | 3 秒 | 关闭后按 connect 策略 |
| 控制消息 stored 回执 | 5 秒 | 同连接最多重发 2 次，然后重连 |
| ping/pong | 30 秒无合法帧 | 关闭并重连 |
| data_hello | 3 秒 | 回到等待连接 |
| 数据无读写进展 | 30 秒 | 保存偏移、关闭并重连 |
| 用户接收确认 | 不超时 | 永久等待，直到拒绝或取消 |
| 离线 outbox | 不过期 | 成功、拒绝或用户取消后结束 |

指数退避在网络接口变化、对端新 announce 或用户点击重试时立即归零。

## 发送伪代码

```text
send_business_event(peer, event):
    transaction:
        if event not yet persisted:
            persist domain rows
        insert outbox(peer, event_id, exact_json)
    publish local UI event
    wake connection manager

outbox_worker(peer):
    connection = get_or_connect(peer)
    for item in ordered_pending_outbox(peer):
        write_length_prefixed_json(connection, item.exact_json)
        wait up to 5s for stored receipt
        if receipt received:
            delete outbox item
        else:
            retry same event_id and exact body
```

## 接收伪代码

```text
control_reader(peer, socket):
    loop:
        frame = read_length_prefixed_json(socket)
        validate envelope and body limits
        if event_id already processed:
            resend stored/final receipt
            continue
        transaction:
            apply domain event
            insert processed_event(sender_device_id, event_id)
            record receipt to send
        enqueue receipt
        publish CoreEvent
```

## 文件接收伪代码

```text
receive_data_entry(header, socket):
    transfer = load accepted transfer
    entry = validate header against manifest
    fd = open_or_create_partial(entry)
    seek(fd, header.offset)
    remaining = header.data_length
    while remaining > 0:
        n = socket.read(reused_buffer[0:min(buffer_size, remaining)])
        if n == 0: return recoverable_disconnect
        write_all(fd, n bytes)
        remaining -= n
        update in_memory_offset(n)
        if 1s elapsed or 64MiB advanced:
            flush_as_required()
            persist_contiguous_offset()
    flush(fd)
    verify final size
    atomically_publish_or_platform_commit()
```

## 示例：控制连接重发但不重复消息

1. A 发送 `text_message(event_id=E1)`。
2. B 在事务中保存消息和 E1，发送 stored 回执。
3. 回执在断网时丢失，A 的 outbox 仍保留 E1。
4. 重连后 A 原样重发 E1。
5. B 命中 `(A,E1)` 唯一记录，不再插入消息，只重新发送回执。
6. A 收到回执并删除 outbox。

## 异常情况

- UDP 报文声称的 DeviceId 与现有 TCP Hello 相同但来源 IP 改变：更新候选地址，现有活跃连接优先，不立即中断。
- 两台设备系统时间差很大：不按 `sent_at_ms` 丢弃业务事件，UI 时间可标记为对端时间；顺序使用本地接收序和 UUIDv7辅助。
- 接收 offset 大于源文件大小：发送方返回协议错误并停止该任务。
- `.partial` 实际大小小于数据库 offset：以较小值修正数据库后请求续传。
- `.partial` 实际大小大于数据库 offset：截断到数据库确认偏移，避免发送未持久化字节被误认为完成。
- 群更新缺少中间版本：请求全量同步，普通消息仍可以按已有成员关系排队。
- 一个文件名在 Windows 合法、Android 非法：接收端按平台规则清理最终名称，并保存原始相对路径用于显示。
- 对端发送超限字段：拒绝事件，不为其分配对应大小内存。

## 实现检查表

- [ ] UDP 报文不超过 1,200 字节且只信任真实来源 IP。
- [ ] 所有 TCP JSON 都使用 4 字节大端长度前缀和 `read_exact`。
- [ ] 每条控制连接只有一个顺序写队列。
- [ ] 业务事件先落库再发送 stored 回执。
- [ ] 重发使用相同 event ID 和完全相同正文。
- [ ] 文件数据是连续原始字节，不编码为 JSON/Base64，不逐块确认。
- [ ] 接收偏移始终表示已安全持久化的连续前缀。
- [ ] 源文件变化后停止并要求重新选择。
- [ ] 群资料使用 owner 产生的单调 revision 全量快照。
- [ ] 自动剪贴板只允许双方已确认的“我的设备”。
- [ ] 所有超时、重试和消息大小限制与本文一致。
- [ ] 畸形帧测试不会导致无限分配、死循环或进程崩溃。
