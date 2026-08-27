# Windows 与 Android 平台行为

## 目的

本文档规定 Windows 10 x64 和 Android 13+ 的启动、文件访问、后台运行、通知、剪贴板、网络变化和进程恢复行为。平台适配器只实现系统能力，不实现消息、群或传输状态机。

## 前置知识

- 已阅读 [总体架构](02-architecture.md) 的文件句柄边界。
- 已阅读 [数据与 API](04-data-and-api.md) 的 `PlatformRequest` 和错误码。
- Android 开发者理解 Activity、Service、SAF、URI 持久授权和 `ParcelFileDescriptor`。

## 平台规范与共同行为

- 平台适配器提供设备显示名建议、应用数据目录、文件选择、接收目录、文件句柄、通知、剪贴板和生命周期事件。
- 平台适配器不得生成 MessageId、修改传输状态或拼接网络报文。
- 所有平台回调携带 `PlatformRequestId`，重复回调由 Rust 幂等拒绝。
- 平台操作失败必须映射到 [权威错误码](04-data-and-api.md#错误码)，同时保留原始系统错误到诊断日志。
- 系统进入休眠、网络切换或应用失焦时，不删除任务；恢复后通知 Rust 重新发现和连接。

## Windows 10 x64

### 进程与单实例

- 使用当前用户范围的命名互斥体 `Local\LANChat.SingleInstance.V1`。
- 第二个进程检测到互斥体后，通过主窗口 `WM_COPYDATA` 传递可选的 `openConversation:<conversation_id>` 路由，恢复首个进程窗口后退出。
- 第二个进程不得再次打开 SQLite、绑定端口或启动托盘图标。
- 首个进程只接受固定 `dwData`、2-4096 字节且以 NUL 结尾的 `WM_COPYDATA`；非会话路由只恢复窗口，不执行其他命令。

### 本地目录

```text
%LOCALAPPDATA%\LAN Chat\
  lan_chat.db
  lan_chat.db-wal
  lan_chat.db-shm
  logs\
  cache\thumbnails\
```

默认接收目录：

```text
%USERPROFILE%\Downloads\LAN Chat
```

- 应用启动时创建数据目录和日志目录。
- 接收目录在首次启动创建；失败则进入阻断设置页。
- `.partial` 文件必须创建在最终目标文件同一目录，以便同卷原子改名。
- partial 名为 `.lanchat-<transfer_id>-<entry_id>.partial`，UI 和文件扫描不展示为完成文件。

### 文件选择和打开

- 文件选择器允许多选，返回绝对规范化路径。
- 文件夹选择后由 Rust 递归枚举文件与目录；清单显式保留空目录。跳过符号链接、目录联接点和无法读取项，并在确认发送前列出跳过数量。
- V1 不跟随符号链接，避免循环和发送目录外内容。
- 打开源文件时使用共享读，允许其他应用读取；不请求共享写，防止传输中被覆盖。
- 传输前和每次续传前重新读取大小、修改时间，与数据库元数据比较。
- 接收端以 `CREATE_NEW` 语义创建 final/partial 名称，重名时按 `(n)` 规则重试，绝不覆盖。
- 完成后 `FlushFileBuffers`，关闭句柄，再使用同目录原子重命名发布。
- 完成项“打开”使用系统文件关联；“显示位置”使用 `explorer.exe /select,`。文件夹发送完成后必须传递根目录，而不是任意子文件。

### Windows 文件名规则

接收相对路径按以下顺序清理：

1. 拒绝绝对路径、盘符、UNC、`.`、`..` 和 NUL。
2. 将 `< > : " / \ | ? *` 替换为 `_`。
3. 去掉每段末尾的空格和点。
4. `CON`、`PRN`、`AUX`、`NUL`、`COM1`-`COM9`、`LPT1`-`LPT9` 后追加 `_`。
5. 空段改为 `_`。
6. 每段最多 200 个 Unicode 字符；超出时保留扩展名并截断主名。
7. 最终绝对路径不得超过应用采用的长路径 API 限制；失败映射 `FILE_INVALID_PATH`。

显示仍使用发送方原始名称；实际保存名存入 `final_display_name`。

### 托盘和窗口

- 关闭主窗口且“关闭时最小化到托盘”开启：隐藏窗口，核心继续运行。
- 托盘菜单固定为“打开猫猫快传”“发送剪贴板”“暂停全部传输”“退出”。
- “退出”先向 Rust 调用 `shutdown()`，最多等待 5 秒持久化；超时记录错误后退出，任务下次恢复。
- 活动传输时退出需要确认；确认只退出程序，不取消任务。
- 开机启动使用当前用户启动项，不申请管理员权限。

### Windows 防火墙

- 安装包为程序申请 Windows Defender 防火墙“专用网络”入站 UDP 53317 和 TCP 53318。
- 不申请“公用网络”规则。
- 无管理员权限无法创建规则时，首次监听可能触发系统提示；UI 显示“请允许专用网络访问”。
- 用户拒绝后映射网络绑定/发现诊断，不反复弹自定义伪权限窗口。
- CI 执行 `scripts/verify_windows_installer.ps1` 检查安装脚本语法、规则数量、端口和 `Private` Profile。安装后的管理员终端使用 `scripts/verify_windows_installer.ps1 -Live -ExecutablePath <安装后的 lan_chat.exe>` 只读核对真实规则。

### Windows 剪贴板

- 使用系统剪贴板变化监听，不轮询。
- 支持 Unicode 文本和单张位图/PNG。
- 自动监听在托盘中继续，但只向 `ClipboardMode` 允许发送的 active 我的设备发送。
- 远端写入时附加应用自定义剪贴板格式 `LAN_CHAT_ORIGIN`，内容为 origin DeviceId、sequence 和 fingerprint。
- 若其他应用剥离自定义格式，使用最近 10 秒内“内容指纹 + 长度”兜底抑制一次回环。
- 打开剪贴板被其他应用占用时退避 50、100、200、400ms，之后返回平台错误；不得阻塞 UI 线程。

### Windows 通知

- 使用系统 Toast；通知点击激活现有单实例并跳转会话。
- 通知按钮通过命名管道/应用激活参数进入 Flutter，再调用统一 Rust 命令。
- 通知正文不显示剪贴板全文；文件通知只显示发送设备、文件名和大小。

### 睡眠和网络变化

- 监听系统睡眠/恢复和网络地址变化。
- 睡眠前不强制取消任务；Socket 自然中断后回到 queued。
- 恢复或 IPv4 地址变化时立即重绑 UDP、重新广播并重连已知待发送设备。
- 不缓存网卡广播地址超过一次网络变化事件。

## Android 13+（API 33）

### SDK 与 ABI

- `minSdk = 33`。
- V1 构建 `arm64-v8a`；开发调试可额外构建 `x86_64` 模拟器，但性能验收只用真机。
- `compileSdk` 和 `targetSdk` 固定为创建工程时 Flutter 稳定版支持的最新已安装稳定 SDK，且不得低于 35。
- Rust 产物通过 Android NDK 构建为 `arm64-v8a` 动态库。

### AndroidManifest 权限

V1 只声明实际使用的权限：

```xml
<uses-permission android:name="android.permission.INTERNET" />
<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.WAKE_LOCK" />
```

不得申请 `MANAGE_EXTERNAL_STORAGE`、`READ_EXTERNAL_STORAGE`、`WRITE_EXTERNAL_STORAGE`、无障碍服务或默认输入法身份。普通 IPv4 UDP/TCP 不需要定位权限；若未来改用受限制 Wi-Fi API，必须另行修订文档。

### SAF 源文件

- 文件/图片使用 `ACTION_OPEN_DOCUMENT`，多文件使用 `EXTRA_ALLOW_MULTIPLE`。
- 文件夹使用 `ACTION_OPEN_DOCUMENT_TREE`。
- 对返回 URI 调用 `takePersistableUriPermission(READ)`；不能持久化时允许本次发送，但应用重启后任务进入 `FILE_PERMISSION_LOST`。
- Kotlin 先查询显示名、大小、修改时间；缺少大小的文档不允许作为 V1 发送源，提示用户换用文件提供器。
- Rust 持久化 `source_ref` 为 URI token，不持久化临时整数 FD。
- 每次开始/续传时，Rust 发出 `OpenSourceFile`；Kotlin 用 `openFileDescriptor(uri, "r")`，校验元数据，`detachFd()` 后交给 Rust。
- 文件夹枚举必须同时返回 directory 和 file SourceItem；空目录作为 size=0 的 directory entry 保留，只有 file entry 在发送时打开 FD。

### SAF 接收目录

- 第一次接受文件时使用 `ACTION_OPEN_DOCUMENT_TREE`。
- 持久化申请 `READ | WRITE` URI 权限，把 Tree URI 作为 `default_receive_ref`。
- 选择目录后必须执行能力探测：创建小型临时文档、写入、刷新、重命名、删除。任一步失败则提示选择其他目录，不保存该 Tree URI。
- Rust 按清单父子顺序发出 `CreateReceiveDirectory`；Kotlin 创建并返回目录引用。`CreateReceiveFile` 再在已存在的父目录下创建 partial 文档并返回 FD/URI token。
- partial 显示名使用 `.lanchat-<entry_id>.partial`；文档提供器禁止点开头时使用 `lanchat-<entry_id>.partial`。
- 完成时 `CommitReceiveFile` 关闭并刷新 FD，使用 `DocumentsContract.renameDocument` 改为最终名称。
- 完成文件“打开”使用 `ACTION_VIEW` 和实际 `content://` URI；文件夹“打开”及“显示位置”使用 `ACTION_OPEN_DOCUMENT_TREE`，通过 `DocumentsContract.EXTRA_INITIAL_URI` 定位根目录或已保存的接收 Tree。不得用 `resolveActivity()` 预判可用性，直接启动并把 `ActivityNotFoundException` 映射为平台错误。
- 由于第三方文档提供器不保证真正原子，V1 只支持通过能力探测的提供器；rename 失败时任务保持 failed，partial 不当成完成文件。
- 系统撤销持久 URI 权限后映射 `FILE_PERMISSION_LOST`，用户重新选择目录再恢复。

### FD 所有权

```text
Kotlin open ParcelFileDescriptor
  -> detachFd() 返回 raw fd
  -> FFI complete_platform_request(fd)
  -> Rust 从 raw fd 构造 owned file
  -> Rust 成为唯一关闭者
```

- `detachFd()` 后 Kotlin 不调用 `close(fd)`。
- FFI 调用失败时，Kotlin 必须关闭尚未成功移交的 raw fd。
- Rust 收到 FD 后即使任务取消也必须在 finally/drop 路径关闭。
- 不允许 Dart 读取 FD 内容或转换为 `Uint8List`。

### 前台服务

Android 后台没有云推送，因此持续可达依赖前台服务。规则：

- “后台保持在线”默认开启；通知权限只控制通知栏展示，不作为启动前台服务的前置条件。
- 应用进入后台且设置开启时启动可见前台服务，保持 Rust 核心、UDP/TCP 和待发送队列运行。
- 有活动传输时，无论设置如何都尝试保持前台服务，除非用户明确停止服务。
- 服务通知固定显示在线/传输摘要，提供“发送剪贴板”“暂停全部传输”“打开应用”。
- 用户从通知停止服务后，网络任务回到 queued；不得偷偷重启，直到用户再次打开应用或开始传输。
- Android/厂商系统仍可能停止进程，因此后台在线是尽力而为；恢复时依靠 SQLite 和 partial 续传。
- 若目标 SDK 对 `dataSync` 前台服务施加运行时限，达到限制前保存状态并停止服务，显示“系统已暂停后台连接，打开应用可继续”，不得循环规避系统限制。

### 通知权限

- 首次启动请求 `POST_NOTIFICATIONS` 前显示一次用途说明。
- 用户拒绝后仍启动前台服务并保持局域网连接；Android 只会隐藏通知栏通知，系统仍可在活动应用管理界面显示该服务。
- 不在用户拒绝后每次启动重复请求；只有用户主动开启后台能力时再次引导。
- 设置页单独选择接收目录时只执行 Tree 选择与能力探测，不得用空传输清单调用文件接收准备接口。

### Android 剪贴板

Android 10+ 规定普通应用只有作为当前焦点应用或默认输入法时才能读取系统剪贴板。本项目不实现输入法，因此：

- Flutter Activity 在前台且有焦点时可以注册剪贴板变化监听。
- Activity 失去焦点时停止自动读取；前台服务不得尝试读取。
- 后台通知“发送剪贴板”打开一个可见的轻量 Activity，取得焦点后读取并显示预览；用户确认后发送。
- 收到允许自动接收的远端内容时，平台可以写入系统剪贴板并显示来源通知。
- 远端写入后保存 origin/sequence/fingerprint，下一次前台监听遇到相同内容时不回发。
- 锁屏时不弹出剪贴板正文；通知只写“收到来自「设备名」的剪贴板”。

### Activity 与进程生命周期

- Activity 重建不重启 Rust 核心；重新绑定事件流并调用 `get_app_snapshot()`。
- Flutter Engine/进程被系统杀死后，所有 Socket 关闭；下次用户或系统合法启动时执行完整恢复。
- `onPause` 只改变剪贴板监听，不暂停网络任务。
- `onTrimMemory` 清理缩略图和 UI 缓存，不清除协议清单和传输缓冲池中的活动缓冲。
- 网络切换通过 `ConnectivityManager.NetworkCallback` 上报 Rust；只在具备 IPv4 且非计量限制策略允许时广播。
- V1 不自动使用移动数据发现；没有 Wi-Fi/Ethernet 局域网时显示离线。

### Android 返回与通知导航

- 系统返回遵循 Flutter 路由栈，不通过 `SystemNavigator.pop` 强制退出。
- 通知使用稳定 route 参数：`conversation_id` 或 `transfer_id`。
- 进程不存在时先启动核心，收到 CoreReady 后再导航；不能在数据库未迁移前查询消息。

## 平台能力表

| 能力 | Windows | Android 13+ |
|---|---|---|
| 后台持续发现 | 托盘运行时支持 | 前台服务尽力支持 |
| 后台自动读剪贴板 | 支持 | 不支持 |
| 后台通知按钮发剪贴板 | 支持 | 支持，需打开可见确认页 |
| 普通路径直接打开 | 支持 | 不支持，使用 SAF URI/FD |
| 最终原子改名 | NTFS 同目录支持 | 依赖通过能力探测的 DocumentsProvider |
| 开机启动 | 当前用户启动项 | V1 不支持 |
| 接收目录 | 普通目录路径 | 持久 Tree URI |

## 示例：Android 接收大文件

1. Rust 收到并保存 Offer，UI 提示用户接受。
2. 没有接收 Tree URI 时启动 SAF 选择器并做读写/重命名探测。
3. 用户接受后，Rust 发出 `CreateReceiveFile`。
4. Kotlin 创建 partial 文档、打开 `rw` FD 并移交 Rust。
5. Rust 直接从 TCP 写 FD，每 1 秒或 64MiB 保存偏移。
6. 数据完成后 Rust 刷新并关闭 FD，发出 `CommitReceiveFile`。
7. Kotlin 重命名成功并回调；Rust 才把 transfer 提交为 completed。

## 异常情况

- Windows 防火墙只允许 TCP、不允许 UDP：已知设备可通过最后 IP 重连，但附近发现降级并显示诊断提示。
- Windows 用户切换：每个用户有独立互斥体、数据库和托盘进程。
- Android 用户选择云盘 DocumentsProvider：能力探测或性能不达标时提示改选本地存储，不承诺局域网满速。
- Android URI 元数据修改时间为 0：记录 0；续传时至少比较 size 和 URI 可读性，无法确认一致时要求重新选择。
- Android 进程在写文件时被杀：下次启动比较 partial 实际大小和数据库 offset，按较小的安全偏移恢复。
- 前台服务通知被用户禁止：继续保持在线和执行活动任务，但普通通知及通知操作不可见；设置页提供重新开启入口。
- 设备锁屏且 OEM 限制网络：视为普通断线，不把任务标记最终失败。

## 实现检查表

- [ ] Windows 第二实例只激活首个实例，不重复打开数据库和端口。
- [ ] Windows partial 与 final 位于同一目录，完成前执行刷新和原子改名。
- [ ] Windows 路径清理覆盖保留名、非法字符、点、空格和路径穿越。
- [ ] Android 没有申请广泛存储权限、无障碍权限或输入法身份。
- [ ] Android SAF URI 取得持久权限并在使用前验证仍有效。
- [ ] Android FD 的打开、移交和关闭责任可以通过测试追踪。
- [ ] Android 后台服务被系统停止后任务能从数据库恢复。
- [ ] Android 后台不读取剪贴板，通知动作先打开有焦点页面。
- [ ] 平台通知动作调用统一 Rust 命令。
- [ ] 网络变化触发 UDP 重绑、立即广播和待发送重连。
- [ ] 所有平台系统错误映射为权威 CoreError。
