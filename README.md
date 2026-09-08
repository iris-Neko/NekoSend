# 猫猫快传：局域网聊天与文件传输

## 目的

本仓库包含产品与技术规范，以及 Windows、Android、Linux 和 macOS 客户端源码。文档面向第一次接触本项目的初级、中级程序员，目标是让实现者按照给定顺序阅读后，可以直接拆任务、编写代码和验收，而不需要重新决定产品行为、网络格式、数据库字段或状态转换。

产品名称为 **猫猫快传**，英文名 **NekoSend**。代码中的 `lan_chat`、`LAN Chat` 数据目录和协议标识是兼容性名称，不随产品展示名称改变。

## 前置知识

发布安装包无需用户自行编译：推送匹配版本号的 `v*` 标签后，GitHub Actions 自动构建四个平台，全部通过后统一发布到 Releases。维护者也可手动补跑已有标签；流程、签名备份和升级注意事项见 [自动发布说明](packaging/RELEASING.md)。

实现者应具备以下基础能力：

- 能阅读 Flutter/Dart、Rust、JSON 和 SQL。
- 理解 TCP 是有序字节流，而不是消息队列。
- 理解 Android Activity、前台服务、通知和 Storage Access Framework（SAF）的基本概念。
- 能使用两台真实设备进行同一局域网测试。

不要求实现者提前掌握自定义加密协议、分布式一致性算法或公网 NAT 穿透；这些均不属于 V1。

## 当前开发状态

当前 0.3.0 已包含 Windows、Android、Linux Flatpak 和 macOS 客户端，并接入四平台自动发布与固定 Android 发布签名，尚未完成完整 V1 真机验收。Windows + Android 14 + Android 16 三实体群聊以及 Linux Flatpak 的发现、消息、文件往返已有验证；macOS 已通过云端启动检查，跨设备互传仍待社区测试。已知限制包括 Android 16 样机后台冻结、尚未完整执行的多厂商/双 Windows 矩阵及人工文档/安装验收，详见 [测试计划](docs/08-test-plan.md)。

- Rust workspace 与 Windows/Linux CI。
- `core_rust` 健康检查、稳定 ID、权威枚举和传输状态机。
- `perf_harness` 原始 TCP 大文件收发命令，文件字节不经过 Flutter、JSON、哈希或 SQLite。
- `integration_harness` TCP 回环传输与接收端持久化确认测试。
- Flutter Windows/Android 工程、Windows 双栏布局、窄窗口单栏路由和 Android 底部导航。
- 完整 SQLite Schema V6、V1 → V6 无损迁移、WAL 配置、稳定 `DeviceId`、附近设备持久化和私聊文字事务；V4 新增 Linux 平台值，V5 新增头像缓存，V6 新增 macOS 平台值，升级数据库后不可用旧版本应用直接打开。
- macOS 0.3.0 测试包：原生菜单栏、文件选择、剪贴板和通知适配，云端构建 Universal DMG/ZIP；本地临时签名，无 Apple 公证。详见 `packaging/macos/README.md`。跨 Mac 通信需要其他端也更新到 0.3.0。
- UDP 自动发现（2 秒广播、7 秒离线）已在 Windows 与 Android 真机同一 Wi-Fi 双向验证。
- 每台设备可在“设置 -> 设备名称”修改自己的名称；Android 默认优先使用系统提供的可读机型，兼容升级旧默认型号代码且不覆盖自定义名称。改名保持 DeviceId 不变，并同步附近列表和已有私聊标题。
- 私聊和群聊消息显示发送者头像与昵称；“设置 -> 头像”提供 12 款内置图案，按 DeviceId 稳定分配默认头像，选择后同步给附近设备并在离线/重启后保留。昵称沿用设备名称，不新增账号。
- TCP `53318` 长期双向控制连接、4 字节大端长度帧、Hello、同连接双向消息、`ping/pong`、投递回执、去重和永久 outbox 重试；同时建连时按 `(initiator_device_id, connection_id)` 确定性保留一条连接。
- Rust → Flutter 使用容量 1,024 的事件流；消息、邀请、绑定、在线状态和最多每 250ms 一次的传输进度按类别局部刷新，断流后一秒重订阅并用快照校正。
- Android 已接入前台服务，用于维持 Flutter/Rust 引擎、UDP/TCP 和后台发送队列；实际后台可用性仍需逐机型验证，后台剪贴板读取按系统限制禁用。
- 真实会话、私聊、永久本地群聊、群成员管理、文件气泡、传输任务筛选/清理/跳转、接收确认、暂停/续传/取消、完成文件打开/显示位置和剪贴板同步均已接入 Rust 核心。
- 接收端区分磁盘满、目录失效和权限丢失；失败后可重新选择 Windows 目录或 Android SAF Tree，并按新目录的实际安全偏移续传。
- Android SAF 写入中途返回系统级 `ENOSPC` 的真机链路已通过：发送、接收两端均进入 `failed/not_enough_space`，关闭故障并更换接收目录后从 `929792` bytes 继续，最终 4MiB 文件大小和 SHA-256 一致且相关 outbox 清空。
- Windows -> Android Rust 数据路径 10GiB 三轮中位 `210.91 MB/s`，同环境 SFTP 中位 `108.88 MB/s`；Android -> Windows 正式应用数据路径三轮中位 `34.040 MB/s`，同文件 SFTP 中位 `20.062 MB/s`。正式 App 通过系统真实 SAF Tree 接收非稀疏 100GiB 文件：锁屏下耗时 `576.6s`、有效吞吐 `186.22 MB/s`、Windows 工作集峰值增量 `34.57MiB`、Android PSS 峰值增量 `1.34MiB`，两端 SHA-256 一致。SQLite 续传与 Flutter 进度的分阶段 A/B 均未造成超过 3% 的中位吞吐下降；1,000 个 1MiB 同源文件和四个并发 512MiB 任务也已完成正式计分。
- `flutter_rust_bridge` 代码生成与 Native Assets 构建钩子；应用启动时会加载 Rust 核心并读取核心/协议版本。
- Windows Release 构建和 Flutter → Rust 真实调用测试。
- Android 14 样机锁屏 `Dozing` 时前台服务和双向消息已验证可用；Android 16 样机在 `isForeground=true` 时仍出现内核进程冻结，不能据此宣称所有受支持 Android 设备的锁屏收发均已通过。
- 三实体节点已验证离线邀请补发、群主离线时手机间文字与文件直传、发送手机重启后的永久队列、群主上线补收、转让群主、改名、移除成员与权限限制，以及三端重启后的群资料和历史保留。Android 16 的冻结恢复需要重新激活应用，仍单独列为未通过项。

运行 Rust 质量门禁：

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

运行 Flutter 质量门禁：

```powershell
Set-Location L:\app_flutter
dart format lib test hook
flutter analyze
flutter test
flutter build windows --debug
flutter build apk --debug --target-platform android-arm64
```

开发工具目录已加入用户 `PATH` 后，命令行直接使用 `cargo`、`flutter`、`dart` 和 `adb`，不得在脚本中写工具的安装绝对路径。Flutter 3.47 的 Windows 工具链不能正确处理当前仓库的中文绝对路径；本机已将仓库持久映射为 `L:`，Flutter 命令统一从该入口运行：

```powershell
Set-Location L:\app_flutter
flutter build windows
```

Rust 代码仍从工作区编辑；`L:` 只作为 Flutter、Gradle 和 MSBuild 的构建入口。

Android 构建使用项目 Gradle Wrapper。当前网络环境的依赖镜像已配置在用户级 Gradle `init.gradle`，仓库不写机器相关路径：

```powershell
Set-Location L:\app_flutter\android
.\gradlew.bat app:assembleDebug --no-daemon -Ptarget-platform=android-arm64
adb install -r ..\build\app\outputs\flutter-apk\app-debug.apk
```

Android Native Assets 必须显式指定 `android-arm64`；否则构建器会同时请求首发不支持的 armv7 Rust 目标。

发布 APK 不允许默认使用 debug 证书。正式签名从系统环境变量 `LAN_CHAT_ANDROID_STORE_FILE`、`LAN_CHAT_ANDROID_STORE_PASSWORD`、`LAN_CHAT_ANDROID_KEY_ALIAS`、`LAN_CHAT_ANDROID_KEY_PASSWORD` 读取，也兼容不提交仓库的 `app_flutter/android/key.properties`。四项必须同时配置；本地仅验证 Release 二进制时可显式设置 `LAN_CHAT_ALLOW_DEBUG_RELEASE_SIGNING=true`（直接调用 Gradle 也可传 `-PlanChatAllowDebugReleaseSigning=true`），该产物不得发布。

性能样机命令：

```powershell
cargo run --release -p perf_harness -- receive --listen 0.0.0.0:53319 --output D:\receive.bin
cargo run --release -p perf_harness -- send --connect 192.168.1.20:53319 --file D:\source.bin
```

两个命令最终输出 JSON 指标。发送端只有在接收端完成 `flush + sync_all` 并返回确认后才结束计时。

## V1 产品定义

LAN Chat 是运行在同一 IPv4 子网中的聊天式传输工具。用户像使用常见聊天软件一样选择附近设备或已有会话，然后发送文字、文件、图片、文件夹和剪贴板内容。会话、群资料、消息记录和未完成任务永久保存在本机。

固定平台与技术边界：

- 客户端：Windows 10 x64、Android 13（API 33）及以上、Linux x86_64（Flatpak / GNOME 50 runtime）、macOS 13 及以上（Apple Silicon / Intel）。
- UI：Flutter。
- 共享核心：Rust。
- 数据库：SQLite WAL。
- 发现：IPv4 UDP 广播。
- 控制协议：TCP 上的长度前缀 UTF-8 JSON。
- 文件协议：独立 TCP 连接上的长度前缀 JSON 头和连续原始字节。
- 网络信任：V1 假定家庭局域网可信，使用明文 TCP。
- 性能目标：相同设备、网络和文件条件下不得慢于 SFTP。

## 明确不做

- 用户账号、手机号、邮箱登录或多设备云账号。
- 互联网传输、跨 VLAN、NAT 穿透、云中继或云备份。
- TLS、证书、密码、端到端加密或复杂身份认证。
- iOS 客户端；macOS 已加入测试构建，但真机验收和 Apple 公证尚未完成。
- 语音、视频、音视频通话、表情商店、朋友圈等社交功能。
- Android 后台静默读取系统剪贴板。
- 文件压缩、内容去重、增量文件同步或应用层分块哈希。

## Linux 安装与构建

Linux 从 0.2.0 起提供真实桌面客户端和 Flatpak 包，沿用聊天、群聊、文件与恢复工作流。其他设备也需更新到 0.2.0 或以上才能识别 Linux 平台。

```sh
flatpak install --user ./NekoSend-linux-x86_64.flatpak
flatpak run io.github.iris_neko.NekoSend
```

构建清单、权限说明和源码构建步骤见 [Linux / Flatpak](packaging/linux/README.md)。构建产物位于 `dist/`，不提交到源码 Git；通过 GitHub Release 或 CI artifacts 分发，不代表已上架 Flathub。Linux 数据存于 `~/.var/app/io.github.iris_neko.NekoSend/data/NekoSend/`。

## 文档阅读顺序

| 顺序 | 文档 | 解决的问题 |
|---:|---|---|
| 1 | [产品与交互](docs/01-product-ux.md) | 用户能看到什么、点击后发生什么 |
| 2 | [总体架构](docs/02-architecture.md) | Flutter、Rust、平台代码如何分工 |
| 3 | [网络协议](docs/03-network-protocol.md) | 两台设备如何发现、连接、发消息和传文件 |
| 4 | [数据与 API](docs/04-data-and-api.md) | 状态、表结构、Rust API、Flutter 事件和错误码 |
| 5 | [平台行为](docs/05-platform-behavior.md) | Windows 与 Android 的系统集成差异 |
| 6 | [性能规范](docs/06-performance.md) | 如何达到 SFTP 级速度及如何测量 |
| 7 | [实施指南](docs/07-implementation-guide.md) | 按什么顺序开发，每一步如何验收 |
| 8 | [测试计划](docs/08-test-plan.md) | 必须覆盖哪些正常、异常和性能场景 |

不得跳过第 3、4 份文档后直接实现网络或数据库。UI 开发可以使用假数据提前进行，但假数据字段必须来自 [数据与 API](docs/04-data-and-api.md)。

## 唯一权威来源

为避免同一个状态或字段在多处被不同地解释，以下内容只能在指定文档中定义：

| 内容 | 唯一权威文档 |
|---|---|
| 产品流程、页面和用户文案 | `01-product-ux.md` |
| 模块边界、线程与依赖方向 | `02-architecture.md` |
| UDP/TCP 报文格式、消息名和超时 | `03-network-protocol.md` |
| ID、状态枚举、SQLite DDL、FFI API、错误码 | `04-data-and-api.md` |
| Windows/Android 生命周期和权限 | `05-platform-behavior.md` |
| 缓冲、并发、吞吐和基准方法 | `06-performance.md` |
| 任务顺序和单任务完成条件 | `07-implementation-guide.md` |
| 测试用例与最终验收门槛 | `08-test-plan.md` |

修改权威定义后，必须搜索所有引用并同步示例。其他文档可以解释权威定义如何被使用，但不得增加同名枚举值、报文字段或错误码。

## 核心术语

| 术语 | 含义 |
|---|---|
| 设备 | 一次 LAN Chat 安装实例。卸载并清除数据后视为新设备。 |
| 附近设备 | 最近在同一子网发现、但不一定建立过会话的设备。 |
| 已知设备 | 用户接受过其文件、群邀请或主动与其建立过会话的设备。 |
| 我的设备 | 两端用户完成双方确认绑定的设备关系，只用于剪贴板与自动接收策略。 |
| 私聊 | 两台设备之间的永久本地会话。 |
| 群聊 | 最多 32 台设备参与、资料分布保存在各成员本地的永久会话。 |
| 群主 | 唯一有权修改群资料和成员列表的设备；不承担消息中继。 |
| 控制连接 | 发送 JSON 控制消息的长期 TCP 连接。 |
| 数据连接 | 为一个传输任务发送连续文件字节的短期 TCP 连接。 |
| Offer | 发送方提出的文件、文件夹或剪贴板图片接收请求。 |
| 待发送任务 | 因对方离线或暂停而保存在本机、等待以后继续的消息或文件任务。 |

## 固定默认行为

- 应用启动后自动发现设备，无需二维码或配对码。
- 第一次接收未知设备的文件时必须确认；确认后该设备成为已知设备。
- 已知设备默认自动接收文件，可在该设备设置中改为每次询问。
- 任务没有自动过期时间，成功或用户取消后才结束。
- 源文件发生移动、删除、大小变化或修改时间变化时停止任务，要求重新选择。
- 接收文件绝不覆盖已有同名文件，使用 `name (1).ext`、`name (2).ext` 递增。
- 群邀请必须由受邀设备确认；接受后群永久保存。
- 自动剪贴板同步默认关闭，且必须先完成“我的设备”双方绑定。
- Windows 可以后台自动监听剪贴板；Android 只有应用处于前台时自动监听，后台使用通知按钮主动发送。

## 示例：最小用户路径

1. 电脑和手机连接同一个家庭 Wi-Fi 并打开 LAN Chat。
2. 两端在 3 秒内出现在“附近设备”。
3. 电脑点击手机，进入空白私聊并发送一个文件。
4. 手机首次弹出接收确认；用户接受并选择接收目录。
5. 文件作为消息气泡显示进度，完成后可以打开或定位。
6. 以后电脑再次向该手机发送文件，手机按设备默认策略自动接收。
7. 用户勾选电脑和手机创建“家里设备”群；手机确认邀请后，群在两端永久存在。

## 异常处理原则

- 所有失败必须落到 [权威错误码](docs/04-data-and-api.md#错误码)，UI 不得只显示“未知错误”。
- 网络断开不是永久失败；可重试消息和传输回到可恢复状态并等待设备上线。
- 数据库写入成功后才可以向 UI 报告“已排队”。
- 文件完成写入、刷新并原子改名后才可以报告“已完成”。
- 应用不得因为单个畸形报文、无效路径、磁盘不足或对端退出而崩溃。
- 无法安全恢复的状态必须保留原始任务记录，并给用户明确的“重试”“重新选择”或“取消”操作。

## 文档与实现检查表

- [ ] 新实现者已按顺序阅读 01-04 文档。
- [ ] 所有产品行为都能在 `01-product-ux.md` 找到唯一说明。
- [ ] 所有网络字段都能在 `03-network-protocol.md` 找到类型、必填性和示例。
- [ ] 所有状态、表字段、API 和错误码都能在 `04-data-and-api.md` 找到定义。
- [ ] Windows 与 Android 的差异没有被塞入 Flutter 页面或通用 Rust 领域逻辑。
- [ ] 性能测试使用 `06-performance.md` 的同机同网对比方法。
- [ ] 每个实施任务都能映射到至少一个测试用例。
- [ ] 文档链接检查、Markdown 检查和术语搜索均通过。
