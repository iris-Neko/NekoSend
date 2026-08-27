import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/platform/platform_bootstrap.dart';

void main() {
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
