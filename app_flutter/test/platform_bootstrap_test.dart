import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/platform/platform_bootstrap.dart';

void main() {
  testWidgets(
    'Linux bootstrap uses the native XDG directory and a distinct platform',
    (tester) async {
      const channel = MethodChannel('dev.lanchat/platform');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        expect(call.method, 'getBootstrapInfo');
        return {
          'dataDirectory':
              '/home/test/.var/app/io.github.iris_neko.NekoSend/data/NekoSend',
          'deviceName': 'Linux workstation',
        };
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      final bootstrap = await PlatformBootstrap.load();
      expect(bootstrap.platform, 'linux');
      expect(bootstrap.deviceName, 'Linux workstation');
      expect(
        bootstrap.dataDirectory,
        contains('/.var/app/io.github.iris_neko.NekoSend/'),
      );
      expect(bootstrap.legacyDeviceName, isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  test('legacy Android model code upgrades to a readable default', () {
    const bootstrap = PlatformBootstrap(
      dataDirectory: '/data/app',
      deviceName: 'Xiaomi 14',
      platform: 'android',
      legacyDeviceName: '23127PN0CC',
    );
    expect(bootstrap.defaultNameUpgradeFor('23127PN0CC'), 'Xiaomi 14');
  });

  test('default name upgrade preserves custom and already upgraded names', () {
    const bootstrap = PlatformBootstrap(
      dataDirectory: '/data/app',
      deviceName: 'Xiaomi 14',
      platform: 'android',
      legacyDeviceName: '23127PN0CC',
    );
    expect(bootstrap.defaultNameUpgradeFor('我的手机'), isNull);
    expect(bootstrap.defaultNameUpgradeFor('Xiaomi 14'), isNull);
  });

  test('completed migration never overrides a later model-code nickname', () {
    const bootstrap = PlatformBootstrap(
      dataDirectory: '/data/app',
      deviceName: 'Xiaomi 14',
      platform: 'android',
    );
    expect(bootstrap.defaultNameUpgradeFor('23127PN0CC'), isNull);
  });

  test('Android default name upgrade never changes Windows names', () {
    const bootstrap = PlatformBootstrap(
      dataDirectory: 'C:/Data',
      deviceName: 'Desktop',
      platform: 'windows',
      legacyDeviceName: 'Old desktop',
    );
    expect(bootstrap.defaultNameUpgradeFor('Old desktop'), isNull);
  });

  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
  });

  test(
    'Windows clipboard contention retries with the fixed backoff budget',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      var attempts = 0;

      final result = await PlatformBootstrap.retryWindowsClipboard(() async {
        attempts++;
        if (attempts < 5) {
          throw PlatformException(code: 'CLIPBOARD_BUSY');
        }
        return 'available';
      });

      expect(result, 'available');
      expect(attempts, 5);
    },
  );

  test('Windows clipboard retry returns the final platform error', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    var attempts = 0;

    await expectLater(
      PlatformBootstrap.retryWindowsClipboard<void>(() async {
        attempts++;
        throw PlatformException(code: 'CLIPBOARD_BUSY');
      }),
      throwsA(isA<PlatformException>()),
    );
    expect(attempts, 5);
  });

  test('non-Windows clipboard operation is not retried', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var attempts = 0;

    await expectLater(
      PlatformBootstrap.retryWindowsClipboard<void>(() async {
        attempts++;
        throw PlatformException(code: 'CLIPBOARD_BUSY');
      }),
      throwsA(isA<PlatformException>()),
    );
    expect(attempts, 1);
  });

  test('failed Rust FD handoff closes the detached descriptor once', () async {
    var closeCount = 0;
    int? closedFd;

    await expectLater(
      PlatformBootstrap.handOffDocumentFd(
        open: () async => 41,
        complete: (_) => throw StateError('FFI handoff failed'),
        close: (fd) async {
          closeCount++;
          closedFd = fd;
        },
      ),
      throwsStateError,
    );

    expect(closeCount, 1);
    expect(closedFd, 41);
  });

  test(
    'successful Rust FD handoff transfers ownership without closing',
    () async {
      var closeCount = 0;
      int? completedFd;

      await PlatformBootstrap.handOffDocumentFd(
        open: () async => 73,
        complete: (fd) => completedFd = fd,
        close: (_) async => closeCount++,
      );

      expect(completedFd, 73);
      expect(closeCount, 0);
    },
  );
}
