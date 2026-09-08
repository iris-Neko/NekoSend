# 测试计划与需求追踪

## 目的

本文档定义 V1 的单元、数据库、协议、集成、UI、平台、故障、性能和文档验收。测试 ID 是实施任务和发布报告引用的唯一标识。

## 前置知识

- 测试者已阅读 [产品与交互](01-product-ux.md) 和 [数据与 API](04-data-and-api.md)。
- 网络测试者已阅读 [网络协议](03-network-protocol.md)。
- 性能测试者严格使用 [性能规范](06-performance.md) 的基准顺序。
- 真机测试前确认两端在同一 IPv4 子网，关闭访客网络/AP 隔离。

## 测试规范与原则

1. 每个业务成功先验证本地事务，再验证网络回执和对端结果。
2. 所有可重试操作至少测试一次断线和一次重复事件。
3. 所有解析器在分配大型内存前验证长度上限。
4. 文件“完成”必须验证最终路径、大小和内容，不只看 UI 100%。
5. 性能测试使用真实 SSD/闪存和真机，不用模拟器成绩。
6. 自动测试不访问用户真实下载目录、剪贴板或固定生产端口。
7. 发布验收保留日志、设备信息、命令和原始测量结果。

## 权威需求编号

| ID | 需求 |
|---|---|
| `RQ-001` | 同一子网设备启动后 3 秒内自动发现，并正确上下线 |
| `RQ-002` | 私聊文字在线送达、离线永久排队、重发不重复 |
| `RQ-003` | 支持文件、图片、文件夹和剪贴板图片连续 TCP 传输 |
| `RQ-004` | 传输可暂停、取消、断线/重启续传，源变化要求重新选择 |
| `RQ-005` | 未知设备首次确认；已知设备可自动接收或每次询问 |
| `RQ-006` | 永久群聊支持邀请、成员、群主、32 台上限和逐成员投递 |
| `RQ-007` | 群主离线时现有成员仍能聊天和传文件 |
| `RQ-008` | 我的设备必须双方确认，可解除，自动剪贴板默认关闭 |
| `RQ-009` | Windows 后台自动剪贴板；Android 前台自动、后台通知按钮 |
| `RQ-010` | 1GbE、Wi-Fi、SFTP、100GB 内存性能达到规定门槛 |
| `RQ-011` | 数据库事务、幂等、崩溃恢复和文件发布保持一致 |
| `RQ-012` | Windows 托盘/单实例与 Android SAF/前台服务符合平台规范 |
| `RQ-013` | UI 操作、状态、文案和恢复入口符合聊天式产品规范 |

## 测试环境

### 自动化环境

- Windows x64 CI：Rust 单元/集成测试、SQLite、Flutter Windows widget test。
- Android CI：Flutter/Dart 测试、Kotlin 单元测试、API 33 模拟器基础生命周期测试。
- Rust 多节点 harness：同一进程内创建多个核心实例，使用临时端口和临时目录。
- 协议 fuzz：长度帧、UDP JSON、控制消息、data_entry 和路径规范化。

### 真机环境

最低矩阵：

| 代号 | 设备 |
|---|---|
| `W1` | Windows 10 x64 台式机，有线 1GbE、SSD |
| `W2` | Windows 11 x64 笔记本，Wi-Fi、SSD |
| `A1` | Android 13/14 arm64 真机，本地存储 SAF Provider |
| `A2` | Android 15+ arm64 真机，不同厂商 |

必须测试 `W1↔W2`、`W1↔A1`、`W2↔A2`。群聊使用至少三台真实设备；32 台上限用 harness 模拟。

### 固定测试数据

- 文本：空字符串、纯空格、中文、emoji、20,000 字符、20,001 字符。
- 文件：0B、1B、1MiB、10GiB、100GiB。
- 文件名：中文、emoji、200 字符、重名、Windows 保留名、非法字符、多个点。
- 文件夹：空文件夹、1,000 个中型文件、10,000 个小文件、10,001 个文件、符号链接循环。
- 剪贴板：普通文字、1MiB 文字、超限文字、PNG、20MiB 图片、超限图片。
- 协议：合法样例、缺字段、错类型、未知字段、超限长度、无效 UTF-8、重复 event。

测试文件内容使用确定性伪随机生成器，保存预期 SHA-256 仅用于测试验收；产品协议仍不计算默认哈希。

## 自动化命令约定

工程实现后 CI 至少执行：

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
dart format --output=none --set-exit-if-changed app_flutter
flutter analyze app_flutter
flutter test app_flutter
```

多节点和性能 harness 提供稳定命令；命令失败返回非零退出码，不要求人工解析“FAIL”文本。

### 当前自动化与真机证据（2026-08-28）

- `core_rust`：83 项通过，包含粘包、逐字节拆包、非法 UTF-8/JSON、未知字段、UDP 超长包、来源 IP、7 秒离线、持久控制连接多消息、`ping/pong`、0/1 字节文件、短读 EOF、连接槽上限、回执丢失重发去重、10,000 文件加目录项的清单边界、V1 -> V3 无损迁移、三节点群协议、不兼容版本错误、业务错误后连接复用、终态协议错误清理发件箱、UTF-8 错误消息字节限制、重复连接确定性仲裁、同一控制 Socket 上的双向同时发送，以及全部持久化 Flutter 命令的 `ClientOperationId` 重放。网络重启回归保证 TCP 控制监听启动失败时不会继续发布 UDP 发现，避免设备看似在线却无法收消息。文件状态机回归还覆盖：任务被用户暂停后，迟到的 `file_accept` 必须保留 `paused/paused_by_user=1`；SAF 平台 FD 不执行不受支持的 `ftruncate`；接收端终态失败会可靠通知发送端，迟到暂停不会覆盖失败；更换目录的 `transfer_resume` 会同步恢复传输、逐接收者投递和消息聚合状态；文件夹发布过程中重启时，已经发布并清除 `partial_ref` 的条目会校验最终文件，剩余条目继续原子发布；完成的文件夹任务返回根目录引用而不是第一个子文件。加上 `integration_harness` 2 项，Rust workspace 共 85 项通过。
- `integration_harness`：2 项通过；三节点群聊用例覆盖群主离线、成员互发、逐成员状态、发送方重启和离线成员补发。
- Flutter：39 项通过；真实 Rust 桥接覆盖事件订阅/失效，控制器覆盖事件流断开重订阅、按事件类别局部刷新、传输进度直接更新内存、传输筛选/清理/跳转，以及只对本次运行中新完成的接收任务自动打开目录；历史完成任务不会在启动时弹目录。360px Android 布局覆盖长文件名、实时速度和 ETA 不溢出，并覆盖完成图片气泡的受限异步缩略图。Windows 剪贴板占用重试、活动传输退出确认、Toast 冷启动/运行中会话跳转、Android 通知剪贴板预览确认和 Android 原生 FD 向 Rust 交接的成功/失败所有权均有直接单测，FFI 接管失败时 `closeRawFd` 恰好调用一次。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`flutter analyze` 均通过；Flutter 39/39 通过，Android ARM64 Debug/Release、Android x86_64 Debug APK 与 Windows Debug/Release 均完成重建。APK 分别确认包含 `lib/arm64-v8a/liblan_chat_core.so` 和 `lib/x86_64/liblan_chat_core.so`。Release APK 使用显式本地验证签名，不作为生产签名产物。Android 真机前台服务在锁屏 `Dozing` 状态为 `isForeground=true`。
- 锁屏手机经实际 `10.1.1.0/24` 网络向 Windows 发送消息，Dart `AppController.sendText` -> Rust -> TCP -> Windows 入库/UI 事件 -> 回执链路通过。包含单连接 actor 和业务协议错误处理的最新 APK 覆盖安装后，实测得到 `message_id=01a040d2-8892-72c3-a27d-0c4b68c0b77f`、消息 `actorA1787792585312`；调用前后手机均为 `mWakefulness=Dozing`，应用前台服务为 `isForeground=true`，Windows 端该文本记录数由两条增加为三条且新消息状态为 `delivered`。发送时从零建立唯一的 `10.1.1.161:46100 -> 10.1.1.203:53318` 控制连接。该证据不替代剩余群聊真机、SAF 大文件和反向性能验收。
- Android 通知权限拒绝与锁屏组合已实机验证：应用通知 importance 为 `NONE`，手机为 `mDreamingLockscreen=true` 时，前台服务仍为 `isForeground=true`，原 `DeviceId=d_5623cf307daae4c9aa521861f7c644bf` 持续被 Windows 从 `10.1.1.161` 发现。另一次同设备真机调用完整发送接口后，Windows 恰好收到一条 `android-device-e2e-1787800891124`，状态为 `delivered`、outbox 为 0、SQLite `quick_check=ok`。这证明通知展示权限不再错误关闭后台网络。
- Windows -> Android 离线补发实机文字链路通过：先执行 `am force-stop`，确认 Android 进程和前台服务均不存在；Windows UI 发送 `message_id=01a040a1-7df5-7c20-aef0-cf8e7352b820` 后，气泡显示“等待发送”，消息状态为 `queued`，逐接收者状态为 `queued`，outbox 事件在重试后仍为 `pending`。随后在手机保持 `mWakefulness=Dozing` 时通过 ADB 启动 `MainActivity`，应用恢复为同一 `DeviceId=d_5623cf307daae4c9aa521861f7c644bf`，前台服务为 `isForeground=true`；Windows 消息转为 `delivered`，逐接收者状态转为 `stored`，对应 outbox 行删除。Android SQLite 中按 `message_id` 查询恰好一条，证明本次恢复没有重复入库。
- Windows `WIN-001`/`WIN-002` 实机通过：点击主窗口关闭按钮后，原进程 PID `53616` 继续运行且 TCP `53318` 仍保持一个监听；再次启动同一 Debug 可执行文件时，第二实例退出码为 `0`，系统中仍只有 PID `53616`，并重新出现原 `LAN Chat` 窗口。`WIN-003` 同样实机通过：20GiB 任务 `transfer_id=01a04202-40b8-7811-a206-7f19d07190b7` 传输中从托盘退出，Windows 进程与 Flutter 会话均真正结束；Android 在 `57671680` bytes 处转为 `queued/connection_error`。Windows 重启后同一任务从该安全偏移恢复，最终双方均为 `completed`、偏移 `21474836480`，Android final 文件实际大小为 `21474836480`。此前 4GiB 活动任务还验证了托盘“暂停全部传输”和隐藏窗口下选择“退出”会先恢复主窗口再显示确认框。
- 重复控制连接和单 Socket 双向复用已通过自动化与实机：注册表单测证明双方统一按 `(initiator_device_id, connection_id)` 保留最小键，并向旧连接发送 `duplicate_connection`；并发单测让两个 actor 在同一 TCP 上同时发送文字，双方均收到 stored 回执且各只入库一次。最新 Windows/Android Debug 实机中，Android 先通过 `10.1.1.161:37112 -> 10.1.1.203:53318` 发送 `message_id=01a040bd-f125-7e92-b45b-c6f3069b9ec0`，随后 Windows 在同一连接反向发送 `message_id=01a040c0-a7a0-7cb0-b844-6a9e7553c98b`；全过程 Windows 仅有这一条 Established 控制连接，两端消息均为 `delivered` 且 Android outbox 为空。
- Android instrumentation 在 Android 14 真机 `M2012K11G` 上共 17 项：16 项通过、0 失败、0 错误，未配置凭据的 SFTP benchmark 1 项跳过。覆盖 SAF 接收目录探测、空目录与 partial 恢复、重命名后 URI 重定向、权限撤销错误映射、代理 FD 的系统级 `ENOSPC`、detached FD 关闭后的 `EBADF`、SAF 发送源重建与 FD 内容、文件夹清单保留根目录引用和空子目录、完成文件 `ACTION_VIEW`、文件夹/显示位置 `ACTION_OPEN_DOCUMENT_TREE + EXTRA_INITIAL_URI`、前台通知三项动作、真实“停止后台在线” `PendingIntent`，以及 Activity 后台/前台/`recreate()` 后复用同一 FlutterEngine。生命周期测试由 ADB shell 启动 Debug-only Activity；剪贴板监听状态始终与真实窗口焦点一致。测试 Provider 和生命周期 Activity 仅存在于 Debug，Release APK 不包含。
- Android Studio 的 `LANChat_API34` Pixel 6 x86_64 AVD 已创建并完成中间开发验证。`android-x64` Debug APK 实际包含 `lib/x86_64/liblan_chat_core.so`，应用在 API 34 上启动后前台服务为 `isForeground=true`，Rust 正常监听 UDP `53317` 与 TCP `53318`；同一套 instrumentation 在 AVD 上通过，SFTP benchmark 仍按未配置凭据的规则跳过。模拟器默认地址为 `10.0.2.16`，NAT 不转发实体 `10.1.1.0/24` 的 UDP 广播，因此该结果只覆盖 Android 页面、平台适配和协议开发，不计作 `E2E-003` 的三台实体设备签署。
- SFTP instrumentation 已扩展为同时支持单文件和递归目录，目录模式逐文件上传并立即核对远端大小，输出总文件数、字节、耗时和吞吐。配置临时基准服务后单独执行的 1,000 文件用例三轮均通过；不配置 `host` 时仍按设计跳过，因此日常全套 instrumentation 不依赖外部服务。
- 为规避 MIUI 对 `adb shell input` 返回的 `INJECT_EVENTS` 拒绝，Debug 构建通过仅在断言开启时注册的 Dart VM service extension 调用应用自己的命令；通用宿主工具为 `app_flutter/tool/debug_command.dart`，Release 不注册这些入口。2026-08-27 从锁屏设备以 ADB 冷启动应用和前台服务后发送 `android-to-windows-lockscreen-20260827-1420`；Windows 收到同一 `message_id=01a041da-7276-7380-b331-307569fd35cc`，两端均为 `delivered`。Windows TCP 监听重启后又在手机 `mWakefulness=Dozing` 时发送 `lockscreen-after-listener-restart-20260827-1656`，Windows 收到 `message_id=01a04271-b40c-7cd0-b0ec-f0b8d3d3fa61`、状态 `delivered`、相关 outbox 为 0，TCP 为 `10.1.1.161:40734 -> 10.1.1.203:53318`，数据库 `quick_check=ok`。修复“TCP 启动失败却保留 UDP 假在线”的网络重启顺序后，新 Windows Debug 又从锁屏手机收到 `message_id=01a0427e-878f-7c13-b13b-470231345b1a`，状态 `delivered`、相关 outbox 为 0，连接为 `10.1.1.161:43918 -> 10.1.1.203:53318`。最终 Debug/Release 重建后，手机保持 `SCREEN_STATE_OFF/INTERACTIVE_STATE_SLEEP` 且前台服务 `isForeground=true`，再次发送 `lockscreen-final-20260827-1804`；Windows 以同一 `message_id=01a042ad-2424-7a61-a01f-808d28e833a9` 入库，两端均为 `delivered`。这说明安全锁屏不妨碍 ADB 驱动的真实发送链路；只有依赖可见窗口的系统选择器和 Activity 交互需要解锁。
- Windows Release 已接入系统 Toast：启动时创建指向当前可执行文件且带 `AppUserModelID=dev.lanchat.LANChat` 的开始菜单快捷方式；锁屏 Android 发来 `toast-e2e-1787807713188` 后，Windows 通知数据库出现 `toast` 记录，payload 含发送设备、正文、稳定私聊 ID 和“打开”按钮。冷启动传入私聊 `openConversation:` 后未读数由 2 变为 0；首实例运行时，Android 发到群 `message_id=01a04289-72e3-7662-b930-f0dd60c82e9a` 使群未读数变为 1，随后第二实例携带群路由启动并以退出码 0 结束，系统仍只有首实例且群未读数变为 0。`WIN-009` 整体通过。安装/卸载脚本通过 PowerShell AST 检查，安装脚本只声明两条 `Profile Private` 规则：UDP 53317 和 TCP 53318；实际提升安装与规则枚举仍待 `WIN-004` 验收。
- 文档与安装器验收已脚本化并接入双平台 CI：`scripts/verify_docs.ps1` 通过 `DOC-001` 至 `DOC-007`，实际解析 24 个 JSON 示例、执行当前 Schema V3，并确认其结构与 V1 → V3 迁移结果一致；`scripts/verify_windows_installer.ps1` 通过 PowerShell AST 确认安装器只声明 UDP 53317、TCP 53318 两条 `Private` 入站规则。非提升终端的只读 `-Live` 检查准确报告当前系统两条规则均未安装；该结果证明诊断路径有效，不算 `WIN-004` 安装后通过证据。`DOC-008` 仍必须由未参与设计的初中级开发者执行。
- Android Release 签名不再静默回退到 debug key：四个 `LAN_CHAT_ANDROID_*` 环境变量或本机 `key.properties` 必须完整配置，缺失时 `verifyReleaseSigningConfiguration` 明确失败；只有显式 `LAN_CHAT_ALLOW_DEBUG_RELEASE_SIGNING=true` 才能生成不可发布的本地验证包。该验证包为 `23,595,508` bytes，包含 ARM64 Rust 库，扫描未发现 Debug service extension、测试 Provider/Activity 或 SFTP 测试类。Windows Release 同步重建通过。
- 全部回归后，ADB 在手机 `mScreenOn=false` 且前台服务 `isForeground=true` 时通过正式发送接口提交 `post-regression-lockscreen-20260827-1935`。Windows 以同一 `message_id=01a04301-82b0-76b1-9d89-fa5c0a877399` 入库，双方状态均为 `delivered`，TCP 控制连接保持建立。该结果再次证明锁屏不关闭 Android 后台网络；受系统限制的是后台剪贴板读取，而不是消息与文件收发。
- 加入文件夹发布恢复修复和 x86_64 模拟器构建支持后，Android ARM64 Debug 覆盖安装并再次执行真机 instrumentation；随后手机保持 `mWakefulness=Dozing`、前台服务 `isForeground=true`，通过应用正式发送接口提交 `final-api34-lockscreen-20260827-2023`。Windows 以同一 `message_id=01a0432f-d84f-7772-89c6-6a27dbbc4f22` 恰好入库一次，Android 发送端和 Windows 接收端均为 `delivered`。
- Android -> Windows 10GiB 单文件正式 Rust/TCP 数据路径三轮通过：三轮为 `35.220/33.905/34.040 MB/s`，中位 `34.040 MB/s`；对应耗时 `304.896/316.688/315.432s`。三轮均为 `completed`、偏移 `10737418240`、相关 outbox 为 0，Windows 工作集峰值增量最大约 `19.22MiB`。同方向 `iperf3` 为 `36.06 MB/s`，LAN Chat 中位达到约 `94.4%`。使用同一 Android 源文件执行的 SFTP 三轮为 `21.076/20.062/18.295 MB/s`，中位 `20.062 MB/s`，LAN Chat 快约 `69.7%`。Android 源、最终 LAN Chat 接收文件和最终 SFTP 接收文件 SHA-256 均为 `732377e7f4a2abdc13ddfa1eb4c9c497fd2a2b294674d056cf51581b47dd586d`。该证据通过反向 `PERF-005`、`PERF-006`，但不替代 Android SAF 发送路径验收。
- Windows -> Android 离线 10GiB 永久队列通过：Android `am force-stop` 后创建 `transfer_id=01a041ce-cc0d-7863-ad24-0b18dbfed572`，任务立即为 `queued`；Windows 重启后任务和 outbox 仍存在。Android 冷启动后自动完成，两端大小均为 `10737418240`，SHA-256 均为 `732377e7f4a2abdc13ddfa1eb4c9c497fd2a2b294674d056cf51581b47dd586d`，outbox 清空且 SQLite `quick_check=ok`。本条完成 `E2E-002` 四个步骤。
- Windows -> Android 10GiB 断点恢复通过：`transfer_id=01a041d5-a615-76c2-a05b-9b0bdf2718fd` 在约 1.68GiB 时断 Wi-Fi，发送端转为 `queued/connection_error` 并保存安全偏移 `3729112284`；恢复网络后从该偏移继续。接收偏移超过 50% 后强杀 Android，partial 实际长度为 `7740172708`；ADB 在锁屏状态冷启动 Activity 后任务自动恢复并完成，没有从 0 开始。最终两端大小和 SHA-256 与上条相同，Android final 文件存在且 partial 消失。另一次 4GiB 任务已在约 99% 处用户暂停、重启并继续完成。
- Windows -> Android 系统真实 SAF 大文件链路通过：Android 默认目录为 `content://com.android.externalstorage.documents/tree/primary%3ADCIM%2FCamera`。10GiB 非稀疏任务 `01a042b3-1944-7dc3-b456-316de226c14c` 约 `49.8s` 完成，源/目标 SHA-256 均为 `732377e7f4a2abdc13ddfa1eb4c9c497fd2a2b294674d056cf51581b47dd586d`。随后 100GiB 非稀疏任务 `01a042ba-6b1b-7c90-9048-12d02bf71dd8` 在手机锁屏时完成，两端大小和 `persistedBytes` 均为 `107374182400`，耗时 `576.6s`、有效吞吐 `186.22 MB/s`；Windows 工作集峰值增量 `34.57MiB`，Android PSS 峰值增量 `1.34MiB`。源/目标 SHA-256 均为 `f0b14a8da7f1c48a0846647a078b97956edd8df451a62fc4b466879aa24d4fd7`，最终文件存在且 partial 消失。该证据完成 `PERF-007` 最终内容签署。
- Android SAF 磁盘不足与恢复真机通过：Debug Provider 使用 `StorageManager.openProxyFileDescriptor` 在约 1MiB 后返回真实 `ENOSPC`。正式任务 `transfer_id=01a0429c-d73c-7e63-8ef2-8fe0ef4cd7d1` 在 Windows 发送端和 Android 接收端均进入 `failed/not_enough_space`，接收端持久安全偏移为 `929792` bytes；关闭故障并更换 SAF 接收目录后自动恢复到 `completed`，最终大小 `4194304` bytes，源/目标 SHA-256 均为 `bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8`，final 存在、当前任务 partial 消失、相关 outbox 为 0。该证据完成 `IT-RESUME-010` 和 `FI-002`。
- 文件夹正式数据路径通过：`transfer_id=01a041d3-39fa-71c1-8788-a0c616d7c7e6` 共 6 个 entry、22 bytes，覆盖根目录、嵌套目录、两个空目录和两个文件；Android 最终目录树与文件内容一致，outbox 清空。该轮目标为应用私有测试目录，SAF 目标见下一条。
- Android SAF 文件夹端到端通过：Windows 通过正式 Rust/TCP 路径发送 `transfer_id=01a041e7-9dee-7400-a9a2-b58dfb7b15d4`，Android `receiveBaseRef=content://dev.lanchat.lan_chat.test.documents/tree/root`。接收经 `ContentResolver`、raw FD、partial 和 rename 完成，共 34 bytes；根文件 SHA-256 为 `2555f1bdecf31cb75546784e1d4fdc4db62a57add78b96c41285bb860c67b993`，嵌套文件为 `98933cfaff83df8dd3791fede0193e6f411b4f3547ec4c83c22539dd9b7896e3`，空目录保留。该证据完成 SAF 文件与文件夹功能链路；SAF 10GiB 性能仍单独计入性能矩阵。
- 正式小文件/多文件链路通过：Windows -> Android 的 `transfer_id=01a0420c-828e-7532-b5e7-3d9a168f17e8` 包含 1,000 个 1MiB 文件，共 `1,048,576,000` bytes、`entry_count=1001`，约 `7.46s` 完成。边界任务 `transfer_id=01a04214-76b3-74f0-a599-cbb91efe89c1` 包含 10,000 个 4KiB 文件，共 `40,960,000` bytes、`entry_count=10001`；Windows 与 Android 最终均为 `completed`，Android 实际文件数恰好 10,000，首尾文件均为 4,096 bytes，`.partial` 为 0，双方相关 outbox 清空。任务在 Android 锁屏 `Dozing` 且前台服务在线时完成；验收后临时源文件、接收文件和临时默认接收目录均已清理。
- `PERF-008` 同源文件 SFTP 对照通过：Android -> Windows 的 LAN Chat 三轮任务 `01a042ce-0906-7470-8871-c3787a4d2bd8`、`01a042ce-c0e5-7232-a615-f83481632bbf`、`01a042cf-451a-7611-ab4c-655984d85fed` 分别耗时 `36.936/33.871/36.735s`，中位 `36.735s`；同一 Android 源目录经 SFTP 三轮耗时 `145.095/150.142/148.154s`，中位 `148.154s`。两种协议的六个目标目录均为 1,000 个文件、`1,048,576,000` bytes。LAN Chat 中位约 `28.54MB/s`，SFTP 中位 `7.078MB/s`，LAN Chat 不慢于 SFTP。
- 两设备永久群实机通过：Windows 创建 `group_id=g:d_1e582934a1bcc579652eeb924b314106:1` 并邀请 Android；Android 接受后 revision 为 2，双方文字均 `delivered`。群主 Windows 离线时 Android 消息先 `queued`，Windows 重启后自动送达且无重复；Android 重启后群仍存在。第三台实体设备和管理操作矩阵仍待执行，因此 `E2E-003` 尚未整体通过。
- 两设备群管理矩阵通过：同一群由 Windows 在 revision 2 将群主转给 Android；Android 改名为“家庭设备管理验收”后再转回 Windows。随后 Windows 移除 Android，Android 本地显示 `localMembership=removed` 且历史保留；重新邀请并接受后双方 revision 为 8、角色和成员一致。Android 重加入后发送 `message_id=01a041f3-ef10-7120-a6de-12903bde769a`，Windows 为 `delivered`。第三台实体设备矩阵仍待执行。
- “我的设备”双方确认、双向剪贴板和解除绑定实机通过：`binding_id=01a03f8a-b0c7-7d73-b59d-13080a3e07d8` 经 Android 确认后双方为 `active/off`，改为 `bidirectional` 后两个方向各发送一次，Windows 系统剪贴板实际读到 Android 文本；两端每条消息和去重记录均为 1、outbox 为 0，无回环。解除后双方为 `removed/off`，自动发送返回“需要有效绑定”。结合下一条的 Android 后台通知动作测试，`E2E-004` 六个步骤均有覆盖。
- Android 后台通知剪贴板链路已自动化验收：原生通知包含“发送剪贴板”动作并路由到带 `send_clipboard` 参数的 MainActivity；Flutter 收到前台动作后读取剪贴板、显示预览，只有点击“发送”才调用 `submitLocalClipboardText(..., automatic=false)`。Widget 39/39 和通知定向 instrumentation 2/2 通过；锁屏时系统仍要求先解锁，不把剪贴板内容显示在锁屏上。
- 2026-08-28 最终锁屏回归：API 34 真机保持 `mWakefulness=Dozing`，前台服务为 `isForeground=true`；Android 通过本次构建发送 `message_id=01a04465-60df-74b3-88fa-1dfd71c45de9`，Windows 从 `10.1.1.161:42164 -> 10.1.1.203:53318` 接收。两端消息均为 `delivered`，相关 outbox 为 0，Windows SQLite `quick_check=ok`。

### 补充回归与设备状态（2026-09-08）

下述真机联调阶段没有修改应用源码，实机复用 2026-08-28 的 Windows Debug 和 Android ARM64 Debug 构建；应用文件 SHA-256、完整命令输出和逐步接口响应保存在本机忽略目录 `.e2e-data/20260908/`。以下只记录实际执行的范围，不将历史结果或尚未执行的设备组合计作本轮通过；后续 Windows 回车修复单独记录。

- Rust workspace 85 项、Flutter 39 项重新执行全部通过；`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 和 `flutter analyze` 均通过。
- Android 14 真机 `M2012K11G` 的现有 instrumentation APK 本轮报告 16 项：15 项通过、SFTP 基准因未配置参数跳过 1 项，没有失败；这里按实际运行器数量记录，不沿用上方历史计数。
- Windows 与 Android 14 双向文字送达，双方按同一 MessageId 各只有一条记录。Android 进程停止后，文字 `01a0802c-7605-7f32-bb07-4708b27b0551` 和 32MiB 文件任务 `01a0802c-7695-7233-9901-69ce35d7b76f` 均排队；Windows 进程重启后记录与 DeviceId 保持，Android 再启动后自动送达且无重复。
- 32MiB 文件完成 Windows -> Android -> Windows 往返，三份 SHA-256 均为 `f3429d993edf24504473174d0ba9c8f61b0c274ee8fd0dc689ac6ae321ab2d30`。重复发送产生递增名称，原文件未覆盖；嵌套文件、根目录及两处空目录的结构和内容均核对通过。Android 接收使用 Debug SAF Provider；反向发送使用应用私有文件路径，不将其记为系统 DocumentsUI 或真实 SAF 发送授权的新增验收。
- 1GiB 任务 `01a0802e-4658-7003-bc2c-2db714e07552` 在 `197132288` bytes 处暂停；接收端进程重启后仍为 paused 且偏移保持。恢复后两端 completed，源与目标 SHA-256 均为 `fdff14a94c824824f9fdb73779ec2071ab12eb50312f5a33554bd89e757c1cbc`。该测试是恢复正确性回归，不是三轮性能计分；校验后已清理两端这份 1GiB 临时文件。
- 锁屏发送、接收分别使用消息 `01a0802f-b33e-7c62-926c-bfe77118d72e`、`01a0802f-b325-7ec1-8d68-79b39883486a`；两端 delivered 且各入库一次，调用前后 Android 均为 Dozing，前台服务为 `isForeground=true`。
- 第二台真机 `23127PN0CC` 为 Android 16 / API 36，与第一台同属小米。初次 APK 安装返回 `INSTALL_FAILED_USER_RESTRICTED`；随后确认实际阻塞在安全中心的单次安装确认，用户确认后主应用安装成功，不能继续归因为 USB 开关未开启。三台实体设备已通过同网段互相发现，Android 16 与另两端的双向私聊也已送达且各只入库一次。独立 instrumentation APK 的安装确认仍被取消，本轮没有运行 Android 16 instrumentation。
- 三实体节点功能场景使用 Windows、Android 14、Android 16 和独立测试群 `g:d_1e582934a1bcc579652eeb924b314106:2`。Android 16 离线时创建群，Android 14 先接受邀请；Android 16 重新上线补收并接受后，三端均为 revision 3、三名 joined 成员。此处是 `E2E-003` 的一 Windows 加两 Android 功能组合，不替代其原定双 Windows 组合或不同厂商矩阵。
- Windows 群主进程退出后，两台手机直接交换群文字 `01a0803a-77f5-7031-a2d3-7ec5dcc6465d`、`01a0803a-784b-7721-9bd2-9343ab48de2a`，发送端为 partially_delivered、另一手机在线副本为 delivered。两台手机还双向发送 32MiB 群文件，MessageId 为 `01a0803b-05dc-7352-90fe-f5005a343e13`、`01a0803b-13ca-7083-849a-73d284c52d5b`；每个发送端各有两个独立任务，手机目标 completed，离线 Windows 目标 queued。两台手机随后重启，离线目标队列仍保留。Windows 再启动后补收全部文字和文件，三端上述四个 MessageId 各只有一条且均 delivered；两部手机和 Windows 接收文件 SHA-256 均与上述 32MiB 源文件一致。
- 群主转让给 Android 16 后三端 revision 4；原群主的改名命令被拒绝。新群主改名后 revision 5，再移除 Android 14 后 revision 6；被移除端禁止继续发言，原历史保留。三端进程重启后，最终仍为 revision 6、同一群名和 Android 16 群主，被移除成员仍为 removed；四条已验收群消息各保留一条。Android 16 的最后复核在下述冻结恢复后完成，不将其计作无干预的后台通过证据。
- **未通过：Android 16 锁屏后台可用性。** 进程重启后手机处于 Dozing，应用前台服务仍为 `isForeground=true`，系统 DeviceIdle 为 ACTIVE 且正在充电，但 PID `15373` 的 `cgroup.freeze=1`、`cgroup.events` 为 `frozen 1`。原生 Dart VM 的 `getVM` 无响应，其他设备也不再发现该手机。再次执行 `am start` 激活 MainActivity 后，同一进程的 `cgroup.freeze` 变为 0，未重装、未重启进程即可读取完整群资料并恢复接口响应；当时屏幕仍报告 Dozing。证据保存在 `android16-freeze-evidence.log` 和 `live-evidence.jsonl`。这证明本次阻塞涉及设备进程冻结，不足以确定具体冻结发起方；未修改该手机的省电或冻结策略，问题仍需定位和回归，不能签署完整后台或发布验收。
- 两台同厂商手机不能替代不同厂商 Android 的验收要求。Linux 辅助机 SSH 与到手机的 TCP 连通性已确认，没有把 Linux 计作受支持客户端。测试后已恢复三端原先的默认接收目录；未清除原有聊天数据库、会话或历史任务，仅新增独立测试群和测试消息，并清理本轮明确创建的 1GiB 大文件。

### Windows 回车发送修复（2026-09-08）

修复桌面多行输入框只监听提交动作、未处理实体回车的问题：Windows 两栏和窄窗口单栏都支持普通回车及小键盘回车发送，Shift+Enter 交由原生输入处理换行。组合输入中的回车保留给输入法，长按不重复发送，发送失败保留草稿，成功后保持输入焦点。Android 保持原有硬件按键和软键盘提交行为。新增 6 项真实按键及平台输入模拟测试，全套 Flutter 45/45 通过，`flutter analyze` 无问题，Windows Debug/Release 均构建成功；确认无未发送草稿和活动传输后，正常重启了 Windows Debug 客户端以验证新的启动文件。原生输入法候选窗口的人工验证不等同于 Widget 中的组合输入模拟。

### 自定义设备名称与可读默认名（2026-09-08）

- 设置页支持点击设备名称整行修改，Windows 弹窗保存后立即刷新；取消不写入，空名称、超长 Unicode 名称和控制字符被校验，后端失败保留草稿。修正已有私聊标题读取，使其采用最新持久化对端名称而非旧标题缓存；改名的校验与落库先于网络服务重启，非法名称不会先停止发现。
- Android 首选系统可读机型，并对旧默认型号代码做一次性兼容升级。两台真机已分别由 `M2012K11G`、`23127PN0CC` 升级为系统返回的 `Mi 11i`、`Xiaomi 14`；Windows 和另一台手机的附近列表及已有私聊标题均同步更新，原 DeviceId 保持不变。
- 实机验证：完成兼容升级后，主动把 Android 14 名称设回 `M2012K11G`，重启后仍保留该自定义值；Android 16 自定义中文名同步到 Windows，重启后同样保留。测试后分别恢复为 `Mi 11i`、`Xiaomi 14`，没有修改系统设备名称或账号信息。33 字符的非法改名被拒绝，原名称与运行中的发现服务保持可用。
- Rust workspace 86/86、Flutter 51/51、Android JVM 7/7 通过；Clippy 与 Flutter analyze 通过，Android ARM64 Debug 和 Windows Debug/Release 构建成功。两部手机已覆盖安装新包，Windows 已在确认无未完成输入和活动传输后正常重启更新。此处 JVM 测试不替代先前尚未执行的 Android 16 instrumentation。

### Android 返回导航与 Linux Flatpak（0.2.0，2026-09-08）

- Android 系统返回使用 PopScope：聊天/传输详情回原列表，附近/设置回会话首页，首页返回只 moveTaskToBack，不关闭 Rust 核心；键盘与弹窗优先消费返回。返回导航、键盘草稿保留和弹窗优先级已通过 Widget 测试；两台手机主应用已覆盖安装，K40 instrumentation 15 项通过、1 项 SFTP 基准跳过。Android 16 测试组件安装确认仍被取消，未把该套 instrumentation 算作通过。
- 新增 Linux 平台标识和 Schema V4。Windows 本地 Rust workspace 90/90、Flutter 58/58 通过；迁移测试验证 V3 身份、绑定、会话和永久 outbox 保留，非法旧外键导致整个 V4 迁移回滚。Windows Debug/Release、Android ARM64 Debug 均成功构建，Android JVM 7/7 通过。
- Linux 使用 Fedora 44 KDE/Wayland 实体桌面，在 GNOME 50 SDK 内从源码构建 0.2.0 Release Flatpak，并实际以用户安装运行。桌面窗口、中文 UI、单实例和 StatusNotifier 托盘均已检查；不是 perf_harness 或只读 TCP 探测。运行权限与 manifest 一致，不包含任意主目录或宿主命令权限。
- 实际安装包被 Windows 和 Android 发现为 linux，设备 ID 为 `d_63db25908316dc2ab4fbd71846a1d466`。Windows -> Linux 消息 `01a080df-9898-7fe2-9b09-213f92f27a71` 已送达；Linux Release 窗口内真实键盘回车发送的消息 `01a080e0-ec1d-75a0-b85b-76c31a047950` 在 Windows 恰好一条且 delivered。
- 32MiB 文件通过 Windows -> Linux -> Windows 完整往返，Linux 的首次接收目录和发送源均通过实际 KDE 文件门户选择。正向任务 `01a080e1-6565-7af2-b948-49510c535987` 和反向任务 `01a080e9-0cfb-7901-9ab9-53f2aa2c9913` 均 completed；三份 SHA-256 为 `f3429d993edf24504473174d0ba9c8f61b0c274ee8fd0dc689ac6ae321ab2d30`。Windows 临时接收目录已恢复，Linux 测试文件限定在 Downloads/NekoSend/Flatpak-Test。
- Wayland 显示在真实 KDE 桌面检查，键盘自动化另在隔离 X11 显示器上操作同一 Release 包；文件门户仍走实际 KDE。结束后切回真实桌面，未使用调试扩展作为 Linux 发布包验收证据。测试截图和日志位于本机忽略目录，未作为用户数据提交 Git。
- 本轮不等同于完整 V1 发布签署：Android 16 锁屏冻结、不同厂商 Android 与双 Windows 矩阵、生产 Android 签名、提升权限安装/防火墙验收和 DOC-008 仍按原记录保留；Linux 的多发行版、开机启动授权及不同 Wayland 剪贴板策略仍有兼容性风险。

## ID、枚举和状态单元测试

| ID | 场景 | 期望 |
|---|---|---|
| `UT-ID-001` | 生成 10 万 DeviceId | 格式合法且无重复 |
| `UT-ID-002` | 两端以相反参数构造私聊 ID | 得到相同 ID |
| `UT-ID-003` | 本地 group sequence 递增 | GroupId 稳定且严格递增 |
| `UT-ID-004` | 大写、短、错误前缀 DeviceId | 全部拒绝 |
| `UT-ID-005` | UUIDv7 ID 序列化/反序列化 | 无信息变化 |
| `UT-ENUM-001` | 所有权威枚举 serde/FFI 往返 | 值完全一致 |
| `UT-ENUM-002` | 未知枚举值 | 返回明确解析错误 |
| `UT-STATE-001` | 状态表中所有允许转换 | 全部成功 |
| `UT-STATE-002` | 状态表外所有转换 | 全部拒绝且状态不变 |
| `UT-STATE-003` | 网络断线 | transferring 回 queued，不进入最终 failed |
| `UT-STATE-004` | 用户取消 | 进入 cancelled 且不能恢复原任务 |
| `UT-STATE-005` | source_changed 处理后重新选择 | failed 可回 queued/accepted |

## 数据库测试

| ID | 场景 | 期望 |
|---|---|---|
| `DB-001` | 空数据库执行当前 Schema V3 | 所有表、索引、PRAGMA 正确 |
| `DB-002` | 已是 V1 再次打开 | 不重复迁移、不丢数据 |
| `DB-003` | 外键/CHECK/UNIQUE 违规 | SQLite 拒绝并回滚 |
| `DB-004` | 发送文字事务中每个写点注入失败 | 不留半条 message/delivery/outbox |
| `DB-005` | 接收事件落库后回执 | 回执记录和业务记录同一事务存在 |
| `DB-006` | 重复 ClientOperationId 相同参数 | 返回首次结果，不重复记录 |
| `DB-007` | 重复 ClientOperationId 不同参数 | 返回首次结果并记录警告 |
| `DB-008` | 重复 `(sender,event_id)` | 不重复业务，只取已有 receipt |
| `DB-009` | offset 合并频率 | 1 秒或 64MiB前不产生额外事务 |
| `DB-010` | completed 提交前文件发布失败 | transfer 不得为 completed |
| `DB-011` | 删除会话 | 取消未完成任务，接收目录文件仍存在 |
| `DB-012` | 群快照无 owner/双 owner/33 人 | 事务拒绝并保持旧 revision |
| `DB-013` | 对端改名与数据库重开 | 已有私聊采用最新持久化设备名称，设备 ID 和会话记录不变 |

DDL 测试必须在真正 SQLite 上运行，不用内存 Map 模拟约束。

## 协议编解码测试

| ID | 场景 | 期望 |
|---|---|---|
| `PROTO-001` | README/协议所有 JSON 示例反序列化 | 全部成功且字段一致 |
| `PROTO-002` | TCP 每 1 字节拆包 | 正确重组一个帧 |
| `PROTO-003` | 多帧一次 read 粘包 | 顺序解析所有帧 |
| `PROTO-004` | 长度 0、1MiB+1 普通帧、16MiB+1 清单 | 分配前拒绝 |
| `PROTO-005` | 无效 UTF-8/JSON | 返回协议错误，不崩溃 |
| `PROTO-006` | 未知 JSON 字段 | 合法消息成功并忽略未知字段 |
| `PROTO-007` | 缺必填字段/字段错类型 | 拒绝该事件 |
| `PROTO-008` | UDP 1,201 字节 | 丢弃且不更新在线状态 |
| `PROTO-009` | announce JSON 伪造 IP | 使用 UDP 来源 IP |
| `PROTO-010` | data_entry 后精确 N 字节再下一个头 | 不把文件字节当 JSON |
| `PROTO-011` | 数据 EOF 少 1 字节 | 记录可恢复中断，不完成 |
| `PROTO-012` | 一个非法控制帧 | 关闭当前连接；对端进程继续运行，合法 outbox 可重连 |

解析器 fuzz 至少持续 10 分钟无 panic、OOM、死循环和越界分配。CI 可运行固定语料短测，夜间运行长 fuzz。

## 发现与连接集成测试

| ID | 场景 | 期望 |
|---|---|---|
| `IT-NET-001` | 两节点启动 | 3 秒内互相 online |
| `IT-NET-002` | 忽略本机 announce | 附近列表无自己 |
| `IT-NET-003` | announce 停止 | 7 秒后离线 |
| `IT-NET-004` | 同 DeviceId 名称变化 | 更新同一 peer，不新建 |
| `IT-NET-005` | 双方同时建立控制连接 | 最终只保留排序最小一条 |
| `IT-NET-006` | 控制连接 30 秒无帧 | 关闭并进入重连 |
| `IT-NET-007` | 网络变化 | 退避归零、重绑并立即 announce |
| `IT-NET-008` | 协议无共同版本 | 返回 UNSUPPORTED_PROTOCOL 并关闭 |
| `IT-NET-009` | 端口已占用 | 启动返回权威绑定错误 |

## 私聊与投递测试

| ID | 场景 | 期望 |
|---|---|---|
| `IT-MSG-001` | 在线发送中文/emoji | 对端存储并回执，双方状态正确 |
| `IT-MSG-002` | 对端离线发送 | 本地立即 queued，上线自动送达 |
| `IT-MSG-003` | stored 回执丢失 | 重发同 event，对端只有一条消息 |
| `IT-MSG-004` | 发送方重启且 outbox 未回执 | 恢复后继续发送 |
| `IT-MSG-005` | 20,001 字符 | 本地拒绝，不创建消息 |
| `IT-MSG-006` | 同名不同 DeviceId | 创建两个独立私聊 |
| `IT-MSG-007` | 删除本机会话 | 对端不受影响，重新聊天创建新本地记录 |

## 文件与续传测试

### 正常路径

| ID | 场景 | 期望 |
|---|---|---|
| `IT-FILE-001` | 0B、1B、1MiB 文件 | 最终大小/测试哈希一致 |
| `IT-FILE-002` | 10GiB 单文件 | 一条数据连接、状态完成、内容一致 |
| `IT-FILE-003` | 图片 | 完成后异步缩略图，不阻塞完成回执 |
| `IT-FILE-004` | 含空目录的 1,000 文件目录 | 保持所有目录结构、空目录和复用数据连接 |
| `IT-FILE-005` | 10,000 小文件 | 成功且不超过清单/内存限制 |
| `IT-FILE-006` | 10,001 文件 | 发送前 PROTOCOL_LIMIT_EXCEEDED |
| `IT-FILE-007` | 目标存在同名 | 保存为 `(1)`，原文件不变 |
| `IT-FILE-008` | Windows 非法/保留名称 | 按平台规则清理并记录 final name |

### 接收策略

| ID | 场景 | 期望 |
|---|---|---|
| `IT-OFFER-001` | 未知设备首次 Offer | 只弹一次确认 |
| `IT-OFFER-002` | 接受未知设备 | peer 变 known，任务开始 |
| `IT-OFFER-003` | known + auto_accept | 不弹窗直接接受 |
| `IT-OFFER-004` | known + ask_every_time | 每个批次一次确认 |
| `IT-OFFER-005` | 拒绝 | 发送方最终 rejected，不自动重试 |
| `IT-OFFER-006` | 重复 Offer event | 不创建第二个 transfer/弹窗 |

### 中断与恢复

| ID | 场景 | 期望 |
|---|---|---|
| `IT-RESUME-001` | 10% 处断网 | 从安全 offset 继续，内容一致 |
| `IT-RESUME-002` | 50% 处杀发送进程 | 重启后从接收 offset 继续 |
| `IT-RESUME-003` | 99% 处杀接收进程 | partial/DB 取较小安全值恢复 |
| `IT-RESUME-004` | 用户暂停后重启 | 保持 paused，用户继续才传 |
| `IT-RESUME-005` | 用户取消接收 | partial 删除，状态 cancelled |
| `IT-RESUME-006` | 源 size/mtime 改变 | failed/source_changed，要求重选 |
| `IT-RESUME-007` | 数据全部到达但发布失败 | 不发送 completed 回执 |
| `IT-RESUME-008` | offset 大于源 size | 协议错误且不读取越界 |
| `IT-RESUME-009` | Android 源在连续传输中被修改 | data_end 前检测并停止发布；无法提供 mtime 的 Provider 记录限制 |
| `IT-RESUME-010` | 接收端写盘中途磁盘满 | failed/not_enough_space，不发送 completed；换目录后按真实 partial 长度续传 |
| `IT-RESUME-011` | Android SAF 权限被撤销 | failed/permission_lost，重选 Tree 后恢复且不复用虚假 offset |
| `IT-RESUME-012` | 文件夹发布一半时进程退出 | 已发布项被识别，重启只发布剩余项且不产生重名副本 |

## 群聊测试

| ID | 场景 | 期望 |
|---|---|---|
| `IT-GROUP-001` | 创建者邀请两台设备 | 稳定 GroupId，邀请分别排队 |
| `IT-GROUP-002` | 接受/拒绝邀请 | 只接受者成为 joined |
| `IT-GROUP-003` | 接受后双方重启 | 群永久存在，revision 一致 |
| `IT-GROUP-004` | 群主离线，成员互发文字 | 正常送达；管理按钮禁用 |
| `IT-GROUP-005` | 一在线一离线群文件 | 在线完成，离线 queued，聚合 1/2 |
| `IT-GROUP-006` | 离线成员次日上线 | 原发送者在线时自动补传 |
| `IT-GROUP-007` | 重复群事件 | 每成员只有一条消息 |
| `IT-GROUP-008` | revision 缺口 | 请求全量同步，不直接跳版本 |
| `IT-GROUP-009` | 非 owner 更新成员 | NOT_GROUP_OWNER，群不变 |
| `IT-GROUP-010` | 转让群主 | 旧 owner 发连续快照，之后新 owner 管理 |
| `IT-GROUP-011` | 退出/移除 | 停止新消息，历史保留 |
| `IT-GROUP-012` | 解散 | 所有收到更新的客户端只读，历史保留 |
| `IT-GROUP-013` | 32 台模型 | 成功；第 33 台 GROUP_FULL |
| `IT-GROUP-014` | 群主卸载/永久离线 | 普通消息仍可，管理不可用，不自动选举 |
| `IT-GROUP-015` | 群主直接退出 | GROUP_OWNER_MUST_TRANSFER，群资料不变 |

## 我的设备与剪贴板测试

| ID | 场景 | 期望 |
|---|---|---|
| `IT-CLIP-001` | 发起绑定，对端接受 | 双方 active，显示我的设备 |
| `IT-CLIP-002` | 重复 BindingId | 不重复弹窗 |
| `IT-CLIP-003` | 对端拒绝 | 不 active，自动模式不可开 |
| `IT-CLIP-004` | 任一端解除 | 双方最终 removed/off，停止自动同步 |
| `IT-CLIP-005` | off/send/receive/bidirectional | 每方向严格按配置执行 |
| `IT-CLIP-006` | 双向自动同步一段文字 | 对端写入一次，不回环 |
| `IT-CLIP-007` | 相同内容由用户再次主动复制 | 经过抑制窗口后可作为新 sequence 发送 |
| `IT-CLIP-008` | 文字/图片超限 | 提示改用文件，不上网 |
| `IT-CLIP-009` | 未绑定设备发 automatic | 拒绝并返回 CLIPBOARD_BINDING_REQUIRED |
| `IT-CLIP-010` | 图片同步 | 复用 file_offer，完成后写剪贴板 |

## Flutter UI 与交互测试

| ID | 场景 | 期望 |
|---|---|---|
| `UI-001` | Windows 800px 两栏 | 左栏和聊天区无重叠、截断或横向溢出 |
| `UI-002` | Android 360px 宽度 | 导航、气泡、长文件名可用 |
| `UI-003` | 会话空状态 | 提供查看附近设备入口 |
| `UI-004` | 所有 MessageKind | 摘要和气泡符合产品文档 |
| `UI-005` | 每个 TransferState | 只显示合法按钮 |
| `UI-006` | 离线发送 | 气泡立即显示等待上线 |
| `UI-007` | 群逐成员详情 | 每人状态准确，聚合数量正确 |
| `UI-008` | 事件流中断重订阅 | snapshot 恢复，无重复 toast |
| `UI-009` | 删除会话确认 | 明确说明不删除已保存文件 |
| `UI-010` | 最长设备名/群名/错误文案 | 控件不溢出，必要时换行 |
| `UI-011` | Windows 640px 单栏后恢复到 800px | 列表/聊天返回正确，恢复两栏后保留会话和滚动位置 |
| `UI-012` | 左栏会话/附近 Tab、搜索、设置和传输入口 | 入口无重复导航，切换后列表和右栏行为符合产品文档 |
| `UI-013` | Windows 发送快捷键与输入法 | 普通/小键盘回车发送且保留焦点，Shift+Enter 换行；组合输入不误发、长按不重复、失败保留草稿，Android 行为不变 |
| `UI-014` | 修改本机名称 | 整行可编辑，保存立即刷新；取消不修改，无效输入被拒绝，失败保留草稿 |

Widget 测试使用权威 DTO 枚举构造假数据，不复制状态机到测试 helper。

## Windows 平台测试

| ID | 场景 | 期望 |
|---|---|---|
| `WIN-001` | 启动第二实例 | 激活首实例，第二实例退出 |
| `WIN-002` | 关闭窗口 | 默认进入托盘，传输继续 |
| `WIN-003` | 托盘退出且有活动任务 | 询问，退出后任务下次恢复 |
| `WIN-004` | 防火墙仅专用网络允许 | 家庭网可发现，公用规则未创建 |
| `WIN-005` | 睡眠/恢复 | 任务 queued 后重新发现续传 |
| `WIN-006` | 网络地址变化 | 重绑并立即广播 |
| `WIN-007` | 同目录 partial 发布 | 原子改名，完成前 final 不存在 |
| `WIN-008` | 托盘后台复制 | 允许方向的我的设备收到一次 |
| `WIN-009` | 通知点击/按钮 | 跳转正确聊天并调用统一命令 |

## Android 平台测试

| ID | 场景 | 期望 |
|---|---|---|
| `AND-001` | 首次拒绝通知权限 | 前台服务仍运行且锁屏可收发；通知栏内容隐藏，设置页提供开启入口 |
| `AND-002` | SAF Tree 能力探测成功 | 持久保存目录，可写/rename/delete |
| `AND-003` | SAF Tree 探测失败 | 不保存目录，要求重新选择 |
| `AND-004` | 源 URI 持久权限重启 | 重启后仍可打开并续传 |
| `AND-005` | URI 权限被系统撤销 | FILE_PERMISSION_LOST，可重新选择 |
| `AND-006` | FD 正常、取消、FFI 失败 | 每条路径恰好关闭一次 |
| `AND-007` | Activity 旋转/重建 | 核心任务继续，snapshot 恢复 UI |
| `AND-008` | 系统杀进程 | 下次启动从 SQLite/partial 恢复 |
| `AND-009` | 用户停止前台服务 | 不偷偷重启，任务回 queued |
| `AND-010` | 应用前台复制 | 自动模式发送一次 |
| `AND-011` | 应用后台复制 | 不自动读取，不发送 |
| `AND-012` | 后台通知发送剪贴板 | 打开有焦点预览，确认后发送 |
| `AND-013` | 接收自动剪贴板 | 允许时写系统剪贴板并抑制回环 |
| `AND-014` | 本地 Provider 10GB 文件 | 内容正确，数据不经过 Dart |
| `AND-015` | 发送/接收含空子目录的 Tree URI | directory entry 保留且不创建数据段 |
| `AND-016` | 可读默认名称及旧默认名升级 | 按平台回退规则取名，只做一次旧默认值检查，自定义名称在重启后保留 |

## 故障注入测试

| ID | 故障 | 注入点 | 期望 |
|---|---|---|---|
| `FI-001` | SQLite 写失败 | 每个事务写点 | 回滚，无虚假成功事件 |
| `FI-002` | 磁盘满 | 写 partial 中途 | failed/not_enough_space，可换目录恢复 |
| `FI-003` | TCP reset | 任意数据百分比 | 保存安全 offset 并 queued |
| `FI-004` | 控制回执丢失 | stored/completed | 重发幂等，无重复 |
| `FI-005` | 慢接收端 | 限制读速 | 发送背压，内存不上涨 |
| `FI-006` | UI 事件队列满 | 高频进度 | 合并进度，业务状态可 snapshot 补回 |
| `FI-007` | 平台请求超时 | FD/commit | 权威超时错误，不泄漏句柄 |
| `FI-008` | 文件发布失败 | rename/commit | 不 completed，partial 保留 |
| `FI-009` | 源文件中途改变 | 续传前 | 停止并要求重选 |
| `FI-010` | 畸形对端连续发包 | TCP parser | 限制后关闭连接，进程存活 |

## 性能测试

| ID | 场景 | 门槛 |
|---|---|---|
| `PERF-001` | Rust CLI 10GiB 单文件 | `iperf3` 字节吞吐 90% |
| `PERF-002` | 加入 SQLite/续传 | 相对 PERF-001 降幅 ≤3% |
| `PERF-003` | 加入 Flutter 进度 | 相对 PERF-002 降幅 ≤3% |
| `PERF-004` | 1GbE SSD 最终版 | 三次中位 105-115MB/s |
| `PERF-005` | Wi-Fi 双向 | 每方向 ≥ `iperf3` 85% |
| `PERF-006` | SFTP 对照 | LAN Chat 中位不得更低 |
| `PERF-007` | 100GiB 文件 | 新增内存 ≤128MiB，内容正确 |
| `PERF-008` | 1,000×1MiB | 总耗时不得慢于 SFTP |
| `PERF-009` | 10,000 小文件 | 总耗时/文件数报告，内存不超限 |
| `PERF-010` | 四连接并发 | 全局/每 peer 槽限制正确且无 OOM |

每项保存三次计分原始值和中位数。性能失败必须附 profiler/系统监视证据和 [排查顺序](06-performance.md#不达标排查顺序) 的结论。

### 已执行证据（2026-08-27）

| 测试 | 状态 | 证据 |
|---|---|---|
| `PERF-001` | 通过 | 10GiB LAN 三轮 `210.91/212.20/209.59 MB/s`，中位 `210.91 MB/s`；详见 [性能实测](06-performance.md#2026-08-27-windows-到-android-实测) |
| `PERF-002` | 通过 | 同一 10GiB 源文件、同一无线时段按数据阶段计时：Raw TCP 三轮中位 `189.134MB/s`，正式 SQLite/续传路径中位 `190.276MB/s`，无下降 |
| `PERF-003` | 通过 | 相邻时段仅切换 Flutter 进度消费：关闭三轮中位 `192.952MB/s`，开启中位 `195.240MB/s`，无下降；Debug 开关不在 Release 注册 |
| `PERF-005` PC -> Android 数据路径 | 通过 | 三轮前置 iperf3 为 `206.49/176.16/175.04 MB/s`；LAN 每轮均超过对应 85% 门槛；Wi-Fi 波动已记录 |
| `PERF-006` PC -> Android 数据路径 | 通过 | SFTP 中位 `108.88 MB/s`，LAN 中位 `210.91 MB/s` |
| `PERF-007` 内存子项 | 通过 | 100GiB 实际流式写入，接收峰值 `4.54MiB`、发送峰值 `10.09MiB` |
| `PERF-007` 最终内容签署 | 通过 | 正式 App 向系统真实 SAF Tree 发送非稀疏 100GiB；`576.6s`、`186.22 MB/s`，两端 `107374182400` bytes 且 SHA-256 同为 `f0b14a8d...24d4fd7`；Windows/Android 峰值增量分别为 `34.57MiB`/`1.34MiB` |
| `PERF-005` Android -> Windows | 通过 | 同方向 iperf3 为 `36.06 MB/s`；LAN Chat 三轮中位 `34.040 MB/s`，达到约 `94.4%` |
| `PERF-006` Android -> Windows | 通过 | 同一 10GiB Android 源文件：SFTP 三轮中位 `20.062 MB/s`，LAN Chat 三轮中位 `34.040 MB/s`，快约 `69.7%`；三份最终文件哈希一致 |
| `PERF-008` | 通过 | 同一 Android 源目录的 1,000 个 1MiB 文件：LAN Chat 三轮中位 `36.735s`（约 `28.54MB/s`），SFTP 三轮中位 `148.154s`（`7.078MB/s`）；六个目标的文件数和字节数均准确 |
| `PERF-009` | 通过 | 正式链路完成 10,000 个 4KiB 文件，接收文件数和首尾大小准确，无 `.partial` 残留，outbox 清空 |
| `PERF-010` | 通过 | 四个并发 512MiB 正式任务三轮中位 `17.897s`、聚合 `119.994MB/s`；每轮全部完成、每个文件大小准确、无 partial，单 peer 最多两个任务同时传输 |

`PERF-001` 至 `PERF-010` 均已有三轮计分、最终真机证据或该场景规定的完整正确性证据，`RQ-010` 性能验收完成。原始值和无线波动处理见 [性能实测](06-performance.md#sqlite-与-flutter-进度分阶段-ab)。

## 端到端验收剧本

### `E2E-001` 首次 Windows → Android

1. 两端清空应用数据后启动并设置名称。
2. 3 秒内在附近页互现。
3. Windows 进入 Android 私聊，发送文字和三个文件。
4. Android 首次 Offer 选择 SAF 本地目录并接受。
5. 验证文字、文件、通知、聊天摘要和最终内容。
6. 再发送文件，验证已知设备默认自动接收。

### `E2E-002` 离线永久队列

1. Android 完全停止应用/服务。
2. Windows 发送文字和 10GiB 文件，验证立即 queued。
3. Windows 重启，任务仍存在。
4. Android 打开，验证自动送达并完成。

2026-08-27 实机结果：四个步骤均通过。文字和 10GiB 文件均在 Android 完全停止时排队；Windows 重启后任务仍存在；Android 打开后自动送达且无重复。消息 ID、传输 ID、SHA-256、状态、回执和 outbox 清理证据见“当前自动化与真机证据”。

### `E2E-003` 三设备永久群

1. W1 邀请 W2、A1 创建群。
2. W2 接受，A1 离线后再接受。
3. W1 离线；W2 和 A1 互发文字/文件。
4. W1 上线补收消息。
5. W1 转让群主给 A1，A1 修改群名并移除 W2。
6. 三端重启，验证群版本和只读/成员状态。

### `E2E-004` 我的设备剪贴板

1. W1 与 A1 双方确认绑定，默认模式仍 off。
2. 开启双向；W1 后台复制文字，A1 前台收到并写入。
3. 验证 A1 不把远端写入回发。
4. A1 进入后台后复制，验证不会自动发送。
5. 点击通知按钮，确认预览后发送到 W1。
6. 解除绑定，验证自动同步立即停止。

### `E2E-005` 断线、空间和恢复

1. 传输 100GiB 测试文件。
2. 10% 断网、50% 杀接收进程、99% 暂停并重启。
3. 每次恢复验证偏移没有倒退超过最近持久化窗口，也没有跳过内容。
4. 接收端制造空间不足，换目录后继续。
5. 最终验证大小和测试 SHA-256。

## 需求追踪矩阵

| 需求 | 主要接口/协议 | 实施任务 | 核心测试 |
|---|---|---|---|
| RQ-001 | discover/announce, PeerEvent | NET-01 | IT-NET-001..009 |
| RQ-002 | send_text, text_message, receipt | DB-02, NET-03, MSG-01 | IT-MSG-001..007 |
| RQ-003 | send_sources, file_offer, data_entry | FILE-01..03 | IT-FILE-001..008 |
| RQ-004 | pause/resume/cancel, transfer state | CORE-02, FILE-04 | IT-RESUME-001..009 |
| RQ-005 | ReceivePolicy, OfferDecision | FILE-02 | IT-OFFER-001..006 |
| RQ-006 | Group API, invite/update/sync | GROUP-01..03 | IT-GROUP-001..013 |
| RQ-007 | per-member outbox/transfer | GROUP-03 | IT-GROUP-004..006,014..015 |
| RQ-008 | binding API/messages | CLIP-01 | IT-CLIP-001..004 |
| RQ-009 | ClipboardMode/PlatformRequest | CLIP-02..03 | IT-CLIP-005..010, AND-010..013 |
| RQ-010 | data socket/buffer/scheduler | PERF-01..02, QA-02 | PERF-001..010 |
| RQ-011 | schema/outbox/processed events | CORE-02, DB-01..02, FILE-04 | DB-001..013, FI-001..009 |
| RQ-012 | PlatformAdapter | WIN-01, AND-01..03 | WIN-001..009, AND-001..016 |
| RQ-013 | Core DTO/ViewModel | UI-01..02 | UI-001..014, E2E-001..005 |

任何新需求必须先获得新 `RQ-` ID，并同时更新产品文档、实施任务和测试，不允许只有 UI 或只有协议改动。

## 文档验收

| ID | 检查 |
|---|---|
| `DOC-001` | 所有相对 Markdown 链接指向存在文件和标题 |
| `DOC-002` | 常见设计占位标记的全文搜索结果为空 |
| `DOC-003` | 权威枚举值只在数据文档定义，其他文档引用一致 |
| `DOC-004` | 协议所有 JSON 示例可解析并符合必填字段 |
| `DOC-005` | 当前 Schema V3 SQL 可执行，且结构与 V1 → V3 迁移结果一致 |
| `DOC-006` | 每个 RQ 都映射接口、任务和测试 |
| `DOC-007` | 每份文档都有目的、前置知识、规范、示例、异常和检查表 |
| `DOC-008` | 未参与设计的初中级开发者能复述 M3 实现而无需补决策 |

## 发布判定

发布候选必须同时满足：

- 所有 Rust、Flutter、Kotlin 自动测试通过。
- `E2E-001` 至 `E2E-005` 在规定真机矩阵通过。
- `PERF-004` 至 `PERF-010` 通过或有明确“不适用”的硬件证据；核心吞吐门槛不可豁免。
- 没有会导致文件错误完成、覆盖已有文件、重复消息、无限剪贴板循环或无法续传的已知缺陷。
- 严重崩溃和数据损坏缺陷为 0。
- DOC-001 至 DOC-008 通过。

## 示例：缺陷报告最低信息

```text
测试 ID: IT-RESUME-003
构建: 0.1.0+abc123
设备: W1 -> A1
步骤: 10GiB 文件 99% 时强杀接收进程
实际: 重启后从 0 开始
期望: 从数据库与 partial 较小安全 offset 继续
TransferId: ...
日志时间: ...
源/目标大小: ...
是否可复现: 3/3
```

不得把包含消息正文、剪贴板正文或完整私人路径的日志附到公共缺陷中。

## 异常情况

- 真机数量不足 32：使用 harness 验证规模和数据模型，至少三台真机验证真实群网络。
- 无法创建 100GiB 文件：该设备不能签署最终性能验收，不能用稀疏文件代替。
- SFTP 未安装：先安装系统 OpenSSH 测试能力；不能跳过对照。
- OEM 杀后台进程：记录厂商和设置，功能按 queued 恢复验收，不要求绕过系统限制。
- 云盘 SAF Provider 慢：不计满速门槛，但仍验证明确提示和功能恢复。
- 测试哈希不同：无论 UI 是否 completed，均为阻断发布的数据损坏缺陷。

## 实现检查表

- [ ] 每个任务 PR 引用至少一个测试 ID。
- [ ] 每种网络事件都有合法、重复、错字段和超限测试。
- [ ] 每个传输状态转换都有正反单测。
- [ ] SQLite 测试使用真实数据库与故障注入。
- [ ] 文件测试验证最终内容，不只验证字节数。
- [ ] Windows 和 Android 均有生命周期及权限测试。
- [ ] 群主离线和成员离线是独立场景。
- [ ] 剪贴板双向模式验证不会循环。
- [ ] 性能报告包含 iperf3、SFTP 和 LAN Chat 三组原始值。
- [ ] 发布前完成需求追踪和初中级开发者复述验收。
