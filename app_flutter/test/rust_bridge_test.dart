import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/src/rust/api/core.dart';
import 'package:lan_chat/src/rust/api/health.dart';
import 'package:lan_chat/src/rust/events.dart';
import 'package:lan_chat/src/rust/frb_generated.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory tempDirectory;
  setUpAll(() async {
    await RustLib.init();
    tempDirectory = await Directory.systemTemp.createTemp('lan_chat_test_');
  });
  tearDownAll(() async {
    shutdownCore();
    RustLib.dispose();
    await tempDirectory.delete(recursive: true);
  });

  test('Flutter loads the Rust core and reads protocol health', () {
    final health = getCoreHealth();

    expect(health.status, 'ready');
    expect(health.coreVersion, '0.3.0');
    expect(health.protocolVersion, 1);
  });

  test('Flutter starts SQLite core without changing its device id', () {
    final databasePath =
        '${tempDirectory.path}${Platform.pathSeparator}lan_chat.db';
    final first = startCore(
      databasePath: databasePath,
      deviceName: '测试电脑',
      platform: 'windows',
      enableDiscovery: false,
    );
    shutdownCore();
    final second = startCore(
      databasePath: databasePath,
      deviceName: '更新后的名称',
      platform: 'windows',
      enableDiscovery: false,
    );

    expect(second.deviceId, first.deviceId);
    expect(second.deviceName, '测试电脑');
    expect(
      updateDeviceName(
        clientOperationId: generateClientOperationId(),
        deviceName: '更新后的名称',
      ).deviceName,
      '更新后的名称',
    );
    expect(second.discoveryAvailable, isFalse);
    expect(File(databasePath).existsSync(), isTrue);
  });

  test('Flutter persists avatars and rejects unsupported IDs', () {
    final local = listDeviceIdentities().singleWhere((item) => item.isLocal);
    expect(local.deviceName, '更新后的名称');
    expect(
      setDeviceAvatar(
        clientOperationId: generateClientOperationId(),
        avatarId: 'rocket',
      ),
      'rocket',
    );
    final updated = listDeviceIdentities().singleWhere((item) => item.isLocal);
    expect(updated.avatarId, 'rocket');
    expect(updated.deviceId, local.deviceId);
    expect(
      () => setDeviceAvatar(
        clientOperationId: generateClientOperationId(),
        avatarId: 'unknown',
      ),
      throwsA(anything),
    );
    expect(
      listDeviceIdentities().singleWhere((item) => item.isLocal).avatarId,
      'rocket',
    );
  });

  test('Flutter reads and updates persisted application settings', () {
    final initial = getAppSettings();
    expect(initial.defaultReceivePolicy, 'auto_accept');
    expect(initial.closeToTray, isTrue);

    final updated = updateAppSettings(
      clientOperationId: generateClientOperationId(),
      defaultReceivePolicy: 'ask_every_time',
      defaultReceiveRef: 'D:/Receive',
      clearDefaultReceiveRef: false,
      notificationsEnabled: false,
      closeToTray: false,
      startOnBoot: true,
      androidKeepOnline: false,
      logLevel: 'debug',
    );
    expect(updated.defaultReceivePolicy, 'ask_every_time');
    expect(updated.defaultReceiveRef, 'D:/Receive');
    expect(updated.notificationsEnabled, isFalse);
    expect(updated.closeToTray, isFalse);
    expect(updated.startOnBoot, isTrue);
    expect(updated.androidKeepOnline, isFalse);
    expect(updated.logLevel, 'debug');
    expect(getAppSettings(), updated);
  });

  test(
    'Rust event stream invalidates the Flutter snapshot after a commit',
    () async {
      final events = subscribeCoreEvents().asBroadcastStream();
      await events
          .firstWhere((event) => event.kind == CoreEventKind.coreReady)
          .timeout(const Duration(seconds: 2));
      final invalidated = events
          .firstWhere(
            (event) => event.kind == CoreEventKind.appSnapshotInvalidated,
          )
          .timeout(const Duration(seconds: 2));

      updateAppSettings(
        clientOperationId: generateClientOperationId(),
        logLevel: 'normal',
        clearDefaultReceiveRef: false,
      );

      expect((await invalidated).kind, CoreEventKind.appSnapshotInvalidated);
    },
  );
}
