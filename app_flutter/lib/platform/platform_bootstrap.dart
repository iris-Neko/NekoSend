import 'dart:convert';
import 'dart:io';
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../src/rust/api/core.dart';

class PlatformBootstrap {
  static const _channel = MethodChannel('dev.lanchat/platform');
  static final _clipboardChanges = StreamController<void>.broadcast();
  static final _trayActions = StreamController<String>.broadcast();
  static final _foregroundActions = StreamController<String>.broadcast();
  static final _networkChanges = StreamController<void>.broadcast();
  static bool _channelHandlerInstalled = false;
  static bool _androidCoreStopped = false;
  static String? _suppressedClipboardText;

  const PlatformBootstrap({
    required this.dataDirectory,
    required this.deviceName,
    required this.platform,
    this.legacyDeviceName,
  });

  final String dataDirectory;
  final String deviceName;
  final String platform;
  final String? legacyDeviceName;

  String? defaultNameUpgradeFor(String currentName) {
    if (platform != 'android' ||
        legacyDeviceName == null ||
        currentName != legacyDeviceName ||
        currentName == deviceName) {
      return null;
    }
    return deviceName;
  }

  static Future<void> completeDeviceNameMigration() =>
      _channel.invokeMethod<void>('completeDeviceNameMigration');

  String get databasePath =>
      '$dataDirectory${Platform.pathSeparator}lan_chat.db';

  static Future<PlatformBootstrap> load() async {
    _installChannelHandler();
    if (defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.linux) {
      final result = await _channel.invokeMapMethod<String, String>(
        'getBootstrapInfo',
      );
      final dataDirectory = result?['dataDirectory'];
      final deviceName = result?['deviceName'];
      if (dataDirectory == null || deviceName == null) {
        throw StateError('Platform bootstrap returned incomplete data');
      }
      return PlatformBootstrap(
        dataDirectory: dataDirectory,
        deviceName: _normalizeName(deviceName),
        platform: defaultTargetPlatform == TargetPlatform.android
            ? 'android'
            : 'linux',
        legacyDeviceName: result?['legacyDeviceName'] == null
            ? null
            : _normalizeName(result!['legacyDeviceName']!),
      );
    }

    final localAppData = Platform.environment['LOCALAPPDATA'];
    if (localAppData == null || localAppData.isEmpty) {
      throw StateError('LOCALAPPDATA is unavailable');
    }
    final dataDirectory = '$localAppData${Platform.pathSeparator}LAN Chat';
    await Directory(dataDirectory).create(recursive: true);
    return PlatformBootstrap(
      dataDirectory: dataDirectory,
      deviceName: _normalizeName(Platform.localHostname),
      platform: 'windows',
    );
  }

  static Future<String?> pickSource(String kind) =>
      _channel.invokeMethod<String>('pickSource', {'kind': kind});

  static Future<String?> resolveImagePreview(String? reference) async {
    if (reference == null || reference.isEmpty) return null;
    if (!reference.startsWith('content://')) return reference;
    if (defaultTargetPlatform != TargetPlatform.android) return null;
    return _channel.invokeMethod<String>('cacheImagePreview', {
      'uri': reference,
    });
  }

  static Future<void> openReference(
    String reference, {
    required bool showInFolder,
  }) => _channel.invokeMethod<void>('openReference', {
    'reference': reference,
    'showInFolder': showInFolder,
  });

  static Future<String?> readClipboardText() async {
    final data = await retryWindowsClipboard(
      () => Clipboard.getData(Clipboard.kTextPlain),
    );
    return data?.text;
  }

  static Future<void> writeClipboardText(
    String text, {
    bool suppressNotification = false,
  }) async {
    if (suppressNotification) _suppressedClipboardText = text;
    await retryWindowsClipboard(
      () => Clipboard.setData(ClipboardData(text: text)),
    );
  }

  static Future<SafPickedSource?> readClipboardImage() async {
    final value = await retryWindowsClipboard(
      () => _channel.invokeMapMethod<String, dynamic>('readClipboardImage'),
    );
    if (value == null) return null;
    final size = BigInt.from(value['size'] as num);
    if (size <= BigInt.zero || size > BigInt.from(20 * 1024 * 1024)) {
      throw StateError('剪贴板图片必须在 20 MiB 以内');
    }
    return SafPickedSource(
      displayName: value['displayName'] as String,
      fingerprint: value['fingerprint'] as String?,
      suppressSync: value['suppressSync'] == true,
      sources: [
        SourceItemDto(
          entryKind: 'file',
          sourceRef: value['sourceRef'] as String,
          relativePath: value['relativePath'] as String,
          size: size,
          modifiedAtMs: value['modifiedAtMs'] as int,
        ),
      ],
    );
  }

  static Future<void> writeClipboardImage(
    String reference,
    String metadataJson,
  ) async {
    final metadata = jsonDecode(metadataJson) as Map<String, dynamic>;
    await retryWindowsClipboard(
      () => _channel.invokeMethod<void>('writeClipboardImage', {
        'reference': reference,
        'metadata': metadataJson,
        'fingerprint': metadata['content_fingerprint'] as String,
      }),
    );
  }

  @visibleForTesting
  static Future<T> retryWindowsClipboard<T>(
    Future<T> Function() operation,
  ) async {
    if (defaultTargetPlatform != TargetPlatform.windows) return operation();
    const delays = [
      Duration.zero,
      Duration(milliseconds: 50),
      Duration(milliseconds: 100),
      Duration(milliseconds: 200),
      Duration(milliseconds: 400),
    ];
    for (var attempt = 0; attempt < delays.length; attempt++) {
      if (delays[attempt] > Duration.zero) {
        await Future<void>.delayed(delays[attempt]);
      }
      try {
        return await operation();
      } on PlatformException {
        if (attempt == delays.length - 1) rethrow;
      }
    }
    throw StateError('unreachable clipboard retry state');
  }

  @visibleForTesting
  static Future<void> handOffDocumentFd({
    required Future<int?> Function() open,
    required void Function(int? fd) complete,
    required Future<void> Function(int fd) close,
  }) async {
    int? detachedFd;
    try {
      detachedFd = await open();
      complete(detachedFd);
      detachedFd = null;
    } catch (_) {
      if (detachedFd != null) {
        try {
          await close(detachedFd);
        } catch (_) {}
      }
      rethrow;
    }
  }

  static Stream<void> get clipboardChanges => _clipboardChanges.stream;

  static bool consumeSuppressedClipboardText(String text) {
    if (_suppressedClipboardText != text) return false;
    _suppressedClipboardText = null;
    return true;
  }

  static void _installChannelHandler() {
    if (_channelHandlerInstalled) return;
    _channelHandlerInstalled = true;
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'clipboardChanged') {
        _clipboardChanges.add(null);
      } else if (call.method == 'traySendClipboard' ||
          call.method == 'trayPauseAllTransfers') {
        _trayActions.add(call.method);
      } else if (call.method == 'androidStopOnline') {
        _androidCoreStopped = true;
        _trayActions.add(call.method);
      } else if (call.method == 'androidSystemOnlineTimeout') {
        _androidCoreStopped = true;
        _trayActions.add(call.method);
      } else if (call.method == 'appResumed' && _androidCoreStopped) {
        _androidCoreStopped = false;
        _networkChanges.add(null);
      } else if (call.method == 'notificationSendClipboard') {
        _foregroundActions.add(call.method);
      } else if (call.method == 'trayExitRequested' ||
          call.method == 'windowExitRequested') {
        _foregroundActions.add(call.method);
      } else if (call.method == 'notificationOpenConversation') {
        final conversationId = call.arguments as String?;
        if (conversationId != null) {
          _foregroundActions.add('openConversation:$conversationId');
        }
      } else if (call.method == 'networkChanged') {
        _networkChanges.add(null);
      }
    });
  }

  static Future<SafPickedSource?> pickSafSource(String kind) async {
    final value = await _channel.invokeMapMethod<String, dynamic>(
      'pickSource',
      {'kind': kind},
    );
    if (value == null) return null;
    final rawSources = value['sources'] as List<dynamic>?;
    if (rawSources == null) {
      throw StateError('Android source picker returned no manifest entries');
    }
    return SafPickedSource(
      displayName: value['displayName'] as String,
      sources: rawSources
          .map((raw) {
            final item = Map<String, dynamic>.from(raw as Map);
            return SourceItemDto(
              entryKind: item['entryKind'] as String,
              sourceRef: item['sourceRef'] as String?,
              relativePath: item['relativePath'] as String,
              size: BigInt.from(item['size'] as num),
              modifiedAtMs: item['modifiedAtMs'] as int,
            );
          })
          .toList(growable: false),
    );
  }

  static Future<SafPreparedReceive?> prepareSafReceive(
    List<TransferEntryDto> entries,
  ) async {
    final savedTreeUri = await _channel.invokeMethod<String>(
      'getSavedReceiveTree',
    );
    if (savedTreeUri != null) {
      try {
        return await _prepareSafReceiveTree(savedTreeUri, entries);
      } catch (_) {
        await _channel.invokeMethod<void>('clearSavedReceiveTree');
      }
    }
    final selected = await _channel.invokeMapMethod<String, dynamic>(
      'pickReceiveDirectory',
    );
    if (selected == null) return null;
    return _prepareSafReceiveTree(selected['treeUri'] as String, entries);
  }

  static Future<SafPreparedReceive?> pickSafReceive(
    List<TransferEntryDto> entries,
  ) async {
    final selected = await _channel.invokeMapMethod<String, dynamic>(
      'pickReceiveDirectory',
    );
    if (selected == null) return null;
    return _prepareSafReceiveTree(selected['treeUri'] as String, entries);
  }

  static Future<String?> pickReceiveDirectory() async {
    final selected = await _channel.invokeMapMethod<String, dynamic>(
      'pickReceiveDirectory',
    );
    return selected?['treeUri'] as String?;
  }

  static Future<SafPreparedReceive> prepareSafReceiveAt(
    String treeUri,
    List<TransferEntryDto> entries,
  ) => _prepareSafReceiveTree(treeUri, entries);

  static Future<bool> requestNotificationPermission() async {
    if (defaultTargetPlatform != TargetPlatform.android) return true;
    final granted = await _channel.invokeMethod<bool>(
      'requestNotificationPermission',
    );
    if (granted == true) return true;
    await _channel.invokeMethod<void>('openNotificationSettings');
    return false;
  }

  static Future<SafPreparedReceive> _prepareSafReceiveTree(
    String treeUri,
    List<TransferEntryDto> entries,
  ) async {
    final value = await _channel.invokeMapMethod<String, dynamic>(
      'prepareReceiveTree',
      {
        'treeUri': treeUri,
        'entries': entries
            .map(
              (entry) => {
                'entryId': entry.entryId,
                'entryKind': entry.entryKind,
                'relativePath': entry.relativePath,
                'size': entry.size.toInt(),
                'persistedOffset': entry.persistedOffset.toInt(),
              },
            )
            .toList(growable: false),
      },
    );
    if (value == null) {
      throw StateError('Android receive tree preparation returned no result');
    }
    final prepared = (value['prepared'] as List<dynamic>)
        .map((raw) {
          final item = Map<String, dynamic>.from(raw as Map);
          return PreparedReceiveEntryDto(
            entryId: item['entryId'] as String,
            destinationRef: item['destinationRef'] as String,
            partialRef: item['partialRef'] as String?,
            persistedOffset: BigInt.from(item['persistedOffset'] as num),
          );
        })
        .toList(growable: false);
    return SafPreparedReceive(
      receiveBaseRef: value['receiveBaseRef'] as String,
      prepared: prepared,
    );
  }

  static String _normalizeName(String value) {
    final trimmed = value.trim();
    if (trimmed.isEmpty) return '猫猫快传设备';
    return String.fromCharCodes(trimmed.runes.take(32));
  }

  static Stream<String> get trayActions => _trayActions.stream;

  static Stream<String> get foregroundActions => _foregroundActions.stream;

  static Stream<void> get networkChanges => _networkChanges.stream;

  static Future<String?> getSavedReceiveTree() =>
      _channel.invokeMethod<String>('getSavedReceiveTree');

  static Future<String?> getDefaultReceiveDirectory() =>
      _channel.invokeMethod<String>('getDefaultReceiveDirectory');

  static Future<void> applyAppSettings({
    required bool notificationsEnabled,
    required bool closeToTray,
    required bool startOnBoot,
    required bool androidKeepOnline,
  }) => _channel.invokeMethod<void>('applyAppSettings', {
    'notificationsEnabled': notificationsEnabled,
    'closeToTray': closeToTray,
    'startOnBoot': startOnBoot,
    'androidKeepOnline': androidKeepOnline,
  });

  static Future<void> updateActiveTransferCount(int count) {
    if (defaultTargetPlatform != TargetPlatform.android &&
        defaultTargetPlatform != TargetPlatform.linux) {
      return Future<void>.value();
    }
    return _channel.invokeMethod<void>('updateActiveTransferCount', {
      'count': count,
    });
  }

  static Future<void> exitApplication() =>
      _channel.invokeMethod<void>('exitApplication');

  static Future<void> moveAndroidTaskToBackground() =>
      _channel.invokeMethod<void>('moveToBackground');

  static Future<void> showSystemNotification({
    required String title,
    required String body,
    String? conversationId,
  }) => _channel.invokeMethod<void>('showNotification', {
    'title': title,
    'body': body,
    'conversationId': conversationId,
  });
}

class SafPickedSource {
  const SafPickedSource({
    required this.displayName,
    required this.sources,
    this.fingerprint,
    this.suppressSync = false,
  });

  final String displayName;
  final List<SourceItemDto> sources;
  final String? fingerprint;
  final bool suppressSync;
}

class SafPreparedReceive {
  const SafPreparedReceive({
    required this.receiveBaseRef,
    required this.prepared,
  });

  final String receiveBaseRef;
  final List<PreparedReceiveEntryDto> prepared;
}

class PlatformRequestPump {
  PlatformRequestPump._(
    this._onLocalClipboardText,
    this._onLocalClipboardImage,
    this._onTraySendClipboard,
    this._onTraySendClipboardImage,
    this._onPauseAllTransfers,
    this._onStopAndroidOnline,
    this._onSystemAndroidOnlineTimeout,
    this._onNetworkChanged,
  ) {
    _timer = Timer.periodic(const Duration(milliseconds: 25), (_) => _poll());
    _clipboardSubscription = PlatformBootstrap.clipboardChanges.listen(
      (_) => _clipboardChanged(),
    );
    _traySubscription = PlatformBootstrap.trayActions.listen(_trayAction);
    _networkSubscription = PlatformBootstrap.networkChanges.listen(
      (_) => _networkChanged(),
    );
  }

  static PlatformRequestPump start({
    required Future<void> Function(String text) onLocalClipboardText,
    required Future<void> Function(SafPickedSource image) onLocalClipboardImage,
    required Future<void> Function(String text) onTraySendClipboard,
    required Future<void> Function(SafPickedSource image)
    onTraySendClipboardImage,
    required Future<void> Function() onPauseAllTransfers,
    required Future<void> Function() onStopAndroidOnline,
    required Future<void> Function() onSystemAndroidOnlineTimeout,
    required Future<void> Function() onNetworkChanged,
  }) => PlatformRequestPump._(
    onLocalClipboardText,
    onLocalClipboardImage,
    onTraySendClipboard,
    onTraySendClipboardImage,
    onPauseAllTransfers,
    onStopAndroidOnline,
    onSystemAndroidOnlineTimeout,
    onNetworkChanged,
  );

  final Future<void> Function(String text) _onLocalClipboardText;
  final Future<void> Function(SafPickedSource image) _onLocalClipboardImage;
  final Future<void> Function(String text) _onTraySendClipboard;
  final Future<void> Function(SafPickedSource image) _onTraySendClipboardImage;
  final Future<void> Function() _onPauseAllTransfers;
  final Future<void> Function() _onStopAndroidOnline;
  final Future<void> Function() _onSystemAndroidOnlineTimeout;
  final Future<void> Function() _onNetworkChanged;
  late final Timer _timer;
  late final StreamSubscription<void> _clipboardSubscription;
  late final StreamSubscription<String> _traySubscription;
  late final StreamSubscription<void> _networkSubscription;
  Timer? _networkDebounce;
  bool _polling = false;
  bool _readingClipboard = false;
  String? _lastObservedClipboardText;
  String? _lastObservedClipboardImageFingerprint;

  Future<void> _clipboardChanged() async {
    if (_readingClipboard) return;
    _readingClipboard = true;
    try {
      final image = await PlatformBootstrap.readClipboardImage();
      if (image != null) {
        if (image.suppressSync) {
          _lastObservedClipboardImageFingerprint = image.fingerprint;
          return;
        }
        if (image.fingerprint != null &&
            _lastObservedClipboardImageFingerprint == image.fingerprint) {
          return;
        }
        _lastObservedClipboardImageFingerprint = image.fingerprint;
        await _onLocalClipboardImage(image);
        return;
      }
      final text = await PlatformBootstrap.readClipboardText();
      if (text == null || text.isEmpty) return;
      if (PlatformBootstrap.consumeSuppressedClipboardText(text)) {
        _lastObservedClipboardText = text;
        return;
      }
      if (_lastObservedClipboardText == text) return;
      _lastObservedClipboardText = text;
      await _onLocalClipboardText(text);
    } finally {
      _readingClipboard = false;
    }
  }

  Future<void> _trayAction(String action) async {
    if (action == 'trayPauseAllTransfers') {
      await _onPauseAllTransfers();
      return;
    }
    if (action == 'androidStopOnline') {
      await _onStopAndroidOnline();
      return;
    }
    if (action == 'androidSystemOnlineTimeout') {
      await _onSystemAndroidOnlineTimeout();
      return;
    }
    if (action == 'traySendClipboard') {
      final image = await PlatformBootstrap.readClipboardImage();
      if (image != null) {
        await _onTraySendClipboardImage(image);
        return;
      }
      final text = await PlatformBootstrap.readClipboardText();
      if (text != null && text.isNotEmpty) {
        await _onTraySendClipboard(text);
      }
    }
  }

  void _networkChanged() {
    _networkDebounce?.cancel();
    _networkDebounce = Timer(const Duration(milliseconds: 600), () {
      _onNetworkChanged();
    });
  }

  Future<void> _poll() async {
    if (_polling) return;
    _polling = true;
    try {
      for (final request in pollPlatformRequests()) {
        try {
          if (request.operation == 'write_clipboard_text') {
            await PlatformBootstrap.writeClipboardText(
              request.uri,
              suppressNotification: true,
            );
          } else if (request.operation == 'write_clipboard_image') {
            await PlatformBootstrap.writeClipboardImage(
              request.uri,
              request.finalName!,
            );
          } else if (request.operation == 'prepare_receive_tree') {
            final entries = jsonDecode(request.finalName!) as List<dynamic>;
            final response = await PlatformBootstrap._channel
                .invokeMapMethod<String, dynamic>('prepareReceiveTree', {
                  'treeUri': request.uri,
                  'entries': entries,
                });
            completePlatformRequest(
              requestId: request.requestId,
              value: jsonEncode(response),
            );
          } else if (request.operation == 'commit_receive_file') {
            final response = await PlatformBootstrap._channel
                .invokeMapMethod<String, dynamic>('commitReceiveFile', {
                  'uri': request.uri,
                  'finalName': request.finalName,
                });
            completePlatformRequest(
              requestId: request.requestId,
              value: response?['uri'] as String?,
            );
          } else {
            await PlatformBootstrap.handOffDocumentFd(
              open: () async {
                final response = await PlatformBootstrap._channel
                    .invokeMapMethod<String, dynamic>('openDocumentFd', {
                      'uri': request.uri,
                      'writable': request.operation == 'open_receive_file',
                    });
                return response?['fd'] as int?;
              },
              complete: (fd) =>
                  completePlatformRequest(requestId: request.requestId, fd: fd),
              close: (fd) => PlatformBootstrap._channel.invokeMethod<void>(
                'closeRawFd',
                {'fd': fd},
              ),
            );
          }
        } catch (error) {
          try {
            completePlatformRequest(
              requestId: request.requestId,
              error: error.toString(),
            );
          } catch (_) {}
        }
      }
    } finally {
      _polling = false;
    }
  }

  void dispose() {
    _timer.cancel();
    _clipboardSubscription.cancel();
    _traySubscription.cancel();
    _networkSubscription.cancel();
    _networkDebounce?.cancel();
  }
}
