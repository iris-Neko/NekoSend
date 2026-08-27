# 实施指南与任务拆分

## 目的

本文档把 V1 拆成初级、中级程序员可以独立完成和验收的任务。任务必须按依赖顺序推进；不得先做完整 UI，再临时决定协议或把文件字节放进 Dart。

## 前置知识

- 完整阅读 [README](../README.md)、[产品与交互](01-product-ux.md)、[总体架构](02-architecture.md)。
- 网络/存储开发必须同时打开 [网络协议](03-network-protocol.md) 和 [数据与 API](04-data-and-api.md)。
- 性能任务必须使用 [性能规范](06-performance.md) 的测量方法。
- 每个任务对应的测试 ID 见 [测试计划](08-test-plan.md)。

## 实施规范

- 每个任务控制在 0.5-3 个开发日；超过 3 天仍无法验收时继续拆分。
- 每个 PR 只完成一个任务 ID 或同一小节内强相关任务。
- PR 必须包含测试；状态机、解析器和数据库变更不接受纯手工测试。
- 不允许在 PR 中引入文档没有定义的枚举、错误码、协议字段或页面流程。
- 发现规范缺失时先修改权威文档并评审，再写代码。
- 性能门禁失败时停止叠加上层功能，先定位数据路径。

## 建议仓库结构

```text
/
  Cargo.toml                 Rust workspace
  rust-toolchain.toml
  core_rust/                 共享核心库
  perf_harness/              无 Flutter 的双端性能程序
  integration_harness/       多虚拟设备协议测试
  app_flutter/               Windows/Android Flutter 应用
  docs/                      本文档包
  scripts/                   测试与基准脚本
```

首个工程 PR 创建结构，但不得移动 `docs/` 或改变文档链接。

## 里程碑总览

| 里程碑 | 交付结果 | 进入条件 | 退出门禁 |
|---|---|---|---|
| M0 | 工具链、CI、空壳模块 | 文档已确认 | Windows/Android 构建、测试命令通过 |
| M1 | Rust TCP 性能样机 | M0 | 大文件达到 iperf3 90% |
| M2 | 数据库、ID、状态和 FFI | M1 | DDL、迁移、状态机、幂等测试通过 |
| M3 | 自动发现、控制连接、私聊文字 | M2 | 双端离线补发且无重复 |
| M4 | 文件/文件夹、暂停和续传 | M3 | 10GB 断线恢复且不慢于阶段门禁 |
| M5 | Flutter 聊天 UI 与 Windows 集成 | M4 | Windows 完整私聊工作流 |
| M6 | Android SAF、前台服务与恢复 | M5 | Windows ↔ Android 真机工作流 |
| M7 | 永久群聊 | M6 | 32 人模型、邀请和离线群消息通过 |
| M8 | 我的设备与剪贴板 | M7 | 双方绑定、方向和回环抑制通过 |
| M9 | 完整回归与发布候选 | M8 | 全部验收和性能门禁通过 |

## M0：工程与持续集成

### FND-01 Rust workspace

实现步骤：

1. 创建稳定 Rust workspace，包含 `core_rust`、`perf_harness` 和 `integration_harness`。
2. `core_rust` 初始只暴露版本常量和健康检查，不实现网络。
3. 配置 `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test --workspace`。
4. 锁定依赖，提交 `Cargo.lock`。

完成标准：Windows x64 本地和 CI 均能运行三条命令，库没有平台 UI 依赖。

常见错误：把 Flutter 插件代码直接放进核心库；在 workspace 根使用未锁定 git 依赖。

### FND-02 Flutter shell

实现步骤：

1. 创建支持 Windows 和 Android 的 Flutter 应用。
2. 设置 Android `minSdk=33` 和所需 ABI。
3. 接入 `flutter_rust_bridge`，调用核心健康检查。
4. 建立 `application`、`presentation`、`platform` 三层目录。
5. 配置 `dart format --output=none --set-exit-if-changed`、`flutter analyze`、`flutter test`。

完成标准：两个平台显示同一个核心版本号；UI 中没有 Socket、SQL 或文件流代码。

### FND-03 测试基础设施

1. 提供临时目录、临时数据库和固定时间/UUID 生成器测试替身。
2. 提供可创建多个 in-process Core 实例的测试配置，端口可仅在测试中覆盖。
3. 提供故障注入接口：断开 Socket、限制写入、模拟磁盘满、延迟回执。
4. 测试默认不得访问真实用户目录或固定 53317/53318 端口。

完成标准：测试并行执行互不争用端口和文件。

## M1：先证明数据通道性能

### PERF-01 Rust 大文件 CLI

只实现两个命令：

```text
perf_harness receive --listen <ip:port> --output <path>
perf_harness send --connect <ip:port> --file <path>
```

实现步骤：

1. 使用 1MiB 复用缓冲和连续 TCP 字节流。
2. 接收端顺序写、flush，并报告有效吞吐、CPU、峰值内存。
3. 不加入 JSON、SQLite、哈希、压缩和 Flutter。
4. 按性能文档与 `iperf3`、SFTP 做三次中位对比。

完成标准：1GbE 达到 iperf3 字节吞吐 90% 或 105MB/s；否则 M2 不开始。

常见错误：把 Socket `write` 完成当成接收端文件完成；每次循环重新分配 Vec；用零文件/全零稀疏文件制造虚假速度。

### PERF-02 缓冲与系统调用对比

对 64KiB、256KiB、1MiB、4MiB 做相同基准，记录中位速度、CPU 和内存，确认默认 1MiB。只有 profiler 证明更优时才修改权威性能文档。

## M2：核心数据与 API

### CORE-01 ID 和枚举

1. 按数据文档实现所有 newtype ID，解析时严格验证前缀、长度和小写格式。
2. 实现私聊 ID 排序、GroupId 本地序号和 UUIDv7 生成。
3. 实现所有权威枚举的 serde/FFI 映射；未知值返回错误。
4. 使用表驱动测试覆盖合法、边界和非法值。

完成标准：没有业务代码使用裸 String 代替 ID；没有散落枚举字符串。

### CORE-02 传输状态机

1. 把 [状态转换表](04-data-and-api.md#传输状态转换) 实现为单一函数。
2. 输入当前状态、目标状态和原因，返回新状态或 `INVALID_ARGUMENT`。
3. 所有网络、UI、恢复路径都必须调用该函数，不直接赋值。

完成标准：每个允许/禁止转换都有单测。

### DB-01 Schema 与迁移

1. 创建 SQLite 连接初始化和迁移器。
2. 原样实现权威 DDL，启用 PRAGMA。
3. 建立 Repository 接口和专用数据库线程。
4. 提供 schema 重建、迁移失败回滚和外键测试。

完成标准：空库一次迁移成功，重复打开不重复执行；非法外键和 CHECK 被拒绝。

### DB-02 事务用例

按顺序实现并测试：

1. 创建设备资料和设置。
2. 创建/打开私聊。
3. 发送文字的 message + deliveries + outbox + client operation 原子事务。
4. 接收事件的去重 + message + processed event + receipt 原子事务。
5. transfer/entry 创建与 offset 合并写入。

完成标准：在每个 INSERT 中间注入失败，事务均不留下半状态。

### API-01 CoreFacade 和事件流

1. 实现 `start/shutdown/snapshot`。
2. 实现有界事件队列和进度合并。
3. 实现 ClientOperationId 幂等包装器。
4. 生成 Dart 类型并用假 Repository 测试。

完成标准：Flutter 重复命令不重复建消息；事件流重连可用 snapshot 校正。

## M3：发现、连接和文字

### NET-01 UDP 发现

1. 实现严格大小限制的 discover/announce 结构。
2. 枚举活动 IPv4 网卡并广播。
3. 实现立即广播、2 秒 announce、7 秒离线和随机单播响应。
4. 把 UDP 来源 IP 与 peer 合并，不信任 JSON IP。
5. 注入网络接口变化测试重绑。

完成标准：两台真机 3 秒内互现；畸形/超限报文不崩溃。

### NET-02 长度帧编解码

1. 实现 `read_exact` 4 字节大端长度。
2. 分普通 1MiB 与清单 16MiB 上限。
3. 实现严格 UTF-8/JSON 校验和单写队列。
4. 使用随机拆包测试模拟 1 字节到任意长度 TCP 分段。

完成标准：任意拆包/粘包都得到相同帧；超限在分配前拒绝。

### NET-03 控制连接

1. 实现 TCP listener、Hello、版本协商和重复连接排序。
2. 实现 ping/pong、30 秒死亡检测和指数退避。
3. outbox 只按 peer 调度，不绑定具体 Socket。
4. 连接更换时保持未回执事件。

完成标准：同时连接、强制断线和重连不会重复消息。

### MSG-01 私聊文字

1. 实现 `send_text` 本地事务。
2. 实现 text_message 接收、去重、stored 回执和状态聚合。
3. 对端离线时保留 outbox；上线立即发送。
4. 实现消息分页和会话摘要。

完成标准：双端在线、离线、回执丢失、重发四个场景均通过。

## M4：文件与续传

### FILE-01 清单与源一致性

1. Windows 枚举 file/directory entry，保留空目录并跳过符号链接。
2. Android 接收平台提供的 URI token、entry kind 与元数据。
3. 清理相对路径并限制 10,000 条、16MiB JSON。
4. 在发送和续传前比较 size/mtime。

完成标准：非法路径、目录循环、源变化和超限都有权威错误码。

### FILE-02 Offer 流程

1. 事务创建逻辑消息、每 peer transfer、entries、delivery 和 outbox。
2. 接收端按未知/已知/每次询问决定是否自动 accept。
3. 实现接受、拒绝和接收目录保存。
4. 私聊和群聊共用逐接收方 transfer 数据结构；群文件为每名有效成员创建独立任务。

完成标准：未知设备只弹一次，重发 Offer 不重复任务。

### FILE-03 数据连接

1. 复用 TCP 监听端口，识别 data_hello。
2. 实现 data_entry 头和精确字节计数。
3. 使用 1MiB 缓冲池，保持字节完全在 Rust/FD 内。
4. 文件夹先创建 directory entry，数据连接只按顺序发送 file entry；空目录不建立数据段。
5. 发出 data_end 前再次检查源 size/mtime；完成 flush、大小检查、发布和双通道完成回执。

完成标准：10GB 文件内容逐字节比较一致；数据通道达到 P1 性能门禁。

### FILE-04 暂停、续传和恢复

1. 实现 pause/resume/cancel 控制消息。
2. 每 1 秒或 64MiB 合并持久化 offset。
3. 启动时核对 partial 实际大小与数据库 offset。
4. 网络断开回 queued；用户暂停重启后保持 paused。
5. 源变化进入可处理 failed；重新选择后验证清单再继续/重建任务。
6. 写盘、sync、rename/SAF commit 分别映射磁盘满、权限丢失和无效目录；这些错误不得落成 connection_error。
7. 接收失败允许重选目录，以新 partial 的实际长度生成 transfer_resume；发布完每个文件立即持久化发布结果。

完成标准：在 10%、50%、99% 断线、进程杀死后都从安全偏移恢复；取消删除 partial。

## M5：Flutter 与 Windows 完整私聊

### UI-01 应用壳和列表

实现首次启动和 Windows 两栏应用壳：左栏包含会话/附近 Tab、当前 Tab 搜索、列表、设置齿轮和底部传输入口，右栏显示聊天或传输任务。640-799px 使用列表/聊天单栏路由，800px 以上恢复两栏。先使用 CoreSnapshot，不创建第二套 UI 状态枚举。

完成标准：左栏在 260-360px 范围可调，窗口跨越 800px 断点时不丢当前会话和滚动位置；Core 重启/事件流重连后页面能用 snapshot 恢复。

### UI-02 聊天和气泡

1. 实现文字输入、附件菜单、剪贴板按钮。
2. 实现所有 MessageKind 气泡和权威状态允许的操作。
3. 实现分页、未读、定位 transfer 和真实群成员送达详情模型。
4. Widget 测试覆盖最长设备名、文件名和错误文案。

### WIN-01 Windows 平台适配

按 [平台文档](05-platform-behavior.md#windows-10-x64) 实现单实例、目录、文件路径、托盘、通知、剪贴板和网络事件。平台通知动作只调用 CoreFacade。

完成标准：关闭窗口后传输继续；重启应用续传；Windows ↔ Windows 完整验收通过。

## M6：Android

### AND-01 SAF 文件代理

1. 实现 source/tree URI 持久授权。
2. 实现目录能力探测。
3. 实现 PlatformRequest 打开源、创建目录、创建 partial、commit rename、删除 partial。
4. 写 FD 所有权测试，确保每条路径恰好关闭一次。

完成标准：10GB 文件内容不经过 Dart，进程重启后 URI 仍可访问。

### AND-02 前台服务与通知

1. 实现通知权限流程和 dataSync 前台服务。
2. 实现在线、传输、暂停、打开聊天和发送剪贴板动作。
3. 用户停止服务后不自动偷启；系统停止后保留 queued。
4. Activity 重建重新订阅 snapshot。

### AND-03 Android 真机互传

在至少两款 API 33+ 真机上完成 Windows ↔ Android 双向文字、文件、文件夹、断线和进程恢复。模拟器只用于开发，不计性能验收。

## M7：永久群聊

### GROUP-01 群数据库事务

实现 GroupId 本地序号、群/成员/会话/邀请/outbox 原子创建，并校验 32 台和唯一 owner。

### GROUP-02 邀请与资料同步

1. 实现 invite/reply/update/sync request/response。
2. 只接受当前 owner 的连续 revision。
3. 缺版本时请求全量快照，不阻塞普通消息。
4. 实现成员加入、移除、普通成员退出、在线目标转让和解散；群主直接退出必须拒绝。

### GROUP-03 群消息与文件扇出

1. 一个逻辑 MessageId，为每名目标创建 delivery/outbox。
2. 文件为每名目标创建 TransferId。
3. 在线成员立即发送，离线成员永久排队。
4. 聚合 MessageState 和送达详情。

完成标准：群主离线时现有成员可通信；一名成员离线不阻塞其他成员；重连不重复。

## M8：我的设备与剪贴板

### CLIP-01 双方绑定

实现 bind/reply/unbind，重复 BindingId 不重复提示，解除后原子关闭 ClipboardMode。

### CLIP-02 手动与自动文字

1. 手动读取预览后发送。
2. active binding 才允许 automatic。
3. 按 origin + sequence 去重，使用 fingerprint 抑制平台回环。
4. 每 peer 分别执行 off/send/receive/bidirectional。

### CLIP-03 图片和平台限制

1. 图片复用 file_offer 和数据连接。
2. 读取图片并写入临时源文件时同步计算 SHA-256；该指纹只用于最大 20 MiB 的剪贴板图片去重，不扩展到普通文件。
3. file_offer 必须携带 origin、sequence、fingerprint 和 automatic；接收端在创建第二个 transfer 前先按 origin + sequence 查重。
4. Windows 把来源 JSON 写入 `LAN_CHAT_ORIGIN` 自定义剪贴板格式；Android 在写系统剪贴板前持久化 fingerprint，下一次读取后一次性消费。
5. Windows 后台监听。
6. Android 仅 Activity 有焦点时自动监听；后台通知打开确认页。
7. 锁屏/通知不显示剪贴板正文。

完成标准：双向开启不产生无限循环；未绑定设备不能自动写剪贴板。

## M9：稳定性和发布候选

### QA-01 故障注入

逐项执行断网、回执丢失、畸形帧、磁盘满、权限撤销、源变化、数据库写失败、进程杀死和系统休眠测试。每个失败必须落到权威状态/错误码。

### QA-02 性能回归

执行性能文档完整三组基准，保存原始命令、硬件、日志和三次结果。未达门禁不创建发布候选。

### QA-03 文档复述验收

邀请一名未参与设计的初中级程序员，要求其只看文档说明：

1. 如何实现首个文字消息。
2. 文件断线后 offset 如何确定。
3. Android 为什么不能后台自动读取剪贴板。
4. 群主离线时什么能做、什么不能做。

任何答案仍需设计者补充决策，则先修订文档。

## 示例：通用伪代码模板

### 新增 Core 命令

```text
validate request
check client_operation_id
BEGIN transaction
  read current state
  validate state transition / permission
  write domain rows
  write outbox if remote work exists
  save operation result
COMMIT
publish CoreEvent
return persisted result
```

### 新增接收事件

```text
decode and validate limits
if processed(sender,event_id): resend receipt; return
BEGIN transaction
  validate domain state
  apply changes
  save processed event and receipt
COMMIT
enqueue receipt
publish CoreEvent
```

任何偏离这两个模板的业务路径都需要在 PR 中解释事务与幂等策略。

## 异常情况与常见错误

- 把 TCP `read()` 返回值当完整消息。
- 先发网络成功，再保存本地数据库。
- 收到重复 event 后直接忽略，不重发回执。
- 把网络断开标记成不可恢复 failed。
- 文件经过 Dart、Base64 或逐块 JSON。
- 每个进度值写 SQLite 或触发 Flutter rebuild。
- Android 只保存 raw fd，不保存可重开的 URI token。
- 把 Android 前台服务误认为可以读取后台剪贴板。
- 为群文件只建一个 TransferId，导致无法表达逐成员进度。
- UI 根据自己的布尔值猜状态，不使用核心枚举。
- 完成前覆盖已有文件，或把 partial 当完成文件展示。

## 实现检查表（每任务完成标准）

- [ ] 实现只包含文档范围内的行为。
- [ ] 单元/集成测试覆盖成功、边界和至少一个失败路径。
- [ ] 新错误使用权威错误码。
- [ ] 新网络字段已在协议文档定义并有示例。
- [ ] 新持久化字段已迁移并有回滚/重开测试。
- [ ] UI 文案和按钮与产品文档一致。
- [ ] 性能敏感路径没有新增逐块分配、日志、FFI 或 SQL。
- [ ] 相关测试 ID 和需求追踪表已更新。
- [ ] 初级程序员可以仅根据任务说明复现验收步骤。
