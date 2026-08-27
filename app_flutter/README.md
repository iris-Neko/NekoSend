# LAN Chat Flutter 客户端

该目录包含 Windows 10 x64 与 Android 13+ 的 Flutter 界面和平台工程。产品行为以仓库根目录的 `docs/01-product-ux.md` 为准，Rust/Flutter 接口以 `docs/04-data-and-api.md` 为准。

## 当前实现

- Windows 800px 及以上使用可调整侧栏宽度的双栏布局，640-799px 使用列表/详情单栏路由。
- Android 使用会话、附近、设置三个底部导航入口。
- 会话、附近设备、消息、群聊、绑定关系和传输任务均来自 Rust 核心与 SQLite，不使用演示数据。
- 已接入 UDP 发现、TCP 控制/数据连接、离线队列、断点续传、接收策略和错误恢复入口。
- Android 已接入 SAF、通知权限、前台服务和受系统限制的剪贴板行为；Windows 已接入后台运行和剪贴板适配。
- `hook/build.dart` 使用 Flutter Native Assets 编译并打包 `../core_rust`。
- `main.dart` 初始化 `flutter_rust_bridge` 并启动共享核心；设置页显示核心和协议版本。
- Rust 事件按附近设备、会话、传输、邀请和绑定分类刷新；正常运行不再每秒轮询全部快照，事件流中断时自动重订阅。

## 验证

在仓库的 ASCII 目录联接下运行：

```powershell
dart format lib test hook
flutter analyze
flutter test
flutter build windows --debug
flutter build apk --debug --target-platform android-arm64
```

V1 只发布 `arm64-v8a`，不要省略 Android 的 `--target-platform android-arm64`；默认 APK 构建还会请求未配置的 32 位 Rust target。

API 33+ x86_64 模拟器使用 Debug 包：

```powershell
flutter build apk --debug --target-platform android-x64
```

模拟器包只用于开发和三节点联调，不纳入 Release 产物或性能验收。

`test/rust_bridge_test.dart` 会加载实际 Rust 动态库，不是 mock；`test/widget_test.dart` 覆盖 Windows 双栏、窄窗口路由、消息、文件、群聊和剪贴板交互。
