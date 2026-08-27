import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../application/app_controller.dart';
import '../application/models.dart';
import '../platform/platform_bootstrap.dart';
import '../src/rust/api/core.dart';

const _ink = Color(0xFF172126);
const _muted = Color(0xFF66757D);
const _canvas = Color(0xFFF2F5F6);
const _border = Color(0xFFDCE3E6);
const _accent = Color(0xFF087F72);
const _accentSoft = Color(0xFFE1F3EF);
const _warning = Color(0xFFB56A00);

class CoreRuntimeInfo {
  const CoreRuntimeInfo._({
    required this.ready,
    required this.summary,
    this.error,
  });

  factory CoreRuntimeInfo.ready({
    required String version,
    required int protocolVersion,
    required String deviceName,
    required String deviceId,
    required bool discoveryAvailable,
    String? discoveryError,
  }) => CoreRuntimeInfo._(
    ready: discoveryAvailable,
    summary:
        '$deviceName · Rust $version · 协议 $protocolVersion · '
        '${discoveryAvailable ? '发现已启动' : '发现不可用'}',
    error: discoveryError,
  );

  factory CoreRuntimeInfo.failed(String error) =>
      CoreRuntimeInfo._(ready: false, summary: 'Rust 核心不可用', error: error);

  final bool ready;
  final String summary;
  final String? error;
}

class LanChatApp extends StatefulWidget {
  const LanChatApp({
    super.key,
    required this.coreRuntime,
    this.nearbyPeerLoader,
    this.conversationLoader,
    this.messageLoader,
    this.messageDeliveryLoader,
    this.openPrivateConversation,
    this.sendTextCommand,
    this.transferLoader,
    this.sourcePicker,
    this.sendSourceCommand,
    this.decideIncomingOfferCommand,
    this.pauseTransferCommand,
    this.resumeTransferCommand,
    this.cancelTransferCommand,
    this.clearCompletedTransfersCommand,
    this.openReferenceCommand,
    this.pickAndSendSourceCommand,
    this.acceptIncomingOfferCommand,
    this.groupInvitationLoader,
    this.createGroupCommand,
    this.groupInviteDecisionCommand,
    this.groupLoader,
    this.groupUpdateCommand,
    this.leaveGroupCommand,
    this.ownDeviceBindingLoader,
    this.requestOwnDeviceBindingCommand,
    this.decideOwnDeviceBindingCommand,
    this.removeOwnDeviceBindingCommand,
    this.setClipboardModeCommand,
    this.submitClipboardTextCommand,
    this.sendClipboardMessageCommand,
    this.clipboardReader,
    this.clipboardImageReader,
    this.sendClipboardImageCommand,
    this.reselectTransferSourceCommand,
    this.settingsLoader,
    this.settingsUpdater,
    this.deviceNameUpdater,
    this.peerReceivePolicyLoader,
    this.peerReceivePolicyUpdater,
    this.systemNotificationCommand,
    this.markConversationReadCommand,
    this.deleteConversationCommand,
    this.platformRequestPump,
    this.coreEventStreamFactory,
    this.foregroundActions,
    this.initialForegroundAction,
    this.shutdownCoreOnDispose = true,
    this.exitApplicationCommand,
    this.errorPresenter = defaultErrorPresenter,
  });

  final CoreRuntimeInfo coreRuntime;
  final NearbyPeerLoader? nearbyPeerLoader;
  final ConversationLoader? conversationLoader;
  final MessageLoader? messageLoader;
  final MessageDeliveryLoader? messageDeliveryLoader;
  final OpenPrivateConversation? openPrivateConversation;
  final SendTextCommand? sendTextCommand;
  final TransferLoader? transferLoader;
  final SourcePicker? sourcePicker;
  final SendSourceCommand? sendSourceCommand;
  final DecideIncomingOfferCommand? decideIncomingOfferCommand;
  final TransferCommand? pauseTransferCommand;
  final TransferCommand? resumeTransferCommand;
  final TransferCommand? cancelTransferCommand;
  final ClearCompletedTransfersCommand? clearCompletedTransfersCommand;
  final OpenReferenceCommand? openReferenceCommand;
  final PickAndSendSourceCommand? pickAndSendSourceCommand;
  final AcceptIncomingOfferCommand? acceptIncomingOfferCommand;
  final GroupInvitationLoader? groupInvitationLoader;
  final CreateGroupCommand? createGroupCommand;
  final GroupInviteDecisionCommand? groupInviteDecisionCommand;
  final GroupLoader? groupLoader;
  final GroupUpdateCommand? groupUpdateCommand;
  final LeaveGroupCommand? leaveGroupCommand;
  final OwnDeviceBindingLoader? ownDeviceBindingLoader;
  final RequestOwnDeviceBindingCommand? requestOwnDeviceBindingCommand;
  final DecideOwnDeviceBindingCommand? decideOwnDeviceBindingCommand;
  final RemoveOwnDeviceBindingCommand? removeOwnDeviceBindingCommand;
  final SetClipboardModeCommand? setClipboardModeCommand;
  final SubmitClipboardTextCommand? submitClipboardTextCommand;
  final SendClipboardMessageCommand? sendClipboardMessageCommand;
  final ClipboardReader? clipboardReader;
  final ClipboardImageReader? clipboardImageReader;
  final SendClipboardImageCommand? sendClipboardImageCommand;
  final ReselectTransferSourceCommand? reselectTransferSourceCommand;
  final SettingsLoader? settingsLoader;
  final SettingsUpdater? settingsUpdater;
  final DeviceNameUpdater? deviceNameUpdater;
  final PeerReceivePolicyLoader? peerReceivePolicyLoader;
  final PeerReceivePolicyUpdater? peerReceivePolicyUpdater;
  final SystemNotificationCommand? systemNotificationCommand;
  final MarkConversationReadCommand? markConversationReadCommand;
  final DeleteConversationCommand? deleteConversationCommand;
  final PlatformRequestPump? platformRequestPump;
  final CoreEventStreamFactory? coreEventStreamFactory;
  final Stream<String>? foregroundActions;
  final String? initialForegroundAction;
  final bool shutdownCoreOnDispose;
  final Future<void> Function()? exitApplicationCommand;
  final ErrorPresenter errorPresenter;

  @override
  State<LanChatApp> createState() => _LanChatAppState();
}

class _LanChatAppState extends State<LanChatApp> {
  late final AppController controller;
  final navigatorKey = GlobalKey<NavigatorState>();
  StreamSubscription<String>? foregroundActionSubscription;

  @override
  void initState() {
    super.initState();
    controller = AppController(
      nearbyPeerLoader: widget.nearbyPeerLoader,
      conversationLoader: widget.conversationLoader,
      messageLoader: widget.messageLoader,
      messageDeliveryLoader: widget.messageDeliveryLoader,
      openPrivateConversation: widget.openPrivateConversation,
      sendTextCommand: widget.sendTextCommand,
      transferLoader: widget.transferLoader,
      sourcePicker: widget.sourcePicker,
      sendSourceCommand: widget.sendSourceCommand,
      decideIncomingOfferCommand: widget.decideIncomingOfferCommand,
      pauseTransferCommand: widget.pauseTransferCommand,
      resumeTransferCommand: widget.resumeTransferCommand,
      cancelTransferCommand: widget.cancelTransferCommand,
      clearCompletedTransfersCommand: widget.clearCompletedTransfersCommand,
      openReferenceCommand: widget.openReferenceCommand,
      pickAndSendSourceCommand: widget.pickAndSendSourceCommand,
      acceptIncomingOfferCommand: widget.acceptIncomingOfferCommand,
      groupInvitationLoader: widget.groupInvitationLoader,
      createGroupCommand: widget.createGroupCommand,
      groupInviteDecisionCommand: widget.groupInviteDecisionCommand,
      groupLoader: widget.groupLoader,
      groupUpdateCommand: widget.groupUpdateCommand,
      leaveGroupCommand: widget.leaveGroupCommand,
      ownDeviceBindingLoader: widget.ownDeviceBindingLoader,
      requestOwnDeviceBindingCommand: widget.requestOwnDeviceBindingCommand,
      decideOwnDeviceBindingCommand: widget.decideOwnDeviceBindingCommand,
      removeOwnDeviceBindingCommand: widget.removeOwnDeviceBindingCommand,
      setClipboardModeCommand: widget.setClipboardModeCommand,
      submitClipboardTextCommand: widget.submitClipboardTextCommand,
      sendClipboardMessageCommand: widget.sendClipboardMessageCommand,
      clipboardReader: widget.clipboardReader,
      clipboardImageReader: widget.clipboardImageReader,
      sendClipboardImageCommand: widget.sendClipboardImageCommand,
      reselectTransferSourceCommand: widget.reselectTransferSourceCommand,
      settingsLoader: widget.settingsLoader,
      settingsUpdater: widget.settingsUpdater,
      deviceNameUpdater: widget.deviceNameUpdater,
      peerReceivePolicyLoader: widget.peerReceivePolicyLoader,
      peerReceivePolicyUpdater: widget.peerReceivePolicyUpdater,
      systemNotificationCommand: widget.systemNotificationCommand,
      markConversationReadCommand: widget.markConversationReadCommand,
      deleteConversationCommand: widget.deleteConversationCommand,
      coreEventStreamFactory: widget.coreEventStreamFactory,
      errorPresenter: widget.errorPresenter,
    );
    foregroundActionSubscription =
        (widget.foregroundActions ?? PlatformBootstrap.foregroundActions)
            .listen(_handleForegroundAction);
    final initialForegroundAction = widget.initialForegroundAction;
    if (initialForegroundAction != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _handleForegroundAction(initialForegroundAction);
      });
    }
  }

  @override
  void dispose() {
    controller.dispose();
    foregroundActionSubscription?.cancel();
    widget.platformRequestPump?.dispose();
    if (widget.nearbyPeerLoader != null && widget.shutdownCoreOnDispose) {
      shutdownCore();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colorScheme = ColorScheme.fromSeed(
      seedColor: _accent,
      brightness: Brightness.light,
      surface: Colors.white,
    );
    return MaterialApp(
      navigatorKey: navigatorKey,
      title: '猫猫快传',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        useMaterial3: true,
        colorScheme: colorScheme,
        scaffoldBackgroundColor: _canvas,
        fontFamilyFallback: const ['Microsoft YaHei UI', 'Noto Sans SC'],
        dividerColor: _border,
        textTheme: const TextTheme(
          titleLarge: TextStyle(
            color: _ink,
            fontSize: 20,
            fontWeight: FontWeight.w700,
          ),
          titleMedium: TextStyle(
            color: _ink,
            fontSize: 15,
            fontWeight: FontWeight.w600,
          ),
          bodyMedium: TextStyle(color: _ink, fontSize: 14),
          bodySmall: TextStyle(color: _muted, fontSize: 12),
        ),
        inputDecorationTheme: InputDecorationTheme(
          filled: true,
          fillColor: const Color(0xFFF0F3F4),
          isDense: true,
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(6),
            borderSide: BorderSide.none,
          ),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(6),
            borderSide: BorderSide.none,
          ),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(6),
            borderSide: const BorderSide(color: _accent),
          ),
        ),
        tooltipTheme: const TooltipThemeData(
          waitDuration: Duration(milliseconds: 400),
        ),
      ),
      home: LanChatHome(
        controller: controller,
        coreRuntime: widget.coreRuntime,
      ),
    );
  }

  void _handleForegroundAction(String action) {
    if (action.startsWith('openConversation:')) {
      final conversationId = action.substring('openConversation:'.length);
      controller.refreshAll();
      if (controller.conversations.any((item) => item.id == conversationId)) {
        controller.selectConversation(conversationId);
      }
      return;
    }
    if (action == 'trayExitRequested' || action == 'windowExitRequested') {
      _confirmApplicationExit();
      return;
    }
    if (action != 'notificationSendClipboard') return;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      final context = navigatorKey.currentContext;
      if (context == null || !mounted) return;
      final clipboardText = await controller.readClipboardText();
      if (!context.mounted) {
        return;
      }
      if (clipboardText == null || clipboardText.isEmpty) {
        ScaffoldMessenger.of(context)
            .showSnackBar(const SnackBar(content: Text('剪贴板中没有文字')));
        return;
      }
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('发送剪贴板到我的设备'),
          content: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 480, maxHeight: 280),
            child: SingleChildScrollView(child: SelectableText(clipboardText)),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('取消'),
            ),
            FilledButton.icon(
              onPressed: () => Navigator.pop(context, true),
              icon: const Icon(LucideIcons.send, size: 17),
              label: const Text('发送'),
            ),
          ],
        ),
      );
      if (confirmed != true || !context.mounted) return;
      final sent = controller.sendClipboardTextToOwnDevices(clipboardText);
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(sent == 0 ? '没有已绑定的“我的设备”' : '已发送到 $sent 台设备')),
      );
    });
  }

  Future<void> _confirmApplicationExit() async {
    final context = navigatorKey.currentContext;
    if (context == null || !mounted) return;
    final hasActiveTransfers = controller.transfers.any(
      (transfer) => const {
        'queued',
        'offered',
        'accepted',
        'transferring',
        'verifying',
      }.contains(transfer.state),
    );
    if (hasActiveTransfers) {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('退出猫猫快传'),
          content: const Text('仍有未完成的传输。退出后任务会保留，并在下次启动时恢复。'),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('继续运行'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('退出'),
            ),
          ],
        ),
      );
      if (confirmed != true) return;
    }
    final exitCommand = widget.exitApplicationCommand;
    if (exitCommand != null) {
      await exitCommand();
    } else {
      shutdownCore();
      await PlatformBootstrap.exitApplication();
    }
  }
}

class LanChatHome extends StatelessWidget {
  const LanChatHome({
    super.key,
    required this.controller,
    this.platform,
    this.coreRuntime = const CoreRuntimeInfo._(
      ready: true,
      summary: 'Rust 核心测试模式',
    ),
  });

  final AppController controller;
  final TargetPlatform? platform;
  final CoreRuntimeInfo coreRuntime;

  @override
  Widget build(BuildContext context) {
    final effectivePlatform = platform ?? defaultTargetPlatform;
    final isAndroid = effectivePlatform == TargetPlatform.android;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) => isAndroid
          ? _MobileShell(controller: controller, coreRuntime: coreRuntime)
          : _DesktopShell(controller: controller, coreRuntime: coreRuntime),
    );
  }
}

class _DesktopShell extends StatefulWidget {
  const _DesktopShell({required this.controller, required this.coreRuntime});

  final AppController controller;
  final CoreRuntimeInfo coreRuntime;

  @override
  State<_DesktopShell> createState() => _DesktopShellState();
}

class _DesktopShellState extends State<_DesktopShell> {
  double sidebarWidth = 300;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: LayoutBuilder(
        builder: (context, constraints) {
          if (constraints.maxWidth < 800) {
            return widget.controller.compactDetailVisible
                ? _MainPane(controller: widget.controller, showBackButton: true)
                : _Sidebar(
                    controller: widget.controller,
                    coreRuntime: widget.coreRuntime,
                  );
          }
          return Row(
            children: [
              SizedBox(
                key: const ValueKey('desktop-sidebar'),
                width: sidebarWidth,
                child: _Sidebar(
                  controller: widget.controller,
                  coreRuntime: widget.coreRuntime,
                ),
              ),
              MouseRegion(
                cursor: SystemMouseCursors.resizeColumn,
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onHorizontalDragUpdate: (details) {
                    setState(() {
                      sidebarWidth = (sidebarWidth + details.delta.dx).clamp(
                        260,
                        360,
                      );
                    });
                  },
                  child: const SizedBox(
                    width: 4,
                    child: ColoredBox(color: _border),
                  ),
                ),
              ),
              Expanded(
                key: const ValueKey('desktop-main-pane'),
                child: _MainPane(controller: widget.controller),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _Sidebar extends StatelessWidget {
  const _Sidebar({required this.controller, required this.coreRuntime});

  final AppController controller;
  final CoreRuntimeInfo coreRuntime;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: const Color(0xFFFAFCFC),
      child: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 14, 10, 10),
              child: Row(
                children: [
                  const Expanded(
                    child: Text(
                      '猫猫快传',
                      style: TextStyle(
                        color: _ink,
                        fontSize: 20,
                        fontWeight: FontWeight.w800,
                      ),
                    ),
                  ),
                  IconButton(
                    tooltip: '设置',
                    onPressed: () => _showSettings(context),
                    icon: const Icon(LucideIcons.settings, size: 20),
                  ),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: SizedBox(
                width: double.infinity,
                child: SegmentedButton<SidebarSection>(
                  showSelectedIcon: false,
                  segments: const [
                    ButtonSegment(
                      value: SidebarSection.conversations,
                      icon: Icon(LucideIcons.messageCircle, size: 17),
                      label: Text('会话'),
                    ),
                    ButtonSegment(
                      value: SidebarSection.nearby,
                      icon: Icon(LucideIcons.radar, size: 17),
                      label: Text('附近'),
                    ),
                  ],
                  selected: {controller.section},
                  onSelectionChanged: (selection) {
                    controller.setSection(selection.first);
                  },
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 12, 12, 8),
              child: TextField(
                key: const ValueKey('sidebar-search'),
                onChanged: controller.setSearchQuery,
                decoration: const InputDecoration(
                  hintText: '搜索',
                  prefixIcon: Icon(LucideIcons.search, size: 18),
                ),
              ),
            ),
            Expanded(
              child: controller.section == SidebarSection.conversations
                  ? _ConversationList(controller: controller)
                  : _NearbyList(controller: controller),
            ),
            _TransferEntry(controller: controller),
          ],
        ),
      ),
    );
  }

  void _showSettings(BuildContext context) {
    showDialog<void>(
      context: context,
      builder: (context) =>
          _SettingsDialog(coreRuntime: coreRuntime, controller: controller),
    );
  }
}

class _ConversationList extends StatelessWidget {
  const _ConversationList({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    final conversations = controller.filteredConversations;
    if (conversations.isEmpty) {
      return const Center(child: Text('没有匹配的会话'));
    }
    return ListView.builder(
      padding: const EdgeInsets.symmetric(horizontal: 8),
      itemCount: conversations.length,
      itemBuilder: (context, index) {
        final conversation = conversations[index];
        final selected =
            controller.mainContent == MainContent.conversation &&
            controller.selectedConversationId == conversation.id;
        return _ConversationTile(
          conversation: conversation,
          selected: selected,
          onTap: () => controller.selectConversation(conversation.id),
        );
      },
    );
  }
}

class _ConversationTile extends StatelessWidget {
  const _ConversationTile({
    required this.conversation,
    required this.selected,
    required this.onTap,
  });

  final ConversationSummary conversation;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Material(
        color: selected ? _accentSoft : Colors.transparent,
        borderRadius: BorderRadius.circular(6),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 10),
            child: Row(
              children: [
                _Avatar(
                  label: conversation.title,
                  group: conversation.isGroup,
                  online: conversation.online,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: Text(
                              conversation.title,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                color: _ink,
                                fontSize: 14,
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                          ),
                          Text(
                            conversation.timeLabel,
                            style: const TextStyle(color: _muted, fontSize: 11),
                          ),
                        ],
                      ),
                      const SizedBox(height: 4),
                      Row(
                        children: [
                          Expanded(
                            child: Text(
                              conversation.preview,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                color: _muted,
                                fontSize: 12,
                              ),
                            ),
                          ),
                          if (conversation.transferProgress
                              case final progress?)
                            Padding(
                              padding: const EdgeInsets.only(left: 8),
                              child: SizedBox(
                                width: 16,
                                height: 16,
                                child: CircularProgressIndicator(
                                  value: progress,
                                  strokeWidth: 2,
                                  color: _accent,
                                  backgroundColor: _border,
                                ),
                              ),
                            )
                          else if (conversation.unreadCount > 0)
                            Container(
                              constraints: const BoxConstraints(minWidth: 18),
                              padding: const EdgeInsets.symmetric(
                                horizontal: 5,
                                vertical: 2,
                              ),
                              decoration: BoxDecoration(
                                color: _accent,
                                borderRadius: BorderRadius.circular(8),
                              ),
                              child: Text(
                                '${conversation.unreadCount}',
                                textAlign: TextAlign.center,
                                style: const TextStyle(
                                  color: Colors.white,
                                  fontSize: 10,
                                ),
                              ),
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _NearbyList extends StatelessWidget {
  const _NearbyList({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    final devices = controller.filteredNearbyDevices;
    return ListView(
      padding: const EdgeInsets.fromLTRB(8, 0, 8, 12),
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
          child: Row(
            children: [
              Expanded(
                child: FilledButton.icon(
                  onPressed: devices.isEmpty
                      ? null
                      : () => _showCreateGroup(context),
                  icon: const Icon(LucideIcons.users, size: 17),
                  label: const Text('创建群聊'),
                ),
              ),
              const SizedBox(width: 6),
              IconButton.outlined(
                tooltip: '重新扫描',
                onPressed: controller.refreshNearby,
                icon: const Icon(LucideIcons.refreshCw, size: 18),
              ),
            ],
          ),
        ),
        if (controller.groupInvitations.isNotEmpty) ...[
          const Padding(
            padding: EdgeInsets.fromLTRB(8, 12, 8, 6),
            child: Text('群聊邀请', style: TextStyle(color: _muted, fontSize: 12)),
          ),
          for (final invitation in controller.groupInvitations)
            _GroupInvitationTile(
              invitation: invitation,
              onAccept: () => controller.decideGroupInvite(invitation.id, true),
              onReject: () =>
                  controller.decideGroupInvite(invitation.id, false),
            ),
        ],
        if (controller.pendingOwnDeviceBindings.isNotEmpty) ...[
          const Padding(
            padding: EdgeInsets.fromLTRB(8, 12, 8, 6),
            child: Text(
              '我的设备请求',
              style: TextStyle(color: _muted, fontSize: 12),
            ),
          ),
          for (final binding in controller.pendingOwnDeviceBindings)
            ListTile(
              key: ValueKey('binding-request-${binding.id}'),
              leading: const Icon(LucideIcons.link),
              title: Text(binding.peerName),
              subtitle: const Text('希望与本机绑定为“我的设备”'),
              trailing: Wrap(
                spacing: 4,
                children: [
                  IconButton(
                    tooltip: '拒绝',
                    onPressed: () =>
                        controller.decideOwnDeviceBinding(binding.id, false),
                    icon: const Icon(LucideIcons.x, size: 18),
                  ),
                  IconButton.filled(
                    tooltip: '接受',
                    onPressed: () =>
                        controller.decideOwnDeviceBinding(binding.id, true),
                    icon: const Icon(LucideIcons.check, size: 18),
                  ),
                ],
              ),
            ),
        ],
        const Padding(
          padding: EdgeInsets.fromLTRB(8, 12, 8, 6),
          child: Text('在线设备', style: TextStyle(color: _muted, fontSize: 12)),
        ),
        if (devices.isEmpty)
          const Padding(
            padding: EdgeInsets.fromLTRB(8, 28, 8, 8),
            child: Center(child: Text('暂未发现在线设备')),
          ),
        for (final device in devices)
          ListTile(
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(6),
            ),
            leading: _DeviceIcon(platform: device.platform),
            title: Text(
              device.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            subtitle: Text(
              [
                if (device.relationLabel != null) device.relationLabel!,
                if (device.address != null) device.address!,
              ].join(' · '),
            ),
            trailing: const SizedBox(
              width: 8,
              height: 8,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: Color(0xFF19A974),
                  shape: BoxShape.circle,
                ),
              ),
            ),
            onTap: () => controller.selectNearbyDevice(device),
          ),
      ],
    );
  }

  Future<void> _showCreateGroup(BuildContext context) async {
    final created = await showDialog<bool>(
      context: context,
      builder: (context) => _CreateGroupDialog(
        devices: controller.nearbyDevices,
        onCreate: controller.createGroup,
      ),
    );
    if (!context.mounted || created == true || controller.lastError == null) {
      return;
    }
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text('无法创建群聊：${controller.lastError}')));
  }
}

class _GroupInvitationTile extends StatelessWidget {
  const _GroupInvitationTile({
    required this.invitation,
    required this.onAccept,
    required this.onReject,
  });

  final GroupInvitationView invitation;
  final VoidCallback onAccept;
  final VoidCallback onReject;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: Colors.white,
          border: Border.all(color: _border),
          borderRadius: BorderRadius.circular(6),
        ),
        child: Padding(
          padding: const EdgeInsets.all(10),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                invitation.groupName,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontWeight: FontWeight.w700),
              ),
              const SizedBox(height: 3),
              Text(
                '${invitation.inviterName} 邀请你加入 · ${invitation.memberDeviceIds.length} 名成员',
                style: const TextStyle(color: _muted, fontSize: 11),
              ),
              const SizedBox(height: 8),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  TextButton(onPressed: onReject, child: const Text('拒绝')),
                  const SizedBox(width: 6),
                  FilledButton(onPressed: onAccept, child: const Text('加入群聊')),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _CreateGroupDialog extends StatefulWidget {
  const _CreateGroupDialog({required this.devices, required this.onCreate});

  final List<NearbyDevice> devices;
  final bool Function(String name, List<String> memberDeviceIds) onCreate;

  @override
  State<_CreateGroupDialog> createState() => _CreateGroupDialogState();
}

class _CreateGroupDialogState extends State<_CreateGroupDialog> {
  final nameController = TextEditingController();
  final selected = <String>{};

  @override
  void initState() {
    super.initState();
    selected.addAll(widget.devices.map((device) => device.id).take(31));
    nameController.text = widget.devices
        .where((device) => selected.contains(device.id))
        .map((device) => device.name)
        .take(3)
        .join('、');
  }

  @override
  void dispose() {
    nameController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('创建群聊'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: nameController,
              maxLength: 50,
              onChanged: (_) => setState(() {}),
              decoration: const InputDecoration(labelText: '群聊名称'),
            ),
            const SizedBox(height: 12),
            Text(
              '选择成员 · ${selected.length}/31',
              style: const TextStyle(color: _muted),
            ),
            const SizedBox(height: 6),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 300),
              child: ListView.builder(
                shrinkWrap: true,
                itemCount: widget.devices.length,
                itemBuilder: (context, index) {
                  final device = widget.devices[index];
                  return CheckboxListTile(
                    dense: true,
                    contentPadding: EdgeInsets.zero,
                    value: selected.contains(device.id),
                    title: Text(
                      device.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    subtitle: device.address == null
                        ? null
                        : Text(device.address!),
                    onChanged: (checked) {
                      setState(() {
                        if (checked == true && selected.length < 31) {
                          selected.add(device.id);
                        } else if (checked != true) {
                          selected.remove(device.id);
                        }
                      });
                    },
                  );
                },
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: selected.isEmpty || nameController.text.trim().isEmpty
              ? null
              : () {
                  final created = widget.onCreate(
                    nameController.text.trim(),
                    selected.toList(growable: false),
                  );
                  if (created) Navigator.pop(context, true);
                },
          child: const Text('创建'),
        ),
      ],
    );
  }
}

class _TransferEntry extends StatelessWidget {
  const _TransferEntry({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    final transfers = controller.transfers;
    final totalBytes = transfers.fold<BigInt>(
      BigInt.zero,
      (total, transfer) => total + transfer.totalSize,
    );
    final persistedBytes = transfers.fold<BigInt>(
      BigInt.zero,
      (total, transfer) => total + transfer.persistedBytes,
    );
    final progress = totalBytes == BigInt.zero
        ? (transfers.isNotEmpty &&
                  transfers.every((transfer) => transfer.state == 'completed')
              ? 1.0
              : 0.0)
        : (persistedBytes.toDouble() / totalBytes.toDouble()).clamp(0.0, 1.0);
    return Material(
      color: Colors.white,
      child: InkWell(
        onTap: controller.showTransfers,
        child: Container(
          height: 58,
          padding: const EdgeInsets.symmetric(horizontal: 16),
          decoration: const BoxDecoration(
            border: Border(top: BorderSide(color: _border)),
          ),
          child: Row(
            children: [
              const Icon(LucideIcons.arrowDownToLine, size: 19, color: _accent),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text(
                      '当前传输',
                      style: TextStyle(fontWeight: FontWeight.w600),
                    ),
                    const SizedBox(height: 2),
                    Text(
                      transfers.isEmpty
                          ? '暂无任务'
                          : '${transfers.length} 个任务 · '
                                '${(progress * 100).round()}%',
                      style: const TextStyle(color: _muted, fontSize: 11),
                    ),
                  ],
                ),
              ),
              SizedBox(
                width: 34,
                child: LinearProgressIndicator(
                  value: progress,
                  minHeight: 3,
                  color: _accent,
                  backgroundColor: _border,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _MainPane extends StatelessWidget {
  const _MainPane({required this.controller, this.showBackButton = false});

  final AppController controller;
  final bool showBackButton;

  @override
  Widget build(BuildContext context) {
    return controller.mainContent == MainContent.transfers
        ? _TransfersPane(
            controller: controller,
            onClose: controller.closeTransfers,
            onBack: showBackButton ? controller.showCompactList : null,
          )
        : controller.hasSelectedConversation
        ? _ConversationPane(
            controller: controller,
            showBackButton: showBackButton,
          )
        : _EmptyConversationPane(
            onBack: showBackButton ? controller.showCompactList : null,
          );
  }
}

class _EmptyConversationPane extends StatelessWidget {
  const _EmptyConversationPane({this.onBack});

  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: _canvas,
      child: SafeArea(
        child: Column(
          children: [
            if (onBack != null)
              Align(
                alignment: Alignment.centerLeft,
                child: IconButton(
                  tooltip: '返回',
                  onPressed: onBack,
                  icon: const Icon(LucideIcons.arrowLeft),
                ),
              ),
            const Expanded(
              child: Center(
                child: Text('从附近设备开始聊天', style: TextStyle(color: _muted)),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ConversationPane extends StatefulWidget {
  const _ConversationPane({
    required this.controller,
    required this.showBackButton,
  });

  final AppController controller;
  final bool showBackButton;

  @override
  State<_ConversationPane> createState() => _ConversationPaneState();
}

class _ConversationPaneState extends State<_ConversationPane> {
  final textController = TextEditingController();
  final messageScrollController = ScrollController();
  final focusedMessageKey = GlobalKey();
  late String _conversationId;
  late int _messageCount;
  String? _lastMessageId;
  String? _focusedMessageId;

  @override
  void initState() {
    super.initState();
    _captureMessageState();
    _scheduleScrollToBottom(animate: false);
    _captureAndScheduleMessageFocus();
  }

  @override
  void didUpdateWidget(covariant _ConversationPane oldWidget) {
    super.didUpdateWidget(oldWidget);
    final messages = widget.controller.selectedMessages;
    final nextConversationId = widget.controller.selectedConversationId;
    final nextLastMessageId = messages.lastOrNull?.id;
    final switchedConversation = nextConversationId != _conversationId;
    final appendedMessage =
        messages.length > _messageCount ||
        (messages.length == _messageCount &&
            nextLastMessageId != null &&
            nextLastMessageId != _lastMessageId);
    final wasNearBottom =
        !messageScrollController.hasClients ||
        messageScrollController.position.pixels -
                messageScrollController.position.minScrollExtent <=
            80;
    final sentByLocalDevice = appendedMessage && messages.last.outgoing;

    _captureMessageState();
    _captureAndScheduleMessageFocus();
    if (switchedConversation ||
        (appendedMessage && (wasNearBottom || sentByLocalDevice))) {
      _scheduleScrollToBottom(animate: !switchedConversation);
    }
  }

  @override
  void dispose() {
    textController.dispose();
    messageScrollController.dispose();
    super.dispose();
  }

  void _captureMessageState() {
    final messages = widget.controller.selectedMessages;
    _conversationId = widget.controller.selectedConversationId;
    _messageCount = messages.length;
    _lastMessageId = messages.lastOrNull?.id;
  }

  void _scheduleScrollToBottom({required bool animate}) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !messageScrollController.hasClients) return;
      final target = messageScrollController.position.minScrollExtent;
      if (animate) {
        messageScrollController.animateTo(
          target,
          duration: const Duration(milliseconds: 160),
          curve: Curves.easeOut,
        );
      } else {
        messageScrollController.jumpTo(target);
      }
    });
  }

  void _captureAndScheduleMessageFocus() {
    final requested = widget.controller.focusedMessageId;
    if (requested == null || requested == _focusedMessageId) return;
    if (!widget.controller.selectedMessages.any(
      (item) => item.id == requested,
    )) {
      return;
    }
    _focusedMessageId = requested;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final target = focusedMessageKey.currentContext;
      if (target != null) {
        Scrollable.ensureVisible(
          target,
          alignment: 0.45,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOut,
        );
      }
      widget.controller.clearFocusedMessage(requested);
    });
  }

  @override
  Widget build(BuildContext context) {
    final conversation = widget.controller.selectedConversation;
    return ColoredBox(
      color: _canvas,
      child: SafeArea(
        child: Column(
          children: [
            _ChatHeader(
              conversation: conversation,
              onMore: () => _showConversationActions(context, conversation),
              onBack: widget.showBackButton
                  ? widget.controller.showCompactList
                  : null,
            ),
            Expanded(
              child: ListView(
                key: const ValueKey('conversation-message-list'),
                controller: messageScrollController,
                reverse: true,
                padding: const EdgeInsets.symmetric(
                  horizontal: 24,
                  vertical: 20,
                ),
                children: [
                  for (final message
                      in widget.controller.selectedMessages.reversed)
                    _MessageBubble(
                      key: message.id == _focusedMessageId
                          ? focusedMessageKey
                          : ValueKey('message-${message.id}'),
                      message: message,
                      onOpen: message.localFileRef == null
                          ? null
                          : () => _openReference(
                              context,
                              message.localFileRef!,
                              showInFolder: false,
                            ),
                      onShowLocation: message.localFileRef == null
                          ? null
                          : () => _openReference(
                              context,
                              message.localFileRef!,
                              showInFolder: true,
                            ),
                      onStatusTap: conversation.isGroup && message.outgoing
                          ? () => _showDeliveryDetails(context, message)
                          : null,
                    ),
                  const Center(
                    child: Padding(
                      padding: EdgeInsets.only(bottom: 18),
                      child: Text(
                        '今天',
                        style: TextStyle(color: _muted, fontSize: 11),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            _Composer(
              textController: textController,
              appController: widget.controller,
              onSend: _sendText,
            ),
          ],
        ),
      ),
    );
  }

  void _sendText() {
    final text = textController.text;
    if (widget.controller.sendText(text)) {
      textController.clear();
      return;
    }
    final error = widget.controller.lastError;
    if (error != null && mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('发送失败：$error')));
    }
  }

  Future<void> _openReference(
    BuildContext context,
    String reference, {
    required bool showInFolder,
  }) async {
    try {
      await PlatformBootstrap.openReference(
        reference,
        showInFolder: showInFolder,
      );
    } catch (error) {
      if (!context.mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(showInFolder ? '无法显示位置：$error' : '无法打开：$error')),
      );
    }
  }

  Future<void> _showDeliveryDetails(
    BuildContext context,
    ChatMessage message,
  ) async {
    final deliveries = widget.controller.loadMessageDeliveries(message.id);
    if (deliveries == null) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('无法读取送达详情：${widget.controller.lastError}')),
        );
      }
      return;
    }
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('送达详情'),
        content: SizedBox(
          width: 420,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                '已送达 ${deliveries.where((item) => item.delivered).length}/${deliveries.length}',
              ),
              const SizedBox(height: 12),
              if (deliveries.isEmpty)
                const Padding(
                  padding: EdgeInsets.symmetric(vertical: 16),
                  child: Text('没有投递目标'),
                )
              else
                Flexible(
                  child: ListView.builder(
                    shrinkWrap: true,
                    itemCount: deliveries.length,
                    itemBuilder: (context, index) {
                      final delivery = deliveries[index];
                      return ListTile(
                        contentPadding: EdgeInsets.zero,
                        leading: Icon(
                          _deliveryIcon(delivery),
                          color: delivery.delivered
                              ? const Color(0xFF16845D)
                              : _deliveryFailed(delivery.state)
                              ? Colors.red
                              : _warning,
                        ),
                        title: Text(delivery.recipientName),
                        subtitle: Text(
                          delivery.failureReason == null
                              ? _deliveryStateLabel(delivery.state)
                              : '${_deliveryStateLabel(delivery.state)} · ${delivery.failureReason}',
                        ),
                      );
                    },
                  ),
                ),
            ],
          ),
        ),
        actions: [
          FilledButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('完成'),
          ),
        ],
      ),
    );
  }

  Future<void> _showGroupDetails(
    BuildContext context,
    ConversationSummary conversation,
  ) async {
    final group = widget.controller.loadGroup(conversation.id);
    if (group == null) {
      if (widget.controller.lastError != null && context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('无法读取群资料：${widget.controller.lastError}')),
        );
      }
      return;
    }
    await showDialog<void>(
      context: context,
      builder: (context) => _GroupDetailsDialog(
        controller: widget.controller,
        initialGroup: group,
      ),
    );
  }

  Future<void> _showConversationActions(
    BuildContext context,
    ConversationSummary conversation,
  ) async {
    if (!conversation.isGroup) {
      await _showPrivateActions(context, conversation);
      return;
    }
    await showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              key: const ValueKey('group-details'),
              leading: const Icon(LucideIcons.users),
              title: const Text('群资料与成员'),
              onTap: () {
                Navigator.pop(sheetContext);
                _showGroupDetails(context, conversation);
              },
            ),
            const Divider(height: 1),
            _deleteConversationTile(context, sheetContext, conversation),
          ],
        ),
      ),
    );
  }

  Future<void> _showPrivateActions(
    BuildContext context,
    ConversationSummary conversation,
  ) async {
    final peerDeviceId = conversation.peerDeviceId;
    if (peerDeviceId == null) return;
    final binding = widget.controller.bindingForPeer(peerDeviceId);
    await showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (binding == null ||
                binding.state == 'removed' ||
                binding.state == 'rejected')
              ListTile(
                key: const ValueKey('request-own-device'),
                leading: const Icon(LucideIcons.link),
                title: const Text('设为我的设备'),
                onTap: () {
                  Navigator.pop(sheetContext);
                  _runPrivateAction(
                    context,
                    widget.controller.requestOwnDeviceBinding(peerDeviceId),
                    '绑定请求已发送',
                  );
                },
              ),
            if (binding?.state == 'pending_outbound')
              const ListTile(
                leading: Icon(LucideIcons.clock),
                title: Text('等待对方确认'),
              ),
            if (binding?.active == true) ...[
              ListTile(
                key: const ValueKey('clipboard-sync-settings'),
                leading: const Icon(LucideIcons.clipboard),
                title: const Text('剪贴板同步设置'),
                subtitle: Text(_clipboardModeLabel(binding!.clipboardMode)),
                onTap: () {
                  Navigator.pop(sheetContext);
                  _showClipboardModeDialog(context, binding);
                },
              ),
              ListTile(
                key: const ValueKey('remove-own-device'),
                leading: const Icon(LucideIcons.unlink),
                title: const Text('解除绑定'),
                onTap: () {
                  Navigator.pop(sheetContext);
                  _confirmRemoveBinding(context, binding);
                },
              ),
            ],
            const Divider(height: 1),
            _deleteConversationTile(context, sheetContext, conversation),
          ],
        ),
      ),
    );
  }

  Widget _deleteConversationTile(
    BuildContext pageContext,
    BuildContext sheetContext,
    ConversationSummary conversation,
  ) {
    return ListTile(
      key: const ValueKey('delete-conversation'),
      leading: const Icon(LucideIcons.trash2, color: Colors.red),
      title: const Text('删除会话', style: TextStyle(color: Colors.red)),
      onTap: () {
        Navigator.pop(sheetContext);
        _confirmDeleteConversation(pageContext, conversation);
      },
    );
  }

  Future<void> _confirmDeleteConversation(
    BuildContext context,
    ConversationSummary conversation,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: Text('删除“${conversation.title}”？'),
        content: const Text(
          '只会删除本机聊天记录，并取消这个会话中尚未完成的文件任务。'
          '已经保存到接收目录的文件不会删除。对方再次发来消息时，会话会重新出现。',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext, false),
            child: const Text('取消'),
          ),
          FilledButton(
            key: const ValueKey('confirm-delete-conversation'),
            onPressed: () => Navigator.pop(dialogContext, true),
            child: const Text('删除会话'),
          ),
        ],
      ),
    );
    if (confirmed != true || !context.mounted) {
      return;
    }
    final messenger = ScaffoldMessenger.of(context);
    final deleted = widget.controller.deleteSelectedConversation();
    messenger.showSnackBar(
      SnackBar(
        content: Text(
          deleted
              ? '已删除本机会话记录'
              : '删除失败：${widget.controller.lastError ?? '无法删除当前会话'}',
        ),
      ),
    );
  }

  void _runPrivateAction(BuildContext context, bool ok, String success) {
    final message = ok
        ? success
        : '操作失败：${widget.controller.lastError ?? '未知错误'}';
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(message)));
  }

  Future<void> _showClipboardModeDialog(
    BuildContext context,
    OwnDeviceBindingView binding,
  ) async {
    final mode = await showDialog<String>(
      context: context,
      builder: (context) => SimpleDialog(
        title: Text('${binding.peerName} · 剪贴板同步'),
        children: [
          for (final option in const [
            ('off', '关闭'),
            ('send_only', '仅发送'),
            ('receive_only', '仅接收'),
            ('bidirectional', '双向'),
          ])
            ListTile(
              leading: Icon(
                option.$1 == binding.clipboardMode
                    ? LucideIcons.circleCheck
                    : LucideIcons.circle,
              ),
              title: Text(option.$2),
              onTap: () => Navigator.pop(context, option.$1),
            ),
        ],
      ),
    );
    if (mode == null || !context.mounted) return;
    _runPrivateAction(
      context,
      widget.controller.setClipboardMode(binding.peerDeviceId, mode),
      '剪贴板同步设置已保存',
    );
  }

  Future<void> _confirmRemoveBinding(
    BuildContext context,
    OwnDeviceBindingView binding,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('解除绑定'),
        content: Text('解除与 ${binding.peerName} 的绑定？剪贴板自动同步也会关闭。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('解除'),
          ),
        ],
      ),
    );
    if (confirmed != true || !context.mounted) return;
    _runPrivateAction(
      context,
      widget.controller.removeOwnDeviceBinding(binding.peerDeviceId),
      '绑定已解除',
    );
  }
}

String _clipboardModeLabel(String mode) => switch (mode) {
  'send_only' => '仅发送',
  'receive_only' => '仅接收',
  'bidirectional' => '双向',
  _ => '关闭',
};

class _ChatHeader extends StatelessWidget {
  const _ChatHeader({required this.conversation, this.onBack, this.onMore});

  final ConversationSummary conversation;
  final VoidCallback? onBack;
  final VoidCallback? onMore;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 68,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      decoration: const BoxDecoration(
        color: Colors.white,
        border: Border(bottom: BorderSide(color: _border)),
      ),
      child: Row(
        children: [
          if (onBack != null)
            IconButton(
              tooltip: '返回',
              onPressed: onBack,
              icon: const Icon(LucideIcons.arrowLeft, size: 20),
            ),
          _Avatar(
            label: conversation.title,
            group: conversation.isGroup,
            online: conversation.online,
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  conversation.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    fontSize: 15,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                const SizedBox(height: 2),
                Text(
                  conversation.isGroup
                      ? '${conversation.onlineCount}/${conversation.memberCount} 台在线'
                      : conversation.online
                      ? '在线'
                      : '离线，消息将在上线后发送',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: conversation.online
                        ? const Color(0xFF16845B)
                        : _muted,
                    fontSize: 11,
                  ),
                ),
              ],
            ),
          ),
          IconButton(
            tooltip: '更多',
            onPressed: onMore,
            icon: const Icon(LucideIcons.ellipsis, size: 21),
          ),
        ],
      ),
    );
  }
}

class _GroupDetailsDialog extends StatefulWidget {
  const _GroupDetailsDialog({
    required this.controller,
    required this.initialGroup,
  });

  final AppController controller;
  final GroupView initialGroup;

  @override
  State<_GroupDetailsDialog> createState() => _GroupDetailsDialogState();
}

class _GroupDetailsDialogState extends State<_GroupDetailsDialog> {
  late GroupView group = widget.initialGroup;

  @override
  Widget build(BuildContext context) {
    final activeMembers = group.members
        .where(
          (member) => const {'invited', 'joined'}.contains(member.membership),
        )
        .toList(growable: false);
    return AlertDialog(
      title: Row(
        children: [
          Expanded(
            child: Text(
              group.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (group.isOwner && group.state == 'active')
            IconButton(
              tooltip: '修改群名',
              onPressed: _rename,
              icon: const Icon(LucideIcons.pencil, size: 18),
            ),
        ],
      ),
      content: SizedBox(
        width: 520,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SelectableText(
              group.id,
              style: const TextStyle(color: _muted, fontSize: 11),
            ),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: Text(
                    '${activeMembers.where((member) => member.online).length}/${activeMembers.length} 台在线 · 版本 ${group.revision}',
                    style: const TextStyle(color: _muted),
                  ),
                ),
                if (group.isOwner && group.state == 'active')
                  FilledButton.tonalIcon(
                    onPressed: _invite,
                    icon: const Icon(LucideIcons.userPlus, size: 17),
                    label: const Text('邀请成员'),
                  ),
              ],
            ),
            const SizedBox(height: 8),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 330),
              child: ListView.builder(
                shrinkWrap: true,
                itemCount: activeMembers.length,
                itemBuilder: (context, index) {
                  final member = activeMembers[index];
                  final owner = member.role == 'owner';
                  return ListTile(
                    contentPadding: EdgeInsets.zero,
                    leading: _MemberAvatar(member: member),
                    title: Text(_memberName(member.deviceId)),
                    subtitle: Text(
                      owner
                          ? '群主${member.online ? ' · 在线' : ' · 离线'}'
                          : member.membership == 'invited'
                          ? '等待确认'
                          : member.online
                          ? '在线'
                          : '离线',
                    ),
                    trailing: group.isOwner && !owner && group.state == 'active'
                        ? PopupMenuButton<String>(
                            tooltip: '成员操作',
                            onSelected: (action) =>
                                _memberAction(action, member),
                            itemBuilder: (context) => [
                              if (member.membership == 'joined' &&
                                  member.online)
                                const PopupMenuItem(
                                  value: 'transfer',
                                  child: Text('转让群主'),
                                ),
                              const PopupMenuItem(
                                value: 'remove',
                                child: Text('移除成员'),
                              ),
                            ],
                          )
                        : null,
                  );
                },
              ),
            ),
          ],
        ),
      ),
      actions: [
        if (group.state == 'active')
          TextButton(
            onPressed: group.isOwner ? _disband : _leave,
            child: Text(group.isOwner ? '解散群聊' : '退出群聊'),
          ),
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('关闭'),
        ),
      ],
    );
  }

  String _memberName(String deviceId) {
    for (final device in widget.controller.nearbyDevices) {
      if (device.id == deviceId) return device.name;
    }
    if (deviceId == group.ownerDeviceId && group.isOwner) return '本机';
    return '${deviceId.substring(0, 10)}…';
  }

  Future<void> _rename() async {
    final controller = TextEditingController(text: group.name);
    final name = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('修改群名'),
        content: TextField(
          autofocus: true,
          controller: controller,
          maxLength: 50,
          decoration: const InputDecoration(labelText: '群聊名称'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text.trim()),
            child: const Text('保存'),
          ),
        ],
      ),
    );
    controller.dispose();
    if (name == null || name.isEmpty || name == group.name) return;
    _apply(() => widget.controller.updateGroup(group, name: name));
  }

  Future<void> _invite() async {
    final existing = group.members
        .where(
          (member) => const {'invited', 'joined'}.contains(member.membership),
        )
        .map((member) => member.deviceId)
        .toSet();
    final candidates = widget.controller.nearbyDevices
        .where((device) => !existing.contains(device.id))
        .toList(growable: false);
    final selected = await showDialog<List<String>>(
      context: context,
      builder: (context) => _PickGroupMembersDialog(devices: candidates),
    );
    if (selected == null || selected.isEmpty) return;
    _apply(() => widget.controller.updateGroup(group, addMemberIds: selected));
  }

  Future<void> _memberAction(String action, GroupMemberView member) async {
    final label = action == 'transfer' ? '将群主转让给该设备？' : '将该设备移出群聊？';
    if (!await _confirm(label)) return;
    _apply(
      () => action == 'transfer'
          ? widget.controller.updateGroup(
              group,
              transferOwnerTo: member.deviceId,
            )
          : widget.controller.updateGroup(
              group,
              removeMemberIds: [member.deviceId],
            ),
    );
  }

  Future<void> _disband() async {
    if (!await _confirm('解散后所有成员只能查看历史消息，确定解散？')) return;
    if (_apply(() => widget.controller.updateGroup(group, disband: true)) &&
        mounted) {
      Navigator.pop(context);
    }
  }

  Future<void> _leave() async {
    if (!await _confirm('退出后将停止接收这个群的新消息，确定退出？')) return;
    if (widget.controller.leaveGroup(group.id) && mounted) {
      Navigator.pop(context);
    }
  }

  Future<bool> _confirm(String text) async =>
      await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('确认操作'),
          content: Text(text),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('确认'),
            ),
          ],
        ),
      ) ??
      false;

  bool _apply(bool Function() operation) {
    final succeeded = operation();
    if (succeeded) {
      final updated = widget.controller.loadGroup(group.id);
      if (updated != null && mounted) setState(() => group = updated);
    } else if (widget.controller.lastError != null && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('群聊操作失败：${widget.controller.lastError}')),
      );
    }
    return succeeded;
  }
}

class _MemberAvatar extends StatelessWidget {
  const _MemberAvatar({required this.member});

  final GroupMemberView member;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: Stack(
        children: [
          const Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: _accentSoft,
                shape: BoxShape.circle,
              ),
              child: Icon(
                LucideIcons.monitorSmartphone,
                size: 18,
                color: _accent,
              ),
            ),
          ),
          if (member.online)
            const Positioned(
              right: 0,
              bottom: 0,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: Color(0xFF19A974),
                  shape: BoxShape.circle,
                  border: Border.fromBorderSide(
                    BorderSide(color: Colors.white, width: 2),
                  ),
                ),
                child: SizedBox(width: 10, height: 10),
              ),
            ),
        ],
      ),
    );
  }
}

class _PickGroupMembersDialog extends StatefulWidget {
  const _PickGroupMembersDialog({required this.devices});

  final List<NearbyDevice> devices;

  @override
  State<_PickGroupMembersDialog> createState() =>
      _PickGroupMembersDialogState();
}

class _PickGroupMembersDialogState extends State<_PickGroupMembersDialog> {
  final selected = <String>{};

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('邀请成员'),
      content: SizedBox(
        width: 400,
        child: widget.devices.isEmpty
            ? const Padding(
                padding: EdgeInsets.symmetric(vertical: 30),
                child: Center(child: Text('没有可邀请的在线设备')),
              )
            : ListView.builder(
                shrinkWrap: true,
                itemCount: widget.devices.length,
                itemBuilder: (context, index) {
                  final device = widget.devices[index];
                  return CheckboxListTile(
                    contentPadding: EdgeInsets.zero,
                    value: selected.contains(device.id),
                    title: Text(device.name),
                    subtitle: device.address == null
                        ? null
                        : Text(device.address!),
                    onChanged: (checked) => setState(() {
                      if (checked == true) {
                        selected.add(device.id);
                      } else {
                        selected.remove(device.id);
                      }
                    }),
                  );
                },
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: selected.isEmpty
              ? null
              : () => Navigator.pop(context, selected.toList(growable: false)),
          child: const Text('发送邀请'),
        ),
      ],
    );
  }
}

class _MessageBubble extends StatelessWidget {
  const _MessageBubble({
    super.key,
    required this.message,
    this.onStatusTap,
    this.onOpen,
    this.onShowLocation,
  });

  final ChatMessage message;
  final VoidCallback? onStatusTap;
  final VoidCallback? onOpen;
  final VoidCallback? onShowLocation;

  @override
  Widget build(BuildContext context) {
    final isFile = message.kind != MessageVisualKind.text;
    return Align(
      alignment: message.outgoing
          ? Alignment.centerRight
          : Alignment.centerLeft,
      child: Container(
        constraints: const BoxConstraints(maxWidth: 440, minWidth: 96),
        margin: const EdgeInsets.only(bottom: 12),
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          color: message.outgoing ? const Color(0xFFDDF4EA) : Colors.white,
          border: Border.all(
            color: message.outgoing ? const Color(0xFFBCE3D5) : _border,
          ),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (message.kind == MessageVisualKind.image &&
                message.localFileRef != null) ...[
              _MessageThumbnail(reference: message.localFileRef!),
              const SizedBox(height: 10),
            ],
            if (isFile)
              Row(
                children: [
                  Container(
                    width: 38,
                    height: 38,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: Colors.white.withValues(alpha: 0.72),
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Icon(
                      _messageIcon(message.kind),
                      color: _accent,
                      size: 20,
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          message.content,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(fontWeight: FontWeight.w600),
                        ),
                        if (message.fileSizeLabel != null)
                          Text(
                            message.fileSizeLabel!,
                            style: const TextStyle(color: _muted, fontSize: 11),
                          ),
                      ],
                    ),
                  ),
                ],
              )
            else
              SelectableText(message.content),
            if (message.progress case final progress?) ...[
              const SizedBox(height: 10),
              LinearProgressIndicator(
                value: progress,
                minHeight: 4,
                color: _accent,
                backgroundColor: const Color(0xFFCAE0DA),
              ),
            ],
            if (onOpen != null || onShowLocation != null) ...[
              const SizedBox(height: 6),
              Wrap(
                spacing: 2,
                children: [
                  if (onOpen != null)
                    TextButton.icon(
                      onPressed: onOpen,
                      icon: const Icon(LucideIcons.externalLink, size: 15),
                      label: Text(
                        message.kind == MessageVisualKind.folder
                            ? '打开目录'
                            : '打开',
                      ),
                    ),
                  if (onShowLocation != null)
                    TextButton.icon(
                      onPressed: onShowLocation,
                      icon: const Icon(LucideIcons.folderSearch, size: 15),
                      label: const Text('显示位置'),
                    ),
                ],
              ),
            ],
            const SizedBox(height: 7),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  message.timeLabel,
                  style: const TextStyle(color: _muted, fontSize: 10),
                ),
                if (message.statusLabel.isNotEmpty) ...[
                  const SizedBox(width: 8),
                  Flexible(
                    child: Tooltip(
                      message: onStatusTap == null ? '' : '查看送达详情',
                      child: InkWell(
                        key: ValueKey('message-delivery-${message.id}'),
                        onTap: onStatusTap,
                        child: Padding(
                          padding: const EdgeInsets.symmetric(vertical: 2),
                          child: Text(
                            message.statusLabel,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: message.statusLabel.contains('等待')
                                  ? _warning
                                  : _muted,
                              fontSize: 10,
                              decoration: onStatusTap == null
                                  ? null
                                  : TextDecoration.underline,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }

  IconData _messageIcon(MessageVisualKind kind) => switch (kind) {
    MessageVisualKind.file => LucideIcons.file,
    MessageVisualKind.image => LucideIcons.image,
    MessageVisualKind.folder => LucideIcons.folder,
    MessageVisualKind.clipboard => LucideIcons.clipboard,
    MessageVisualKind.text => LucideIcons.messageCircle,
  };
}

class _MessageThumbnail extends StatefulWidget {
  const _MessageThumbnail({required this.reference});

  final String reference;

  @override
  State<_MessageThumbnail> createState() => _MessageThumbnailState();
}

class _MessageThumbnailState extends State<_MessageThumbnail> {
  late Future<String?> _previewPath = PlatformBootstrap.resolveImagePreview(
    widget.reference,
  );

  @override
  void didUpdateWidget(covariant _MessageThumbnail oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.reference != widget.reference) {
      _previewPath = PlatformBootstrap.resolveImagePreview(widget.reference);
    }
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 240,
      height: 160,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(6),
        child: FutureBuilder<String?>(
          future: _previewPath,
          builder: (context, snapshot) {
            final path = snapshot.data;
            if (path == null) {
              return const ColoredBox(
                color: Color(0xFFE7ECEE),
                child: Center(child: Icon(LucideIcons.image, color: _muted)),
              );
            }
            return Image.file(
              File(path),
              fit: BoxFit.cover,
              cacheWidth: 640,
              errorBuilder: (_, _, _) => const ColoredBox(
                color: Color(0xFFE7ECEE),
                child: Center(child: Icon(LucideIcons.image, color: _muted)),
              ),
            );
          },
        ),
      ),
    );
  }
}

bool _deliveryFailed(String state) =>
    const {'rejected', 'failed', 'cancelled'}.contains(state);

IconData _deliveryIcon(MessageDeliveryView delivery) {
  if (delivery.delivered) return LucideIcons.circleCheck;
  if (_deliveryFailed(delivery.state)) return LucideIcons.circleX;
  return LucideIcons.clock3;
}

String _deliveryStateLabel(String state) => switch (state) {
  'stored' || 'completed' => '已送达',
  'accepted' => '已接受，等待传输',
  'sending' => '正在发送',
  'rejected' => '已拒绝',
  'failed' => '发送失败',
  'cancelled' => '已取消',
  _ => '等待上线',
};

class _Composer extends StatelessWidget {
  const _Composer({
    required this.textController,
    required this.appController,
    required this.onSend,
  });

  final TextEditingController textController;
  final AppController appController;
  final VoidCallback onSend;

  @override
  Widget build(BuildContext context) {
    final keyboardVisible = MediaQuery.viewInsetsOf(context).bottom > 0;
    return Container(
      padding: EdgeInsets.fromLTRB(
        14,
        keyboardVisible ? 6 : 10,
        14,
        keyboardVisible ? 6 : 14,
      ),
      decoration: const BoxDecoration(
        color: Colors.white,
        border: Border(top: BorderSide(color: _border)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          MenuAnchor(
            menuChildren: [
              MenuItemButton(
                leadingIcon: const Icon(LucideIcons.file, size: 18),
                onPressed: () => _sendSource(context, MessageVisualKind.file),
                child: const Text('文件'),
              ),
              MenuItemButton(
                leadingIcon: const Icon(LucideIcons.image, size: 18),
                onPressed: () => _sendSource(context, MessageVisualKind.image),
                child: const Text('图片'),
              ),
              MenuItemButton(
                leadingIcon: const Icon(LucideIcons.folder, size: 18),
                onPressed: () => _sendSource(context, MessageVisualKind.folder),
                child: const Text('文件夹'),
              ),
            ],
            builder: (context, menuController, _) => IconButton(
              tooltip: '附件',
              onPressed: () => menuController.isOpen
                  ? menuController.close()
                  : menuController.open(),
              icon: const Icon(LucideIcons.paperclip, size: 20),
            ),
          ),
          IconButton(
            tooltip: '剪贴板',
            onPressed: () => _sendClipboard(context),
            icon: const Icon(LucideIcons.clipboard, size: 20),
          ),
          Expanded(
            child: TextField(
              key: const ValueKey('message-input'),
              controller: textController,
              minLines: 1,
              maxLines: keyboardVisible ? 1 : 5,
              maxLength: 20000,
              buildCounter: (
                _, {
                required currentLength,
                required isFocused,
                maxLength,
              }) => null,
              onSubmitted: (_) => onSend(),
              decoration: const InputDecoration(hintText: '输入消息'),
            ),
          ),
          const SizedBox(width: 8),
          IconButton.filled(
            tooltip: '发送',
            onPressed: onSend,
            icon: const Icon(LucideIcons.send, size: 19),
          ),
        ],
      ),
    );
  }

  Future<void> _sendSource(BuildContext context, MessageVisualKind kind) async {
    final sent = await appController.sendSource(kind);
    if (!context.mounted || sent || appController.lastError == null) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('无法添加附件：${appController.lastError}')),
    );
  }

  Future<void> _sendClipboard(BuildContext context) async {
    final image = await appController.readClipboardImage();
    if (!context.mounted) return;
    if (image != null) {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('发送剪贴板图片'),
          content: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 480, maxHeight: 360),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Flexible(
                  child: Image.file(
                    File(image.sourceRef),
                    fit: BoxFit.contain,
                    errorBuilder: (_, _, _) =>
                        const Icon(LucideIcons.image, size: 64, color: _muted),
                  ),
                ),
                const SizedBox(height: 12),
                Text('${image.displayName} · ${_clipboardBytes(image.size)}'),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('取消'),
            ),
            FilledButton.icon(
              onPressed: () => Navigator.pop(context, true),
              icon: const Icon(LucideIcons.send, size: 17),
              label: const Text('发送'),
            ),
          ],
        ),
      );
      if (confirmed != true || !context.mounted) return;
      if (!appController.sendClipboardImage(image)) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('发送失败：${appController.lastError ?? '未知错误'}')),
        );
      }
      return;
    }
    final text = await appController.readClipboardText();
    if (!context.mounted) return;
    if (text == null || text.isEmpty) {
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text('剪贴板中没有文字')));
      return;
    }
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('发送剪贴板'),
        content: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 480, maxHeight: 280),
          child: SingleChildScrollView(child: SelectableText(text)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('取消'),
          ),
          FilledButton.icon(
            onPressed: () => Navigator.pop(context, true),
            icon: const Icon(LucideIcons.send, size: 17),
            label: const Text('发送'),
          ),
        ],
      ),
    );
    if (confirmed != true || !context.mounted) return;
    if (!appController.sendClipboardText(text)) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('发送失败：${appController.lastError ?? '未知错误'}')),
      );
    }
  }

  String _clipboardBytes(BigInt bytes) {
    final mib = bytes.toDouble() / (1024 * 1024);
    if (mib >= 1) return '${mib.toStringAsFixed(1)} MB';
    return '${(bytes.toDouble() / 1024).toStringAsFixed(1)} KB';
  }
}

enum _TransferFilter { active, waiting, completed, failed }

class _TransfersPane extends StatefulWidget {
  const _TransfersPane({
    required this.controller,
    required this.onClose,
    this.onBack,
  });

  final AppController controller;
  final VoidCallback onClose;
  final VoidCallback? onBack;

  @override
  State<_TransfersPane> createState() => _TransfersPaneState();
}

class _TransfersPaneState extends State<_TransfersPane> {
  _TransferFilter filter = _TransferFilter.active;

  @override
  Widget build(BuildContext context) {
    final transfers = widget.controller.transfers
        .where((transfer) => _matchesFilter(transfer, filter))
        .toList(growable: false);
    return ColoredBox(
      color: _canvas,
      child: SafeArea(
        child: Column(
          children: [
            Container(
              height: 68,
              padding: const EdgeInsets.symmetric(horizontal: 12),
              decoration: const BoxDecoration(
                color: Colors.white,
                border: Border(bottom: BorderSide(color: _border)),
              ),
              child: Row(
                children: [
                  if (widget.onBack != null)
                    IconButton(
                      tooltip: '返回',
                      onPressed: widget.onBack,
                      icon: const Icon(LucideIcons.arrowLeft, size: 20),
                    ),
                  const Expanded(
                    child: Text(
                      '当前传输',
                      style: TextStyle(
                        fontSize: 16,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ),
                  IconButton(
                    tooltip: '清理已完成记录',
                    onPressed:
                        widget.controller.transfers.any(
                          (item) => item.state == 'completed',
                        )
                        ? () => _clearCompleted(context)
                        : null,
                    icon: const Icon(LucideIcons.listX, size: 20),
                  ),
                  IconButton(
                    tooltip: '关闭',
                    onPressed: widget.onClose,
                    icon: const Icon(LucideIcons.x, size: 20),
                  ),
                ],
              ),
            ),
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
              child: SegmentedButton<_TransferFilter>(
                segments: const [
                  ButtonSegment(
                    value: _TransferFilter.active,
                    label: Text('进行中'),
                  ),
                  ButtonSegment(
                    value: _TransferFilter.waiting,
                    label: Text('等待'),
                  ),
                  ButtonSegment(
                    value: _TransferFilter.completed,
                    label: Text('已完成'),
                  ),
                  ButtonSegment(
                    value: _TransferFilter.failed,
                    label: Text('失败'),
                  ),
                ],
                selected: {filter},
                onSelectionChanged: (value) =>
                    setState(() => filter = value.single),
              ),
            ),
            Expanded(
              child: transfers.isEmpty
                  ? Center(child: Text(_emptyLabel(filter)))
                  : ListView.separated(
                      padding: const EdgeInsets.all(20),
                      itemCount: transfers.length,
                      separatorBuilder: (_, _) => const SizedBox(height: 8),
                      itemBuilder: (context, index) {
                        final transfer = transfers[index];
                        return _TransferRow(
                          transfer: transfer,
                          conversationTitle: _conversationTitle(transfer),
                          onTap: () => widget.controller
                              .openTransferConversation(transfer),
                          onOpen: transfer.localFileRef == null
                              ? null
                              : () => _openReference(
                                  context,
                                  transfer.localFileRef!,
                                  showInFolder: false,
                                ),
                          onShowLocation: transfer.localFileRef == null
                              ? null
                              : () => _openReference(
                                  context,
                                  transfer.localFileRef!,
                                  showInFolder: true,
                                ),
                          onAccept: transfer.awaitsDecision
                              ? () => _accept(context, transfer.id)
                              : null,
                          onReject: transfer.awaitsDecision
                              ? () => widget.controller.rejectIncomingOffer(
                                  transfer.id,
                                )
                              : null,
                          onPause: _canPause(transfer)
                              ? () =>
                                    widget.controller.pauseTransfer(transfer.id)
                              : null,
                          onResume: transfer.state == 'paused'
                              ? () => widget.controller.resumeTransfer(
                                  transfer.id,
                                )
                              : null,
                          onReselect: transfer.needsReselection
                              ? () => _reselect(context, transfer)
                              : null,
                          onCancel: _canCancel(transfer)
                              ? () => _confirmCancel(context, transfer)
                              : null,
                        );
                      },
                    ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _accept(BuildContext context, String transferId) async {
    final accepted = await widget.controller.acceptIncomingOffer(transferId);
    if (!context.mounted || accepted || widget.controller.lastError == null) {
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('无法接收：${widget.controller.lastError}')),
    );
  }

  Future<void> _reselect(BuildContext context, TransferView transfer) async {
    final replaced = await widget.controller.reselectTransferSource(transfer);
    if (!context.mounted || replaced || widget.controller.lastError == null) {
      return;
    }
    final subject = transfer.direction == 'receive' ? '接收目录不可用' : '源文件不匹配';
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('$subject：${widget.controller.lastError}')),
    );
  }

  bool _canPause(TransferView transfer) =>
      const {'queued', 'accepted', 'transferring'}.contains(transfer.state);

  bool _canCancel(TransferView transfer) =>
      !const {'completed', 'cancelled'}.contains(transfer.state);

  Future<void> _confirmCancel(
    BuildContext context,
    TransferView transfer,
  ) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('取消传输？'),
        content: Text('将取消“${transfer.displayName}”，已接收的临时数据会被删除。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('返回'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('取消传输'),
          ),
        ],
      ),
    );
    if (confirmed == true) widget.controller.cancelTransfer(transfer.id);
  }

  bool _matchesFilter(TransferView transfer, _TransferFilter value) =>
      switch (value) {
        _TransferFilter.active => const {
          'accepted',
          'transferring',
          'paused',
          'verifying',
        }.contains(transfer.state),
        _TransferFilter.waiting => const {
          'queued',
          'offered',
        }.contains(transfer.state),
        _TransferFilter.completed => transfer.state == 'completed',
        _TransferFilter.failed => const {
          'failed',
          'rejected',
          'cancelled',
        }.contains(transfer.state),
      };

  String _emptyLabel(_TransferFilter value) => switch (value) {
    _TransferFilter.active => '没有进行中的传输',
    _TransferFilter.waiting => '没有等待中的传输',
    _TransferFilter.completed => '没有已完成的传输',
    _TransferFilter.failed => '没有失败的传输',
  };

  String _conversationTitle(TransferView transfer) =>
      widget.controller.conversations
          .where((item) => item.id == transfer.conversationId)
          .map((item) => item.title)
          .firstOrNull ??
      transfer.peerDeviceId;

  Future<void> _openReference(
    BuildContext context,
    String reference, {
    required bool showInFolder,
  }) async {
    try {
      await PlatformBootstrap.openReference(
        reference,
        showInFolder: showInFolder,
      );
    } catch (error) {
      if (!context.mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(showInFolder ? '无法显示位置：$error' : '无法打开：$error')),
      );
    }
  }

  Future<void> _clearCompleted(BuildContext context) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('清理已完成记录？'),
        content: const Text('只清理传输记录，不会删除已接收或已发送的文件。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('清理'),
          ),
        ],
      ),
    );
    if (confirmed != true || !context.mounted) return;
    final count = widget.controller.clearCompletedTransfers();
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          count < 0
              ? '清理失败：${widget.controller.lastError ?? '未知错误'}'
              : '已清理 $count 条记录，文件未删除',
        ),
      ),
    );
  }
}

class _TransferRow extends StatelessWidget {
  const _TransferRow({
    required this.transfer,
    required this.conversationTitle,
    this.onTap,
    this.onOpen,
    this.onShowLocation,
    this.onAccept,
    this.onReject,
    this.onPause,
    this.onResume,
    this.onReselect,
    this.onCancel,
  });

  final TransferView transfer;
  final String conversationTitle;
  final VoidCallback? onTap;
  final VoidCallback? onOpen;
  final VoidCallback? onShowLocation;
  final VoidCallback? onAccept;
  final VoidCallback? onReject;
  final VoidCallback? onPause;
  final VoidCallback? onResume;
  final VoidCallback? onReselect;
  final VoidCallback? onCancel;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.white,
      shape: RoundedRectangleBorder(
        side: const BorderSide(color: _border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: InkWell(
        borderRadius: BorderRadius.circular(8),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Row(
            children: [
              const Icon(LucideIcons.file, color: _accent),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      transfer.displayName,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    const SizedBox(height: 3),
                    Text(
                      '${transfer.direction == 'send' ? '发送到' : '接收自'} '
                      '$conversationTitle',
                      style: const TextStyle(color: _muted, fontSize: 11),
                    ),
                    const SizedBox(height: 8),
                    LinearProgressIndicator(
                      value: transfer.progress,
                      minHeight: 4,
                      color: transfer.awaitsDecision ? _warning : _accent,
                      backgroundColor: _border,
                    ),
                    const SizedBox(height: 5),
                    Text(
                      '${_formatTransferBytes(transfer.persistedBytes)} / '
                      '${_formatTransferBytes(transfer.totalSize)} · '
                      '${_transferStatusLabel(transfer)}'
                      '${_transferLiveSuffix(transfer)}',
                      style: TextStyle(
                        color: transfer.awaitsDecision ? _warning : _muted,
                        fontSize: 11,
                      ),
                    ),
                    if (transfer.activeEntryRelativePath != null) ...[
                      const SizedBox(height: 3),
                      Text(
                        transfer.activeEntryRelativePath!,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(color: _muted, fontSize: 11),
                      ),
                    ],
                  ],
                ),
              ),
              const SizedBox(width: 8),
              if (onReject != null)
                IconButton(
                  tooltip: '拒绝',
                  onPressed: onReject,
                  icon: const Icon(LucideIcons.x, size: 19),
                ),
              if (onAccept != null)
                IconButton.filled(
                  tooltip: '接收',
                  onPressed: onAccept,
                  icon: const Icon(LucideIcons.download, size: 19),
                ),
              if (onPause != null)
                IconButton(
                  tooltip: '暂停',
                  onPressed: onPause,
                  icon: const Icon(LucideIcons.pause, size: 19),
                ),
              if (onResume != null)
                IconButton(
                  tooltip: '继续',
                  onPressed: onResume,
                  icon: const Icon(LucideIcons.play, size: 19),
                ),
              if (onReselect != null)
                IconButton.filledTonal(
                  tooltip: transfer.direction == 'receive'
                      ? '重新选择接收目录'
                      : '重新选择源文件',
                  onPressed: onReselect,
                  icon: const Icon(LucideIcons.folderOpen, size: 19),
                ),
              if (onCancel != null)
                IconButton(
                  tooltip: '取消传输',
                  onPressed: onCancel,
                  icon: const Icon(LucideIcons.trash2, size: 19),
                ),
              if (onOpen != null)
                IconButton(
                  tooltip: '打开',
                  onPressed: onOpen,
                  icon: const Icon(LucideIcons.externalLink, size: 19),
                ),
              if (onShowLocation != null)
                IconButton(
                  tooltip: '显示位置',
                  onPressed: onShowLocation,
                  icon: const Icon(LucideIcons.folderSearch, size: 19),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

String _transferStateLabel(String state) => switch (state) {
  'queued' => '等待设备上线',
  'offered' => '等待确认',
  'accepted' => '准备传输',
  'transferring' => '传输中',
  'paused' => '已暂停',
  'verifying' => '正在完成',
  'completed' => '已完成',
  'rejected' => '已拒绝',
  'cancelled' => '已取消',
  _ => '传输失败',
};

String _transferStatusLabel(TransferView transfer) =>
    switch (transfer.failureReason) {
      'source_changed' => '需要重新选择源文件',
      'permission_lost' => '文件权限失效，需要重新选择',
      'not_enough_space' => '接收设备空间不足',
      'rejected' => '对方已拒绝',
      'connection_error' => '连接中断，等待重试',
      'user_cancelled' => '已取消',
      _ => _transferStateLabel(transfer.state),
    };

String _formatTransferBytes(BigInt bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return '${value.toStringAsFixed(unit == 0 || value >= 100 ? 0 : 1)} '
      '${units[unit]}';
}

String _transferLiveSuffix(TransferView transfer) {
  if (transfer.bytesPerSecond <= BigInt.zero) return '';
  final speed = _formatTransferBytes(transfer.bytesPerSecond);
  final eta = transfer.etaSeconds;
  return eta == null ? ' · $speed/s' : ' · $speed/s · 剩余 ${_formatEta(eta)}';
}

String _formatEta(BigInt seconds) {
  final value = seconds > BigInt.from(359999) ? 359999 : seconds.toInt();
  final hours = value ~/ 3600;
  final minutes = (value % 3600) ~/ 60;
  final remainingSeconds = value % 60;
  if (hours > 0) return '$hours小时${minutes.toString().padLeft(2, '0')}分';
  if (minutes > 0) {
    return '$minutes分${remainingSeconds.toString().padLeft(2, '0')}秒';
  }
  return '$remainingSeconds秒';
}

class _MobileShell extends StatefulWidget {
  const _MobileShell({required this.controller, required this.coreRuntime});

  final AppController controller;
  final CoreRuntimeInfo coreRuntime;

  @override
  State<_MobileShell> createState() => _MobileShellState();
}

class _MobileShellState extends State<_MobileShell> {
  int index = 0;

  @override
  Widget build(BuildContext context) {
    if (widget.controller.compactDetailVisible) {
      return Scaffold(
        body: _MainPane(controller: widget.controller, showBackButton: true),
      );
    }
    final body = switch (index) {
      0 => _ConversationList(controller: widget.controller),
      1 => _NearbyList(controller: widget.controller),
      _ => _MobileSettings(
        coreRuntime: widget.coreRuntime,
        controller: widget.controller,
      ),
    };
    return Scaffold(
      appBar: AppBar(
        title: Text(switch (index) {
          0 => '会话',
          1 => '附近设备',
          _ => '设置',
        }),
      ),
      body: SafeArea(child: body),
      bottomNavigationBar: NavigationBar(
        selectedIndex: index,
        onDestinationSelected: (value) {
          setState(() => index = value);
          if (value < 2) {
            widget.controller.setSection(
              value == 0 ? SidebarSection.conversations : SidebarSection.nearby,
            );
          }
        },
        destinations: const [
          NavigationDestination(
            icon: Icon(LucideIcons.messageCircle),
            label: '会话',
          ),
          NavigationDestination(icon: Icon(LucideIcons.radar), label: '附近'),
          NavigationDestination(icon: Icon(LucideIcons.settings), label: '设置'),
        ],
      ),
    );
  }
}

class _MobileSettings extends StatelessWidget {
  const _MobileSettings({required this.coreRuntime, required this.controller});

  final CoreRuntimeInfo coreRuntime;
  final AppController controller;

  @override
  Widget build(BuildContext context) {
    return _SettingsContent(
      coreRuntime: coreRuntime,
      controller: controller,
      isAndroid: true,
    );
  }
}

class _SettingsDialog extends StatelessWidget {
  const _SettingsDialog({required this.coreRuntime, required this.controller});

  final CoreRuntimeInfo coreRuntime;
  final AppController controller;

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('设置'),
      content: SizedBox(
        width: 420,
        height: 560,
        child: _SettingsContent(
          coreRuntime: coreRuntime,
          controller: controller,
          isAndroid: false,
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('完成'),
        ),
      ],
    );
  }
}

class _SettingsContent extends StatelessWidget {
  const _SettingsContent({
    required this.coreRuntime,
    required this.controller,
    required this.isAndroid,
  });

  final CoreRuntimeInfo coreRuntime;
  final AppController controller;
  final bool isAndroid;

  @override
  Widget build(BuildContext context) {
    final settings = controller.settings;
    if (settings == null) {
      return const Center(child: CircularProgressIndicator());
    }
    return ListView(
      children: [
        ListTile(
          title: const Text('设备名称'),
          subtitle: Text(settings.deviceName),
          trailing: IconButton(
            tooltip: '修改设备名称',
            onPressed: () => _renameDevice(context, settings.deviceName),
            icon: const Icon(LucideIcons.pencil, size: 18),
          ),
        ),
        ListTile(
          title: const Text('本机设备 ID'),
          subtitle: Text(
            settings.deviceId,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          trailing: IconButton(
            tooltip: '复制设备 ID',
            onPressed: () async {
              await Clipboard.setData(ClipboardData(text: settings.deviceId));
              if (context.mounted) {
                ScaffoldMessenger.of(context)
                    .showSnackBar(const SnackBar(content: Text('设备 ID 已复制')));
              }
            },
            icon: const Icon(LucideIcons.copy, size: 18),
          ),
        ),
        ListTile(
          title: const Text('接收目录'),
          subtitle: Text(settings.defaultReceiveRef ?? '尚未设置'),
          trailing: IconButton(
            tooltip: '选择接收目录',
            onPressed: () => _pickReceiveDirectory(context, settings),
            icon: const Icon(LucideIcons.folderOpen, size: 18),
          ),
        ),
        ListTile(
          title: const Text('已知设备默认接收策略'),
          trailing: DropdownButton<String>(
            value: settings.defaultReceivePolicy,
            items: const [
              DropdownMenuItem(value: 'auto_accept', child: Text('自动接收')),
              DropdownMenuItem(value: 'ask_every_time', child: Text('每次询问')),
            ],
            onChanged: (value) {
              if (value != null) {
                _save(context, settings.copyWith(defaultReceivePolicy: value));
              }
            },
          ),
        ),
        SwitchListTile(
          value: settings.autoOpenReceiveDirectory,
          onChanged: (value) => _save(
            context,
            settings.copyWith(autoOpenReceiveDirectory: value),
          ),
          title: const Text('传输完成后打开接收目录'),
        ),
        if (!isAndroid) ...[
          SwitchListTile(
            value: settings.startOnBoot,
            onChanged: (value) =>
                _save(context, settings.copyWith(startOnBoot: value)),
            title: const Text('开机启动'),
          ),
          SwitchListTile(
            value: settings.closeToTray,
            onChanged: (value) =>
                _save(context, settings.copyWith(closeToTray: value)),
            title: const Text('关闭窗口时最小化到托盘'),
          ),
        ] else
          SwitchListTile(
            value: settings.androidKeepOnline,
            onChanged: (value) =>
                _save(context, settings.copyWith(androidKeepOnline: value)),
            title: const Text('后台保持在线'),
          ),
        SwitchListTile(
          value: settings.notificationsEnabled,
          onChanged: (value) =>
              _save(context, settings.copyWith(notificationsEnabled: value)),
          title: const Text('通知'),
        ),
        ListTile(
          title: const Text('诊断日志'),
          trailing: DropdownButton<String>(
            value: settings.logLevel,
            items: const [
              DropdownMenuItem(value: 'normal', child: Text('普通')),
              DropdownMenuItem(value: 'debug', child: Text('调试')),
            ],
            onChanged: (value) {
              if (value != null) {
                _save(context, settings.copyWith(logLevel: value));
              }
            },
          ),
        ),
        if (controller.peerReceivePolicies.isNotEmpty) ...[
          const Padding(
            padding: EdgeInsets.fromLTRB(16, 16, 16, 4),
            child: Text(
              '每设备接收策略',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
          ),
          for (final peer in controller.peerReceivePolicies)
            ListTile(
              title: Text(peer.deviceName),
              subtitle: Text(peer.relation == 'own_device' ? '我的设备' : '已知设备'),
              trailing: DropdownButton<String>(
                value: peer.override ?? 'inherit',
                items: const [
                  DropdownMenuItem(value: 'inherit', child: Text('跟随默认')),
                  DropdownMenuItem(value: 'auto_accept', child: Text('自动接收')),
                  DropdownMenuItem(
                    value: 'ask_every_time',
                    child: Text('每次询问'),
                  ),
                ],
                onChanged: (value) {
                  if (value == null) return;
                  final ok = controller.setPeerReceivePolicy(
                    peer.deviceId,
                    value == 'inherit' ? null : value,
                  );
                  if (!ok) _showError(context);
                },
              ),
            ),
        ],
        _OwnDevicesSettings(controller: controller),
        _CoreRuntimeTile(coreRuntime: coreRuntime),
      ],
    );
  }

  Future<void> _renameDevice(BuildContext context, String current) async {
    final text = TextEditingController(text: current);
    final next = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('修改设备名称'),
        content: TextField(
          controller: text,
          autofocus: true,
          maxLength: 32,
          decoration: const InputDecoration(labelText: '设备名称'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, text.text.trim()),
            child: const Text('保存'),
          ),
        ],
      ),
    );
    text.dispose();
    if (next == null || next.isEmpty || !context.mounted) return;
    if (!controller.renameDevice(next)) _showError(context);
  }

  Future<void> _pickReceiveDirectory(
    BuildContext context,
    AppSettingsView current,
  ) async {
    try {
      String? reference;
      if (isAndroid) {
        reference = await PlatformBootstrap.pickReceiveDirectory();
      } else {
        reference = await PlatformBootstrap.pickSource('folder');
      }
      if (reference == null || !context.mounted) return;
      await _save(context, current.copyWith(defaultReceiveRef: reference));
    } catch (error) {
      controller.lastError = error.toString();
      if (context.mounted) _showError(context);
    }
  }

  Future<void> _save(BuildContext context, AppSettingsView next) async {
    if (!await controller.saveSettings(next) && context.mounted) {
      _showError(context);
    }
  }

  void _showError(BuildContext context) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('设置保存失败：${controller.lastError ?? '未知错误'}')),
    );
  }
}

class _OwnDevicesSettings extends StatelessWidget {
  const _OwnDevicesSettings({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    final bindings = controller.activeOwnDeviceBindings;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Padding(
          padding: EdgeInsets.fromLTRB(16, 12, 16, 4),
          child: Text('我的设备', style: TextStyle(fontWeight: FontWeight.w700)),
        ),
        if (bindings.isEmpty)
          const ListTile(
            leading: Icon(LucideIcons.link),
            title: Text('尚未绑定设备'),
            subtitle: Text('在私聊右上角的更多菜单中发起绑定'),
          ),
        for (final binding in bindings)
          ListTile(
            key: ValueKey('own-device-${binding.peerDeviceId}'),
            leading: Icon(
              LucideIcons.smartphone,
              color: binding.online ? _accent : _muted,
            ),
            title: Text(binding.peerName),
            subtitle: Text(binding.online ? '在线' : '离线'),
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                DropdownButton<String>(
                  value: binding.clipboardMode,
                  underline: const SizedBox.shrink(),
                  items: const [
                    DropdownMenuItem(value: 'off', child: Text('关闭')),
                    DropdownMenuItem(value: 'send_only', child: Text('仅发送')),
                    DropdownMenuItem(value: 'receive_only', child: Text('仅接收')),
                    DropdownMenuItem(value: 'bidirectional', child: Text('双向')),
                  ],
                  onChanged: (mode) {
                    if (mode != null) {
                      controller.setClipboardMode(binding.peerDeviceId, mode);
                    }
                  },
                ),
                IconButton(
                  tooltip: '解除绑定',
                  onPressed: () =>
                      controller.removeOwnDeviceBinding(binding.peerDeviceId),
                  icon: const Icon(LucideIcons.unlink, size: 18),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

class _CoreRuntimeTile extends StatelessWidget {
  const _CoreRuntimeTile({required this.coreRuntime});

  final CoreRuntimeInfo coreRuntime;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Icon(
        coreRuntime.ready ? LucideIcons.circleCheck : LucideIcons.circleAlert,
        color: coreRuntime.ready
            ? _accent
            : Theme.of(context).colorScheme.error,
      ),
      title: Text(coreRuntime.summary),
      subtitle: coreRuntime.error == null
          ? null
          : Text(
              coreRuntime.error!,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
    );
  }
}

class _Avatar extends StatelessWidget {
  const _Avatar({
    required this.label,
    required this.group,
    required this.online,
  });

  final String label;
  final bool group;
  final bool online;

  @override
  Widget build(BuildContext context) {
    return Stack(
      clipBehavior: Clip.none,
      children: [
        CircleAvatar(
          radius: 19,
          backgroundColor: group
              ? const Color(0xFFD8E5F2)
              : const Color(0xFFE7E0F2),
          foregroundColor: group
              ? const Color(0xFF2E5D82)
              : const Color(0xFF604782),
          child: group
              ? const Icon(LucideIcons.users, size: 18)
              : Text(
                  label.characters.first,
                  style: const TextStyle(fontWeight: FontWeight.w700),
                ),
        ),
        if (online)
          Positioned(
            right: -1,
            bottom: -1,
            child: Container(
              width: 10,
              height: 10,
              decoration: BoxDecoration(
                color: const Color(0xFF19A974),
                shape: BoxShape.circle,
                border: Border.all(color: Colors.white, width: 2),
              ),
            ),
          ),
      ],
    );
  }
}

class _DeviceIcon extends StatelessWidget {
  const _DeviceIcon({required this.platform});

  final DevicePlatform platform;

  @override
  Widget build(BuildContext context) {
    final android = platform == DevicePlatform.android;
    return Container(
      width: 38,
      height: 38,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: android ? const Color(0xFFE4F2E8) : const Color(0xFFE3ECF7),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Icon(
        android ? LucideIcons.smartphone : LucideIcons.monitor,
        size: 19,
        color: android ? const Color(0xFF347451) : const Color(0xFF3E648B),
      ),
    );
  }
}
