import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/application/app_controller.dart';
import 'package:lan_chat/application/models.dart';
import 'package:lan_chat/presentation/device_avatar.dart';
import 'package:lan_chat/presentation/lan_chat_app.dart';

const _identities = [
  DeviceIdentityView(
    deviceId: 'self',
    deviceName: '我的电脑',
    avatarId: 'cat',
    isLocal: true,
  ),
  DeviceIdentityView(deviceId: 'phone', deviceName: '小米手机', avatarId: 'rocket'),
  DeviceIdentityView(
    deviceId: 'offline',
    deviceName: '离线的 K40',
    avatarId: 'rabbit',
  ),
];

const _settings = AppSettingsView(
  deviceName: '我的电脑',
  deviceId: 'self',
  platform: 'windows',
  defaultReceivePolicy: 'auto_accept',
  defaultReceiveRef: null,
  notificationsEnabled: false,
  closeToTray: true,
  startOnBoot: false,
  androidKeepOnline: true,
  logLevel: 'normal',
);

const _messages = [
  ChatMessage(
    id: 'incoming',
    senderDeviceId: 'phone',
    content: '照片已经整理好了',
    timeLabel: '10:01',
    outgoing: false,
    kind: MessageVisualKind.text,
    statusLabel: '',
  ),
  ChatMessage(
    id: 'offline-msg',
    senderDeviceId: 'offline',
    content: '这条消息来自离线设备',
    timeLabel: '10:02',
    outgoing: false,
    kind: MessageVisualKind.text,
    statusLabel: '',
  ),
  ChatMessage(
    id: 'outgoing',
    senderDeviceId: 'self',
    content: '收到了，稍后发给你',
    timeLabel: '10:03',
    outgoing: true,
    kind: MessageVisualKind.text,
    statusLabel: '已送达',
  ),
];

Future<AppController> _pumpChat(
  WidgetTester tester, {
  bool group = true,
  double width = 1000,
  List<DeviceIdentityView> identities = _identities,
  DeviceIdentityLoader? loader,
  DeviceAvatarUpdater? updater,
  GlobalKey? captureKey,
}) async {
  final previewFont = Platform.environment['NEKOSEND_AVATAR_PREVIEW_FONT'];
  if (previewFont != null) {
    await tester.runAsync(() async {
      await (FontLoader('AvatarPreview')..addFont(
            File(previewFont).readAsBytes().then(ByteData.sublistView),
          ))
          .load();
      await (FontLoader('packages/lucide_icons_flutter/Lucide')..addFont(
            rootBundle.load('packages/lucide_icons_flutter/assets/lucide.ttf'),
          ))
          .load();
    });
  }
  await tester.binding.setSurfaceSize(Size(width, 760));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final controller = AppController(
    initialConversations: [
      ConversationSummary(
        id: 'chat',
        title: group ? '家庭设备' : '小米手机',
        preview: '',
        timeLabel: '',
        isGroup: group,
        online: true,
        peerDeviceId: group ? null : 'phone',
      ),
    ],
    initialMessages: {
      'chat': group ? _messages : [_messages.first, _messages.last],
    },
    deviceIdentityLoader: loader ?? () => identities,
    deviceAvatarUpdater: updater,
  );
  controller.settings = _settings;
  controller.refreshAll();
  controller.selectConversation('chat');
  addTearDown(controller.dispose);
  await tester.pumpWidget(
    RepaintBoundary(
      key: captureKey,
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: ThemeData(
          fontFamily: previewFont == null ? null : 'AvatarPreview',
          colorSchemeSeed: const Color(0xFF087F72),
        ),
        home: LanChatHome(
          controller: controller,
          platform: width < 600
              ? TargetPlatform.android
              : TargetPlatform.windows,
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return controller;
}

Future<void> _capture(WidgetTester tester, GlobalKey key, String name) async {
  final directory = Platform.environment['NEKOSEND_CAPTURE_AVATARS'];
  if (directory == null) return;
  await tester.runAsync(() async {
    final boundary =
        key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    final image = await boundary.toImage(pixelRatio: 1);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    await Directory(directory).create(recursive: true);
    await File('$directory/$name.png')
        .writeAsBytes(bytes!.buffer.asUint8List());
    image.dispose();
  });
}

void main() {
  test(
    'built-in avatar IDs match rendering styles and stable defaults differ',
    () {
      expect(builtinAvatarStyles.keys.toList(), builtinAvatarIds);
      expect(defaultAvatarId('d_00000000000000000000000000000000'), 'cat');
      expect(defaultAvatarId('d_00000000000000000000000000000001'), 'dog');
      expect({
        for (var i = 0; i < 12; i++)
          defaultAvatarId('d_${i.toRadixString(16).padLeft(32, '0')}'),
      }, hasLength(12));
    },
  );

  for (final group in [false, true]) {
    testWidgets(
      '${group ? 'Group' : 'Private'} messages show each sender name and avatar',
      (tester) async {
        final capture = GlobalKey();
        await _pumpChat(tester, group: group, captureKey: capture);
        expect(
          tester
              .widget<Text>(
                find.byKey(const ValueKey('message-sender-incoming')),
              )
              .data,
          '小米手机',
        );
        expect(
          tester
              .widget<Text>(
                find.byKey(const ValueKey('message-sender-outgoing')),
              )
              .data,
          '我的电脑',
        );
        expect(
          tester
              .widget<DeviceAvatar>(
                find.byKey(const ValueKey('message-avatar-incoming')),
              )
              .avatarId,
          'rocket',
        );
        expect(
          tester
              .widget<DeviceAvatar>(
                find.byKey(const ValueKey('message-avatar-outgoing')),
              )
              .avatarId,
          'cat',
        );
        if (group) {
          expect(
            tester
                .widget<Text>(
                  find.byKey(const ValueKey('message-sender-offline-msg')),
                )
                .data,
            '离线的 K40',
          );
          expect(
            tester
                .widget<DeviceAvatar>(
                  find.byKey(const ValueKey('message-avatar-offline-msg')),
                )
                .avatarId,
            'rabbit',
          );
        }
        expect(tester.takeException(), isNull);
        await _capture(
          tester,
          capture,
          group ? 'group-desktop' : 'private-desktop',
        );
      },
    );
  }

  testWidgets(
    'mobile long nicknames and unknown avatars stay within the message column',
    (tester) async {
      final capture = GlobalKey();
      await _pumpChat(
        tester,
        width: 360,
        captureKey: capture,
        identities: [
          _identities.first,
          DeviceIdentityView(
            deviceId: 'phone',
            deviceName: List.filled(32, '名').join(),
            avatarId: 'future-avatar',
          ),
          _identities.last,
        ],
      );
      final nickname = tester.widget<Text>(
        find.byKey(const ValueKey('message-sender-incoming')),
      );
      expect(nickname.maxLines, 1);
      expect(nickname.overflow, TextOverflow.ellipsis);
      for (final id in ['incoming', 'offline-msg', 'outgoing']) {
        final rect = tester.getRect(find.byKey(ValueKey('message-avatar-$id')));
        expect(rect.left, greaterThanOrEqualTo(0));
        expect(rect.right, lessThanOrEqualTo(360));
      }
      expect(tester.takeException(), isNull);
      await _capture(tester, capture, 'group-mobile');
    },
  );

  testWidgets(
    'saved avatar updates visible history; cancelled and failed selection do not',
    (tester) async {
      var identities = _identities.toList();
      final saved = <String>[];
      var fail = false;
      final capture = GlobalKey();
      final controller = await _pumpChat(
        tester,
        captureKey: capture,
        loader: () => identities,
        updater: (id) {
          if (fail) throw StateError('写入失败');
          saved.add(id);
          identities = [
            DeviceIdentityView(
              deviceId: 'self',
              deviceName: '我的电脑',
              avatarId: id,
              isLocal: true,
            ),
            ..._identities.skip(1),
          ];
        },
      );
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('device-avatar-setting')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('builtin-avatar-grid')), findsOneWidget);
      for (final id in builtinAvatarIds) {
        expect(find.byKey(ValueKey('avatar-option-$id')), findsOneWidget);
      }
      await _capture(tester, capture, 'avatar-picker');
      await tester.tap(find.byKey(const ValueKey('avatar-option-dog')));
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(saved, isEmpty);
      await tester.tap(find.byKey(const ValueKey('device-avatar-setting')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('avatar-option-rocket')));
      fail = true;
      await tester.tap(find.text('保存'));
      await tester.pumpAndSettle();
      expect(controller.localIdentity.avatarId, 'cat');
      expect(find.byKey(const ValueKey('builtin-avatar-grid')), findsOneWidget);
      fail = false;
      await tester.tap(find.text('保存'));
      await tester.pumpAndSettle();
      expect(saved, ['rocket']);
      expect(controller.localIdentity.avatarId, 'rocket');
      await tester.tap(find.text('完成'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<DeviceAvatar>(
              find.byKey(const ValueKey('message-avatar-outgoing')),
            )
            .avatarId,
        'rocket',
      );
    },
  );

  testWidgets(
    'presence events refresh cached sender profiles without losing messages',
    (tester) async {
      var identities = _identities.toList();
      final events = StreamController<CoreEventUpdate>();
      final controller = AppController(
        nearbyPeerLoader: () => const [],
        initialConversations: const [
          ConversationSummary(
            id: 'chat',
            title: '群聊',
            preview: '',
            timeLabel: '',
            isGroup: true,
            online: true,
          ),
        ],
        initialMessages: const {'chat': _messages},
        coreEventStreamFactory: () => events.stream,
        deviceIdentityLoader: () => identities,
      );
      try {
        identities = [
          _identities.first,
          const DeviceIdentityView(
            deviceId: 'phone',
            deviceName: '新昵称',
            avatarId: 'moon',
          ),
          _identities.last,
        ];
        events.add(const CoreEventUpdate(CoreEventArea.presence));
        await tester.pump();
        expect(controller.senderIdentity(_messages.first).deviceName, '新昵称');
        expect(controller.senderIdentity(_messages.first).avatarId, 'moon');
        expect(controller.selectedMessages, hasLength(3));
        expect(controller.senderIdentity(_messages[1]).deviceName, '离线的 K40');
      } finally {
        controller.dispose();
        unawaited(events.close());
        await tester.pump();
      }
    },
  );
}
