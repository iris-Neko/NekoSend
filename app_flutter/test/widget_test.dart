import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/application/app_controller.dart';
import 'package:lan_chat/application/models.dart';
import 'package:lan_chat/presentation/lan_chat_app.dart';

const _fixtureNearbyDevices = [
  NearbyDevice(
    id: 'phone',
    name: '我的手机',
    platform: DevicePlatform.android,
    relationLabel: '我的设备',
  ),
  NearbyDevice(
    id: 'tablet',
    name: '客厅平板',
    platform: DevicePlatform.android,
    relationLabel: null,
  ),
  NearbyDevice(
    id: 'laptop',
    name: '书房笔记本',
    platform: DevicePlatform.windows,
    relationLabel: '已知',
  ),
];

const _fixtureConversations = [
  ConversationSummary(
    id: 'family',
    title: '家里人',
    preview: '[文件夹] 家庭相册（128 项）',
    timeLabel: '20:18',
    isGroup: true,
    online: true,
    unreadCount: 2,
    transferProgress: 0.72,
  ),
  ConversationSummary(
    id: 'phone',
    title: '我的手机',
    preview: '[文件] report.pdf',
    timeLabel: '昨天',
    isGroup: false,
    online: true,
    peerDeviceId: 'phone',
  ),
  ConversationSummary(
    id: 'laptop',
    title: '书房笔记本',
    preview: '[剪贴板] 192.168.1.20',
    timeLabel: '周一',
    isGroup: false,
    online: false,
    peerDeviceId: 'laptop',
  ),
];

const _fixtureMessages = <String, List<ChatMessage>>{
  'family': [
    ChatMessage(
      id: 'm1',
      content: '周末的照片我整理好了',
      timeLabel: '20:16',
      outgoing: false,
      kind: MessageVisualKind.text,
      statusLabel: '',
    ),
    ChatMessage(
      id: 'm2',
      content: '家庭相册',
      timeLabel: '20:17',
      outgoing: true,
      kind: MessageVisualKind.folder,
      statusLabel: '2/3 已送达',
      fileSizeLabel: '128 项 · 2.8 GB',
      progress: 0.72,
    ),
  ],
  'phone': [
    ChatMessage(
      id: 'm3',
      content: 'report.pdf',
      timeLabel: '昨天',
      outgoing: true,
      kind: MessageVisualKind.file,
      statusLabel: '已完成',
      fileSizeLabel: '18.4 MB',
      progress: 1,
    ),
  ],
};

AppController _buildUiFixtureController({
  SendTextCommand? sendTextCommand,
  DeviceNameUpdater? deviceNameUpdater,
}) {
  final controller = AppController(
    initialNearbyDevices: _fixtureNearbyDevices,
    initialConversations: _fixtureConversations,
    initialMessages: _fixtureMessages,
    deviceNameUpdater: deviceNameUpdater,
    openPrivateConversation: (device) => _fixtureConversations.firstWhere(
      (conversation) => conversation.peerDeviceId == device.id,
    ),
    sendTextCommand:
        sendTextCommand ??
        (_, text) => ChatMessage(
          id: 'sent-${text.hashCode}',
          content: text,
          timeLabel: '刚刚',
          outgoing: true,
          kind: MessageVisualKind.text,
          statusLabel: '正在发送',
        ),
  );
  return controller;
}

Future<AppController> _pumpComposerFixture(
  WidgetTester tester, {
  TargetPlatform platform = TargetPlatform.windows,
  Size size = const Size(1000, 700),
  SendTextCommand? sendTextCommand,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final controller = _buildUiFixtureController(
    sendTextCommand: sendTextCommand,
  );
  addTearDown(controller.dispose);
  controller.selectConversation('phone');
  await tester.pumpWidget(
    MaterialApp(
      home: LanChatHome(controller: controller, platform: platform),
    ),
  );
  return controller;
}

void main() {
  for (final sourceIndex in [0, 1]) {
    testWidgets(
      'Android system back returns a chat to source tab $sourceIndex',
      (tester) async {
        await tester.binding.setSurfaceSize(const Size(360, 800));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final controller = _buildUiFixtureController();
        addTearDown(controller.dispose);
        await tester.pumpWidget(
          MaterialApp(
            home: LanChatHome(
              controller: controller,
              platform: TargetPlatform.android,
            ),
          ),
        );
        if (sourceIndex == 1) {
          await tester.tap(find.text('附近'));
          await tester.pump();
        }
        controller.selectConversation('phone');
        await tester.pump();
        expect(find.byKey(const ValueKey('message-input')), findsOneWidget);
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(controller.compactDetailVisible, isFalse);
        expect(
          tester
              .widget<NavigationBar>(find.byType(NavigationBar))
              .selectedIndex,
          sourceIndex,
        );
        expect(find.byKey(const ValueKey('message-input')), findsNothing);
      },
    );
  }

  testWidgets('Android back returns transfers to the list', (tester) async {
    final controller = await _pumpComposerFixture(
      tester,
      platform: TargetPlatform.android,
      size: const Size(360, 800),
    );
    controller.showTransfers();
    await tester.pump();
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(controller.compactDetailVisible, isFalse);
    expect(find.byType(NavigationBar), findsOneWidget);
  });

  testWidgets(
    'Android root back backgrounds only after returning from another tab',
    (tester) async {
      const channel = MethodChannel('dev.lanchat/platform');
      final calls = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call.method);
        return null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      await tester.binding.setSurfaceSize(const Size(360, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final controller = _buildUiFixtureController();
      addTearDown(controller.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.android,
          ),
        ),
      );
      await tester.tap(find.text('附近'));
      await tester.pump();
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(
        tester.widget<NavigationBar>(find.byType(NavigationBar)).selectedIndex,
        0,
      );
      expect(calls, isEmpty);
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(calls, ['moveToBackground']);
      expect(find.byType(LanChatHome), findsOneWidget);
    },
  );

  testWidgets(
    'Android back first dismisses the keyboard without leaving the chat',
    (tester) async {
      final controller = await _pumpComposerFixture(
        tester,
        platform: TargetPlatform.android,
        size: const Size(360, 800),
      );
      final input = find.byKey(const ValueKey('message-input'));
      await tester.enterText(input, '未发送草稿');
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      addTearDown(tester.view.resetViewInsets);
      await tester.pump();
      await tester.binding.handlePopRoute();
      await tester.pump();
      expect(controller.compactDetailVisible, isTrue);
      expect(tester.widget<TextField>(input).controller!.text, '未发送草稿');
      expect(
        tester
            .widget<EditableText>(
              find.descendant(of: input, matching: find.byType(EditableText)),
            )
            .focusNode
            .hasFocus,
        isFalse,
      );
      tester.view.resetViewInsets();
      await tester.pump();
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(controller.compactDetailVisible, isFalse);
    },
  );

  const nameSettings = AppSettingsView(
    deviceName: 'Xiaomi 14',
    deviceId: 'device-name-test',
    platform: 'android',
    defaultReceivePolicy: 'auto_accept',
    defaultReceiveRef: null,
    notificationsEnabled: false,
    closeToTray: true,
    startOnBoot: false,
    androidKeepOnline: true,
    logLevel: 'normal',
  );

  testWidgets(
    'Windows device name row saves and immediately refreshes the dialog',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final renamed = <String>[];
      final controller = _buildUiFixtureController(
        deviceNameUpdater: (name) {
          renamed.add(name);
          return nameSettings.copyWith(deviceName: name);
        },
      );
      controller.settings = nameSettings;
      addTearDown(controller.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.windows,
          ),
        ),
      );
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('device-name-setting')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('device-name-input')),
        '  我的工作电脑  ',
      );
      await tester.tap(find.text('保存'));
      await tester.pumpAndSettle();
      expect(renamed, ['我的工作电脑']);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('device-name-setting')),
          matching: find.text('我的工作电脑'),
        ),
        findsOneWidget,
      );
      expect(controller.settings!.deviceId, 'device-name-test');
      await tester.tap(find.byKey(const ValueKey('device-name-setting')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('device-name-input')),
        '不保存的名字',
      );
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(renamed, hasLength(1));
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('device-name-setting')),
          matching: find.text('我的工作电脑'),
        ),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('Android back dismisses a dialog before leaving settings', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(360, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = _buildUiFixtureController();
    controller.settings = nameSettings;
    addTearDown(controller.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.android,
        ),
      ),
    );
    await tester.tap(find.text('设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('device-name-setting')));
    await tester.pumpAndSettle();
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('device-name-input')), findsNothing);
    expect(
      tester.widget<NavigationBar>(find.byType(NavigationBar)).selectedIndex,
      2,
    );
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(
      tester.widget<NavigationBar>(find.byType(NavigationBar)).selectedIndex,
      0,
    );
  });

  testWidgets(
    'Android rename validates input and keeps the draft after save failure',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(360, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      var attempts = 0;
      var failSave = true;
      final controller = _buildUiFixtureController(
        deviceNameUpdater: (name) {
          attempts++;
          if (failSave) throw StateError('无法保存名称');
          return nameSettings.copyWith(deviceName: name);
        },
      );
      controller.settings = nameSettings;
      addTearDown(controller.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.android,
          ),
        ),
      );
      await tester.tap(find.text('设置'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('device-name-setting')));
      await tester.pumpAndSettle();
      final input = find.byKey(const ValueKey('device-name-input'));
      await tester.enterText(input, '   ');
      await tester.tap(find.text('保存'));
      await tester.pump();
      expect(attempts, 0);
      expect(find.text('请输入设备名称'), findsOneWidget);
      await tester.enterText(input, List.filled(5, '👨‍👩‍👧‍👦').join());
      await tester.tap(find.text('保存'));
      await tester.pump();
      expect(attempts, 0);
      expect(find.text('设备名称最多 32 个字符'), findsOneWidget);
      await tester.enterText(input, '我的手机');
      await tester.tap(find.text('保存'));
      await tester.pump();
      expect(attempts, 1);
      expect(find.byKey(const ValueKey('device-name-input')), findsOneWidget);
      expect(find.text('我的手机'), findsOneWidget);
      expect(controller.settings!.deviceName, 'Xiaomi 14');
      failSave = false;
      await tester.tap(find.text('保存'));
      await tester.pumpAndSettle();
      expect(attempts, 2);
      expect(controller.settings!.deviceName, '我的手机');
      expect(find.text('我的手机'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  test('controller has no built-in data or no-op success commands', () {
    final controller = AppController();
    addTearDown(controller.dispose);

    expect(controller.nearbyDevices, isEmpty);
    expect(controller.conversations, isEmpty);
    expect(controller.requestOwnDeviceBinding('missing'), isFalse);
    expect(controller.setClipboardMode('missing', 'both'), isFalse);
  });

  test('first real conversation is selected and its messages are loaded', () {
    const conversation = ConversationSummary(
      id: 'private-1',
      title: '测试手机',
      preview: '手机发来的消息',
      timeLabel: '刚刚',
      isGroup: false,
      online: true,
    );
    const incoming = ChatMessage(
      id: 'message-1',
      content: '手机发来的消息',
      timeLabel: '刚刚',
      outgoing: false,
      kind: MessageVisualKind.text,
      statusLabel: '',
    );
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [conversation],
      messageLoader: (conversationId) =>
          conversationId == conversation.id ? const [incoming] : const [],
    );
    addTearDown(controller.dispose);

    expect(controller.selectedConversationId, conversation.id);
    expect(controller.selectedMessages, const [incoming]);
  });

  testWidgets('core event stream refreshes snapshots and reconnects', (
    tester,
  ) async {
    final first = StreamController<CoreEventUpdate>();
    final second = StreamController<CoreEventUpdate>();
    var subscriptions = 0;
    var nearbyLoads = 0;
    var conversationLoads = 0;
    final controller = AppController(
      nearbyPeerLoader: () {
        nearbyLoads++;
        return const [];
      },
      conversationLoader: () {
        conversationLoads++;
        return const [];
      },
      coreEventStreamFactory: () {
        subscriptions++;
        return subscriptions == 1 ? first.stream : second.stream;
      },
    );
    try {
      expect(nearbyLoads, 1);
      expect(conversationLoads, 1);
      expect(subscriptions, 1);
      first.add(const CoreEventUpdate(CoreEventArea.conversation));
      await tester.pump();
      expect(nearbyLoads, 1);
      expect(conversationLoads, 2);

      await first.close();
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(subscriptions, 2);

      final beforeSecondEvent = nearbyLoads;
      second.add(const CoreEventUpdate(CoreEventArea.all));
      await tester.pump();
      expect(nearbyLoads, beforeSecondEvent + 1);
    } finally {
      controller.dispose();
    }
  });

  testWidgets(
    'transfer progress events update memory without database reload',
    (tester) async {
      final events = StreamController<CoreEventUpdate>();
      var transferLoads = 0;
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        transferLoader: () {
          transferLoads++;
          return [
            TransferView(
              id: 'transfer-1',
              messageId: 'message-1',
              peerDeviceId: 'device-1',
              direction: 'receive',
              state: 'transferring',
              displayName: 'photos',
              totalSize: BigInt.from(1000),
              entryCount: 2,
              persistedBytes: BigInt.zero,
              pausedByUser: false,
            ),
          ];
        },
        coreEventStreamFactory: () => events.stream,
      );
      try {
        expect(transferLoads, 1);
        events.add(
          CoreEventUpdate(
            CoreEventArea.transfer,
            transferProgress: TransferProgressUpdate(
              transferId: 'transfer-1',
              state: 'transferring',
              persistedBytes: BigInt.from(500),
              totalSize: BigInt.from(1000),
              bytesPerSecond: BigInt.from(250),
              etaSeconds: BigInt.from(2),
              activeEntryRelativePath: 'album/photo.jpg',
            ),
          ),
        );
        await tester.pump();

        expect(transferLoads, 1);
        expect(controller.transfers.single.persistedBytes, BigInt.from(500));
        expect(controller.transfers.single.bytesPerSecond, BigInt.from(250));
        expect(controller.transfers.single.etaSeconds, BigInt.from(2));
        expect(
          controller.transfers.single.activeEntryRelativePath,
          'album/photo.jpg',
        );
      } finally {
        controller.dispose();
      }
    },
  );

  testWidgets(
    'auto-open ignores history and opens each newly received directory once',
    (tester) async {
      final events = StreamController<CoreEventUpdate>();
      final opened = <String>[];
      final historic = TransferView(
        id: 'historic-transfer',
        messageId: 'historic-message',
        conversationId: 'private-1',
        peerDeviceId: 'peer-1',
        direction: 'receive',
        state: 'completed',
        displayName: 'historic.pdf',
        totalSize: BigInt.one,
        entryCount: 1,
        persistedBytes: BigInt.one,
        pausedByUser: false,
        receiveBaseRef: 'content://tree/history',
        localFileRef: 'content://document/history',
      );
      var loaded = <TransferView>[historic];
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        transferLoader: () => loaded,
        settingsLoader: () => const AppSettingsView(
          deviceName: 'Phone',
          deviceId: 'device-1',
          platform: 'android',
          defaultReceivePolicy: 'auto_accept',
          defaultReceiveRef: 'content://tree/receive',
          notificationsEnabled: false,
          closeToTray: false,
          startOnBoot: false,
          androidKeepOnline: true,
          autoOpenReceiveDirectory: true,
          logLevel: 'normal',
        ),
        openReferenceCommand: (reference, showInFolder) async {
          opened.add('$reference|$showInFolder');
        },
        coreEventStreamFactory: () => events.stream,
      );
      try {
        expect(opened, isEmpty);
        final fresh = TransferView(
          id: 'fresh-transfer',
          messageId: 'fresh-message',
          conversationId: 'private-1',
          peerDeviceId: 'peer-1',
          direction: 'receive',
          state: 'completed',
          displayName: 'album',
          totalSize: BigInt.one,
          entryCount: 2,
          persistedBytes: BigInt.one,
          pausedByUser: false,
          receiveBaseRef: 'content://tree/receive',
          localFileRef: 'content://document/album',
        );
        loaded = [fresh, historic];
        events.add(const CoreEventUpdate(CoreEventArea.transfer));
        await tester.pump();
        expect(opened, ['content://tree/receive|true']);

        events.add(const CoreEventUpdate(CoreEventArea.transfer));
        await tester.pump();
        expect(opened, hasLength(1));
      } finally {
        controller.dispose();
      }
    },
  );

  test(
    'real-data controller opens a nearby peer without demo conversations',
    () {
      const device = NearbyDevice(
        id: 'd_00000000000000000000000000000002',
        name: '测试手机',
        platform: DevicePlatform.android,
        relationLabel: null,
      );
      var sent = false;
      final controller = AppController(
        nearbyPeerLoader: () => const [device],
        conversationLoader: () => const [],
        messageLoader: (_) => const [],
        openPrivateConversation: (_) => const ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
          peerDeviceId: 'd_00000000000000000000000000000002',
        ),
        sendTextCommand: (conversationId, text) {
          sent = conversationId == 'private-1' && text == '你好';
          return const ChatMessage(
            id: 'message-1',
            content: '你好',
            timeLabel: '刚刚',
            outgoing: true,
            kind: MessageVisualKind.text,
            statusLabel: '等待发送',
          );
        },
      );
      addTearDown(controller.dispose);

      expect(controller.conversations, isEmpty);
      controller.selectNearbyDevice(device);
      expect(controller.selectedConversation.title, '测试手机');
      expect(controller.sendText('你好'), isTrue);
      expect(sent, isTrue);
      expect(controller.selectedMessages.single.content, '你好');
    },
  );

  test('send failure is exposed instead of being silently discarded', () {
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      sendTextCommand: (_, _) => throw StateError('Rust 核心正在重启'),
    );
    addTearDown(controller.dispose);
    controller.selectConversation('private-1');

    expect(controller.sendText('不能丢失的消息'), isFalse);
    expect(controller.lastError, contains('Rust 核心正在重启'));
    expect(controller.selectedMessages, isEmpty);
  });

  test('controller presents a stable user-facing core error', () {
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      sendTextCommand: (_, _) => throw StateError('database write failed: 5'),
      errorPresenter: (_) => '无法保存本地状态',
    );
    addTearDown(controller.dispose);
    controller.selectConversation('private-1');

    expect(controller.sendText('不会入库'), isFalse);
    expect(controller.lastError, '无法保存本地状态');
    expect(controller.lastError, isNot(contains('database')));
  });

  testWidgets('Windows 800px and wider uses two columns', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = _buildUiFixtureController();
    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );

    expect(find.byKey(const ValueKey('desktop-sidebar')), findsOneWidget);
    expect(find.byKey(const ValueKey('desktop-main-pane')), findsOneWidget);
    expect(find.text('家里人'), findsWidgets);
  });

  testWidgets('Windows 640px uses list to chat single-pane routing', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(640, 520));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = _buildUiFixtureController();
    addTearDown(controller.dispose);

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    expect(find.byKey(const ValueKey('desktop-sidebar')), findsNothing);

    await tester.tap(find.text('我的手机').first);
    await tester.pump();
    expect(find.byKey(const ValueKey('message-input')), findsOneWidget);
  });

  testWidgets(
    'Android 360px renders long live transfer details without overflow',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(360, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final controller = _buildUiFixtureController();
      addTearDown(controller.dispose);
      final longName = '${List.filled(40, '家庭照片').join()}-archive.zip';
      controller.transfers = [
        TransferView(
          id: 'transfer-mobile',
          messageId: 'message-mobile',
          peerDeviceId: 'phone',
          direction: 'send',
          state: 'transferring',
          displayName: longName,
          totalSize: BigInt.from(1024 * 1024 * 1024),
          entryCount: 120,
          persistedBytes: BigInt.from(512 * 1024 * 1024),
          pausedByUser: false,
          bytesPerSecond: BigInt.from(64 * 1024 * 1024),
          etaSeconds: BigInt.from(8),
          activeEntryRelativePath: '$longName/2026/照片.jpg',
        ),
      ];
      controller.showTransfers();

      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.android,
          ),
        ),
      );
      await tester.pump();

      expect(find.text('当前传输'), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsOneWidget);
      expect(find.textContaining('64.0 MB/s'), findsOneWidget);
      expect(find.textContaining('剩余 8秒'), findsOneWidget);
      expect(find.byTooltip('暂停'), findsOneWidget);
      expect(find.byTooltip('取消传输'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('message composer appends a local message', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = _buildUiFixtureController();
    addTearDown(controller.dispose);

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.enterText(find.byKey(const ValueKey('message-input')), '测试消息');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();

    expect(find.text('测试消息'), findsOneWidget);
  });

  for (final physicalKey in [
    PhysicalKeyboardKey.enter,
    PhysicalKeyboardKey.numpadEnter,
  ]) {
    testWidgets(
      'Windows ${physicalKey.debugName} sends once and retains input focus',
      (tester) async {
        final controller = await _pumpComposerFixture(
          tester,
          size: Size(
            physicalKey == PhysicalKeyboardKey.enter ? 1000 : 640,
            700,
          ),
        );
        final input = find.byKey(const ValueKey('message-input'));
        await tester.enterText(input, '实体回车发送');
        await tester.sendKeyEvent(
          LogicalKeyboardKey.enter,
          physicalKey: physicalKey,
        );
        await tester.pump();

        expect(
          controller.selectedMessages.where((m) => m.content == '实体回车发送'),
          hasLength(1),
        );
        expect(tester.widget<TextField>(input).controller!.text, isEmpty);
        expect(
          tester
              .widget<EditableText>(
                find.descendant(of: input, matching: find.byType(EditableText)),
              )
              .focusNode
              .hasFocus,
          isTrue,
        );
      },
      variant: TargetPlatformVariant.only(TargetPlatform.windows),
    );
  }

  testWidgets('macOS desktop sends with Enter and preserves IME composition', (
    tester,
  ) async {
    final controller = await _pumpComposerFixture(
      tester,
      platform: TargetPlatform.macOS,
    );
    final count = controller.selectedMessages.length;
    final input = find.byKey(const ValueKey('message-input'));
    await tester.showKeyboard(input);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'Mac message',
        selection: TextSelection.collapsed(offset: 11),
        composing: TextRange(start: 0, end: 11),
      ),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(controller.selectedMessages, hasLength(count));
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'Mac message',
        selection: TextSelection.collapsed(offset: 11),
      ),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(controller.selectedMessages.last.content, 'Mac message');
    expect(controller.selectedMessages, hasLength(count + 1));
    expect(tester.widget<TextField>(input).controller!.text, isEmpty);
    expect(tester.takeException(), isNull);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('Windows Shift+Enter inserts a newline without sending', (
    tester,
  ) async {
    final controller = await _pumpComposerFixture(tester);
    final count = controller.selectedMessages.length;
    final input = find.byKey(const ValueKey('message-input'));
    await tester.enterText(input, '第一行');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    final handled = await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();

    expect(handled, isFalse);
    expect(controller.selectedMessages, hasLength(count));
    // Key simulation does not perform the native text input edit.
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '第一行\n',
        selection: TextSelection.collapsed(offset: 4),
      ),
    );
    await tester.pump();
    expect(tester.widget<TextField>(input).controller!.text, '第一行\n');
    expect(controller.selectedMessages, hasLength(count));
    await tester.enterText(input, '第一行\n第二行');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(controller.selectedMessages.last.content, '第一行\n第二行');
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));

  testWidgets('Windows Enter leaves IME composition to the input method', (
    tester,
  ) async {
    final controller = await _pumpComposerFixture(tester);
    final count = controller.selectedMessages.length;
    final input = find.byKey(const ValueKey('message-input'));
    await tester.showKeyboard(input);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '中文',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      ),
    );
    await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(controller.selectedMessages, hasLength(count));
    expect(tester.widget<TextField>(input).controller!.text, '中文');
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '中文',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
    expect(controller.selectedMessages, hasLength(count));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(controller.selectedMessages.last.content, '中文');
    expect(controller.selectedMessages, hasLength(count + 1));
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));

  testWidgets(
    'Windows held Enter does not retry a failure or discard the draft',
    (tester) async {
      var attempts = 0;
      await _pumpComposerFixture(
        tester,
        sendTextCommand: (_, _) {
          attempts++;
          throw StateError('send failed');
        },
      );
      final input = find.byKey(const ValueKey('message-input'));
      await tester.enterText(input, '保留草稿');
      await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
      await tester.pump();

      expect(attempts, 1);
      expect(tester.widget<TextField>(input).controller!.text, '保留草稿');
      await tester.enterText(input, '   ');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(attempts, 1);
      expect(tester.widget<TextField>(input).controller!.text, '   ');
    },
    variant: TargetPlatformVariant.only(TargetPlatform.windows),
  );

  testWidgets('Android retains hardware newline and keyboard submit behavior', (
    tester,
  ) async {
    final controller = await _pumpComposerFixture(
      tester,
      platform: TargetPlatform.android,
      size: const Size(360, 800),
    );
    final count = controller.selectedMessages.length;
    final input = find.byKey(const ValueKey('message-input'));
    await tester.enterText(input, 'Android message');
    final handled = await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(handled, isFalse);
    expect(controller.selectedMessages, hasLength(count));
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'Android message\n',
        selection: TextSelection.collapsed(offset: 16),
      ),
    );
    await tester.pump();
    expect(
      tester.widget<TextField>(input).controller!.text,
      'Android message\n',
    );
    expect(controller.selectedMessages, hasLength(count));
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(controller.selectedMessages, hasLength(count + 1));
    expect(controller.selectedMessages.last.content, 'Android message\n');
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));

  testWidgets('opening a long conversation starts at the newest message', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 500));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final messages = List.generate(
      30,
      (index) => ChatMessage(
        id: 'message-$index',
        content: '历史消息 $index',
        timeLabel: '刚刚',
        outgoing: index.isEven,
        kind: MessageVisualKind.text,
        statusLabel: '已送达',
      ),
    );
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '历史消息 29',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
          peerDeviceId: 'd_00000000000000000000000000000002',
        ),
      ],
      messageLoader: (_) => messages,
    );
    controller.selectConversation('private-1');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.pump();

    final list = tester.widget<ListView>(
      find.byKey(const ValueKey('conversation-message-list')),
    );
    expect(
      list.controller!.position.pixels,
      list.controller!.position.minScrollExtent,
    );
    expect(find.text('历史消息 29'), findsWidgets);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('sending a message returns a scrolled conversation to bottom', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 500));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final messages = List.generate(
      30,
      (index) => ChatMessage(
        id: 'message-$index',
        content: '历史消息 $index',
        timeLabel: '刚刚',
        outgoing: false,
        kind: MessageVisualKind.text,
        statusLabel: '已送达',
      ),
    );
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '历史消息 29',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => messages,
      sendTextCommand: (_, text) => ChatMessage(
        id: 'local-message',
        content: text,
        timeLabel: '刚刚',
        outgoing: true,
        kind: MessageVisualKind.text,
        statusLabel: '正在发送',
      ),
    );
    controller.selectConversation('private-1');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.pump();
    final list = tester.widget<ListView>(
      find.byKey(const ValueKey('conversation-message-list')),
    );
    list.controller!.jumpTo(list.controller!.position.maxScrollExtent);
    await tester.enterText(find.byKey(const ValueKey('message-input')), '最新消息');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();

    expect(
      list.controller!.position.pixels,
      list.controller!.position.minScrollExtent,
    );
    expect(find.text('最新消息'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  test('source picker cancellation does not enqueue a transfer', () async {
    var sendCount = 0;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      sourcePicker: (_) async => null,
      sendSourceCommand: (_, _, _) => sendCount++,
    );
    addTearDown(controller.dispose);
    controller.selectConversation('private-1');

    expect(await controller.sendSource(MessageVisualKind.file), isFalse);
    expect(sendCount, 0);
  });

  test('selected source is passed to the Rust send command', () async {
    String? sentKind;
    String? sentPath;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-1',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      transferLoader: () => const [],
      sourcePicker: (_) async => r'C:\source\photo.jpg',
      sendSourceCommand: (conversationId, sourcePath, messageKind) {
        expect(conversationId, 'private-1');
        sentPath = sourcePath;
        sentKind = messageKind;
      },
    );
    addTearDown(controller.dispose);
    controller.selectConversation('private-1');

    expect(await controller.sendSource(MessageVisualKind.image), isTrue);
    expect(sentPath, r'C:\source\photo.jpg');
    expect(sentKind, 'image');
  });

  testWidgets('incoming own-device request can be accepted from nearby', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    String? acceptedBinding;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [],
      messageLoader: (_) => const [],
      ownDeviceBindingLoader: () => const [
        OwnDeviceBindingView(
          id: 'b_01900000-0000-7000-8000-000000000001',
          peerDeviceId: 'd_00000000000000000000000000000002',
          peerName: '测试手机',
          state: 'pending_inbound',
          clipboardMode: 'off',
          incoming: true,
          online: true,
          createdAtMs: 1,
        ),
      ],
      decideOwnDeviceBindingCommand: (bindingId, accept) {
        if (accept) acceptedBinding = bindingId;
      },
    );
    controller.setSection(SidebarSection.nearby);

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(find.byTooltip('接受'));
    await tester.pump();

    expect(acceptedBinding, 'b_01900000-0000-7000-8000-000000000001');
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('clipboard button previews and confirms text before sending', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    String? sentClipboard;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'd_00000000000000000000000000000002',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
          peerDeviceId: 'd_00000000000000000000000000000002',
        ),
      ],
      messageLoader: (_) => const [],
      clipboardReader: () async => '来自系统剪贴板',
      sendClipboardMessageCommand: (conversationId, text) {
        expect(conversationId, 'd_00000000000000000000000000000002');
        sentClipboard = text;
        return const ChatMessage(
          id: 'clipboard-1',
          content: '来自系统剪贴板',
          timeLabel: '刚刚',
          outgoing: true,
          kind: MessageVisualKind.clipboard,
          statusLabel: '正在发送',
        );
      },
    );
    controller.selectConversation('d_00000000000000000000000000000002');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(find.byTooltip('剪贴板'));
    await tester.pumpAndSettle();
    expect(find.text('来自系统剪贴板'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, '发送'));
    await tester.pumpAndSettle();

    expect(sentClipboard, '来自系统剪贴板');
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('Android notification action previews and confirms clipboard', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(420, 780));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final foregroundActions = StreamController<String>();
    addTearDown(foregroundActions.close);
    String? sentPeer;
    String? sentText;
    bool? sentAutomatically;

    await tester.pumpWidget(
      LanChatApp(
        coreRuntime: CoreRuntimeInfo.ready(
          version: 'test',
          protocolVersion: 1,
          deviceName: '测试手机',
          deviceId: 'd_00000000000000000000000000000001',
          discoveryAvailable: true,
        ),
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [],
        messageLoader: (_) => const [],
        ownDeviceBindingLoader: () => const [
          OwnDeviceBindingView(
            id: 'b_01900000-0000-7000-8000-000000000001',
            peerDeviceId: 'd_00000000000000000000000000000002',
            peerName: '测试电脑',
            state: 'active',
            clipboardMode: 'off',
            incoming: false,
            online: true,
            createdAtMs: 1,
          ),
        ],
        clipboardReader: () async => '通知里的剪贴板内容',
        submitClipboardTextCommand: (peerDeviceId, text, automatic) {
          sentPeer = peerDeviceId;
          sentText = text;
          sentAutomatically = automatic;
        },
        foregroundActions: foregroundActions.stream,
        shutdownCoreOnDispose: false,
      ),
    );

    foregroundActions.add('notificationSendClipboard');
    await tester.pumpAndSettle();
    expect(find.text('发送剪贴板到我的设备'), findsOneWidget);
    expect(find.text('通知里的剪贴板内容'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, '发送'));
    await tester.pumpAndSettle();

    expect(sentPeer, 'd_00000000000000000000000000000002');
    expect(sentText, '通知里的剪贴板内容');
    expect(sentAutomatically, isFalse);
  });

  testWidgets('tray exit confirms while a transfer is active', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final foregroundActions = StreamController<String>();
    addTearDown(foregroundActions.close);
    var exited = false;

    await tester.pumpWidget(
      LanChatApp(
        coreRuntime: CoreRuntimeInfo.ready(
          version: 'test',
          protocolVersion: 1,
          deviceName: '测试电脑',
          deviceId: 'd_00000000000000000000000000000001',
          discoveryAvailable: true,
        ),
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [],
        messageLoader: (_) => const [],
        transferLoader: () => [
          TransferView(
            id: 'transfer-active',
            messageId: 'message-active',
            peerDeviceId: 'd_00000000000000000000000000000002',
            direction: 'send',
            state: 'transferring',
            displayName: 'large.bin',
            totalSize: BigInt.from(1024 * 1024),
            entryCount: 1,
            persistedBytes: BigInt.from(512 * 1024),
            pausedByUser: false,
          ),
        ],
        foregroundActions: foregroundActions.stream,
        shutdownCoreOnDispose: false,
        exitApplicationCommand: () async => exited = true,
      ),
    );

    foregroundActions.add('trayExitRequested');
    await tester.pumpAndSettle();
    expect(find.text('仍有未完成的传输。退出后任务会保留，并在下次启动时恢复。'), findsOneWidget);
    expect(exited, isFalse);
    await tester.tap(find.widgetWithText(FilledButton, '退出'));
    await tester.pumpAndSettle();
    expect(exited, isTrue);
  });

  testWidgets('notification action opens its conversation', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final foregroundActions = StreamController<String>();
    addTearDown(foregroundActions.close);

    await tester.pumpWidget(
      LanChatApp(
        coreRuntime: CoreRuntimeInfo.ready(
          version: 'test',
          protocolVersion: 1,
          deviceName: '测试电脑',
          deviceId: 'd_00000000000000000000000000000001',
          discoveryAvailable: true,
        ),
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [
          ConversationSummary(
            id: 'first',
            title: '第一个会话',
            preview: 'first',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
          ),
          ConversationSummary(
            id: 'toast-target',
            title: '通知目标会话',
            preview: 'target',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
          ),
        ],
        messageLoader: (conversationId) => conversationId == 'toast-target'
            ? const [
                ChatMessage(
                  id: 'toast-message',
                  content: '由通知打开的消息',
                  timeLabel: '刚刚',
                  outgoing: false,
                  kind: MessageVisualKind.text,
                  statusLabel: '',
                ),
              ]
            : const [],
        foregroundActions: foregroundActions.stream,
        shutdownCoreOnDispose: false,
      ),
    );
    expect(find.text('由通知打开的消息'), findsNothing);

    foregroundActions.add('openConversation:toast-target');
    await tester.pumpAndSettle();
    expect(find.text('由通知打开的消息'), findsOneWidget);
  });

  testWidgets('cold notification activation opens its conversation', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    await tester.pumpWidget(
      LanChatApp(
        coreRuntime: CoreRuntimeInfo.ready(
          version: 'test',
          protocolVersion: 1,
          deviceName: '测试电脑',
          deviceId: 'd_00000000000000000000000000000001',
          discoveryAvailable: true,
        ),
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [
          ConversationSummary(
            id: 'first',
            title: '第一个会话',
            preview: 'first',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
          ),
          ConversationSummary(
            id: 'toast-target',
            title: '通知目标会话',
            preview: 'target',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
          ),
        ],
        messageLoader: (conversationId) => conversationId == 'toast-target'
            ? const [
                ChatMessage(
                  id: 'cold-toast-message',
                  content: '冷启动通知打开的消息',
                  timeLabel: '刚刚',
                  outgoing: false,
                  kind: MessageVisualKind.text,
                  statusLabel: '',
                ),
              ]
            : const [],
        initialForegroundAction: 'openConversation:toast-target',
        shutdownCoreOnDispose: false,
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('冷启动通知打开的消息'), findsOneWidget);
  });

  testWidgets('clipboard text can be confirmed and sent to a group', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    String? targetConversation;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'g:d_00000000000000000000000000000001:1',
          title: '家庭设备',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: true,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      clipboardReader: () async => '群剪贴板内容',
      sendClipboardMessageCommand: (conversationId, text) {
        targetConversation = conversationId;
        return ChatMessage(
          id: 'clipboard-group-1',
          content: text,
          timeLabel: '刚刚',
          outgoing: true,
          kind: MessageVisualKind.clipboard,
          statusLabel: '正在发送',
        );
      },
    );
    controller.selectConversation('g:d_00000000000000000000000000000001:1');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(find.byTooltip('剪贴板'));
    await tester.pumpAndSettle();
    expect(find.text('群剪贴板内容'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, '发送'));
    await tester.pumpAndSettle();

    expect(targetConversation, 'g:d_00000000000000000000000000000001:1');
    expect(find.text('群聊剪贴板发送将在后续版本开放'), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('clipboard image preview is preferred and can be sent', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    ClipboardImageSelection? sentImage;
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'private-image',
          title: '测试手机',
          preview: '开始聊天',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      messageLoader: (_) => const [],
      clipboardReader: () => throw StateError('不应读取文字'),
      clipboardImageReader: () async => ClipboardImageSelection(
        displayName: 'clipboard.png',
        sourceRef: r'C:\missing-preview.png',
        relativePath: 'clipboard.png',
        size: BigInt.from(1024),
        modifiedAtMs: 1,
        fingerprint:
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      ),
      sendClipboardImageCommand: (conversationId, image) {
        expect(conversationId, 'private-image');
        sentImage = image;
        return const ChatMessage(
          id: 'clipboard-image-1',
          content: 'clipboard.png',
          timeLabel: '刚刚',
          outgoing: true,
          kind: MessageVisualKind.image,
          statusLabel: '等待接收',
        );
      },
    );
    controller.selectConversation('private-image');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(find.byTooltip('剪贴板'));
    await tester.pumpAndSettle();
    expect(find.text('发送剪贴板图片'), findsOneWidget);
    expect(find.textContaining('clipboard.png'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, '发送'));
    await tester.pumpAndSettle();

    expect(sentImage?.displayName, 'clipboard.png');
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets(
    'completed image message renders a bounded asynchronous preview',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final directory = Directory.systemTemp.createTempSync(
        'lan-chat-preview-',
      );
      addTearDown(() => directory.deleteSync(recursive: true));
      final imageFile = File(
        '${directory.path}${Platform.pathSeparator}preview.png',
      );
      imageFile.writeAsBytesSync(
        base64Decode(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
        ),
      );
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [
          ConversationSummary(
            id: 'preview-conversation',
            title: '图片预览',
            preview: '[图片] preview.png',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
          ),
        ],
        messageLoader: (_) => [
          ChatMessage(
            id: 'preview-message',
            content: 'preview.png',
            timeLabel: '刚刚',
            outgoing: false,
            kind: MessageVisualKind.image,
            statusLabel: '已完成',
            fileSizeLabel: '1 项 · 68 B',
            progress: 1,
            localFileRef: imageFile.path,
          ),
        ],
      );
      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.windows,
          ),
        ),
      );
      await tester.pump();

      final image = tester.widget<Image>(find.byType(Image).first);
      expect((image.image as ResizeImage).width, 640);
      expect(
        tester
            .widgetList<LinearProgressIndicator>(
              find.byType(LinearProgressIndicator),
            )
            .any((indicator) => indicator.value == 1),
        isTrue,
      );
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      controller.dispose();
    },
  );

  test(
    'opening a conversation marks messages read through the visible order',
    () {
      String? markedConversationId;
      int? markedThroughSortOrder;
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [
          ConversationSummary(
            id: 'private-read',
            title: '测试手机',
            preview: '两条未读消息',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
            unreadCount: 2,
          ),
        ],
        messageLoader: (_) => const [
          ChatMessage(
            id: 'read-1',
            content: '第一条',
            timeLabel: '刚刚',
            outgoing: false,
            kind: MessageVisualKind.text,
            statusLabel: '已送达',
            localSortOrder: 7,
          ),
          ChatMessage(
            id: 'read-2',
            content: '第二条',
            timeLabel: '刚刚',
            outgoing: false,
            kind: MessageVisualKind.text,
            statusLabel: '已送达',
            localSortOrder: 11,
          ),
        ],
        markConversationReadCommand: (conversationId, throughSortOrder) {
          markedConversationId = conversationId;
          markedThroughSortOrder = throughSortOrder;
        },
      );
      addTearDown(controller.dispose);

      controller.selectConversation('private-read');

      expect(markedConversationId, 'private-read');
      expect(markedThroughSortOrder, 11);
      expect(controller.selectedConversation.unreadCount, 0);
    },
  );

  testWidgets(
    'private conversation deletion explains scope and returns empty',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1000, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      String? deletedConversationId;
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        conversationLoader: () => const [
          ConversationSummary(
            id: 'private-delete',
            title: '测试手机',
            preview: '待删除',
            timeLabel: '刚刚',
            isGroup: false,
            online: true,
            peerDeviceId: 'd_00000000000000000000000000000002',
          ),
        ],
        messageLoader: (_) => const [],
        deleteConversationCommand: (conversationId) {
          deletedConversationId = conversationId;
        },
      );
      controller.selectConversation('private-delete');

      await tester.pumpWidget(
        MaterialApp(
          home: LanChatHome(
            controller: controller,
            platform: TargetPlatform.windows,
          ),
        ),
      );
      await tester.tap(find.byTooltip('更多'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('delete-conversation')));
      await tester.pumpAndSettle();

      expect(find.textContaining('只会删除本机聊天记录'), findsOneWidget);
      expect(find.textContaining('已经保存到接收目录的文件不会删除'), findsOneWidget);
      await tester.tap(
        find.byKey(const ValueKey('confirm-delete-conversation')),
      );
      await tester.pumpAndSettle();

      expect(deletedConversationId, 'private-delete');
      expect(controller.conversations, isEmpty);
      expect(find.text('从附近设备开始聊天'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      controller.dispose();
    },
  );

  testWidgets('group delivery status opens per-member details', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    const conversationId = 'g:d_00000000000000000000000000000001:1';
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: conversationId,
          title: '家庭设备',
          preview: '群消息',
          timeLabel: '刚刚',
          isGroup: true,
          online: true,
          memberCount: 3,
        ),
      ],
      messageLoader: (_) => const [
        ChatMessage(
          id: 'group-message-1',
          content: '请查收',
          timeLabel: '刚刚',
          outgoing: true,
          kind: MessageVisualKind.text,
          statusLabel: '已送达 1/2',
          deliveredCount: 1,
          deliveryCount: 2,
        ),
      ],
      messageDeliveryLoader: (_) => const [
        MessageDeliveryView(
          recipientDeviceId: 'd_00000000000000000000000000000002',
          recipientName: '客厅手机',
          state: 'stored',
          updatedAtMs: 1,
          delivered: true,
        ),
        MessageDeliveryView(
          recipientDeviceId: 'd_00000000000000000000000000000003',
          recipientName: '书房平板',
          state: 'queued',
          updatedAtMs: 1,
          delivered: false,
        ),
      ],
    );
    controller.selectConversation(conversationId);

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(
      find.byKey(const ValueKey('message-delivery-group-message-1')),
    );
    await tester.pumpAndSettle();

    expect(find.text('送达详情'), findsOneWidget);
    expect(find.text('已送达 1/2'), findsWidgets);
    expect(find.text('客厅手机'), findsOneWidget);
    expect(find.text('书房平板'), findsOneWidget);
    expect(find.text('等待上线'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('group conversation more menu exposes details and deletion', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final controller = AppController(
      nearbyPeerLoader: () => const [],
      conversationLoader: () => const [
        ConversationSummary(
          id: 'g:d_00000000000000000000000000000001:1',
          title: '家庭设备',
          preview: '群消息',
          timeLabel: '刚刚',
          isGroup: true,
          online: true,
          memberCount: 3,
        ),
      ],
      messageLoader: (_) => const [],
      deleteConversationCommand: (_) {},
    );
    controller.selectConversation('g:d_00000000000000000000000000000001:1');

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );
    await tester.tap(find.byTooltip('更多'));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('group-details')), findsOneWidget);
    expect(find.byKey(const ValueKey('delete-conversation')), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
  });

  testWidgets('transfer filters clear records and jump to the source message', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1000, 700));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    var cleared = false;
    final completed = TransferView(
      id: 'completed-transfer',
      messageId: 'completed-message',
      conversationId: 'transfer-conversation',
      peerDeviceId: 'peer-1',
      direction: 'receive',
      state: 'completed',
      displayName: 'completed.pdf',
      totalSize: BigInt.from(100),
      entryCount: 1,
      persistedBytes: BigInt.from(100),
      pausedByUser: false,
      localFileRef: r'C:\Downloads\completed.pdf',
    );
    final waiting = TransferView(
      id: 'waiting-transfer',
      messageId: 'waiting-message',
      conversationId: 'transfer-conversation',
      peerDeviceId: 'peer-1',
      direction: 'send',
      state: 'queued',
      displayName: 'waiting.zip',
      totalSize: BigInt.from(200),
      entryCount: 1,
      persistedBytes: BigInt.zero,
      pausedByUser: false,
    );
    final controller = AppController(
      initialConversations: const [
        ConversationSummary(
          id: 'transfer-conversation',
          title: '客厅电脑',
          preview: 'waiting.zip',
          timeLabel: '刚刚',
          isGroup: false,
          online: true,
        ),
      ],
      initialMessages: const {
        'transfer-conversation': [
          ChatMessage(
            id: 'completed-message',
            content: 'completed.pdf',
            timeLabel: '刚刚',
            outgoing: false,
            kind: MessageVisualKind.file,
            statusLabel: '已完成',
          ),
          ChatMessage(
            id: 'waiting-message',
            content: 'waiting.zip',
            timeLabel: '刚刚',
            outgoing: true,
            kind: MessageVisualKind.file,
            statusLabel: '等待设备上线',
          ),
        ],
      },
      transferLoader: () => cleared ? [waiting] : [completed, waiting],
      clearCompletedTransfersCommand: () {
        cleared = true;
        return 1;
      },
    );
    controller.transfers = [completed, waiting];
    controller.showTransfers();
    addTearDown(controller.dispose);

    await tester.pumpWidget(
      MaterialApp(
        home: LanChatHome(
          controller: controller,
          platform: TargetPlatform.windows,
        ),
      ),
    );

    expect(find.text('没有进行中的传输'), findsOneWidget);
    await tester.tap(find.text('已完成'));
    await tester.pump();
    expect(find.text('completed.pdf'), findsOneWidget);
    expect(find.text('接收自 客厅电脑'), findsOneWidget);

    await tester.tap(find.byTooltip('清理已完成记录'));
    await tester.pumpAndSettle();
    expect(find.textContaining('不会删除'), findsOneWidget);
    await tester.tap(find.text('清理').last);
    await tester.pumpAndSettle();
    expect(cleared, isTrue);
    expect(find.text('completed.pdf'), findsNothing);

    await tester.tap(find.text('等待'));
    await tester.pump();
    await tester.tap(find.text('waiting.zip').last);
    await tester.pumpAndSettle();
    expect(controller.mainContent, MainContent.conversation);
    expect(controller.selectedConversationId, 'transfer-conversation');
    expect(find.text('waiting.zip'), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}
