import 'dart:async';
import 'dart:io';

import '../application/composer_controller.dart';
import '../src/rust/api/composer.dart' as rust;
import '../src/rust/api/core.dart' as core;
import 'platform_bootstrap.dart';

ComposerServices createComposerServices(PlatformBootstrap bootstrap) {
  final session = core.generateClientOperationId();
  final root = Directory(
    '${bootstrap.dataDirectory}${Platform.pathSeparator}composer-sources',
  );
  final sessionRoot = Directory(
    '${root.path}${Platform.pathSeparator}$session',
  );
  // Retained by service closures for the application lifetime. The OS releases
  // this non-blocking lease on exit, including crashes and forced termination.
  final ownership = () async {
    await sessionRoot.create(recursive: true);
    final lease = await File('${sessionRoot.path}/.lease')
        .open(mode: FileMode.append);
    await lease.lock(FileLock.exclusive);
    return lease;
  }();
  // Drafts are session-only. Submitted sources keep an on-disk ownership marker
  // because a lost bridge response must not cause deletion of a queued file.
  final cleanup = () async {
    try {
      await ownership;
      if (!await root.exists()) return;
      await for (final oldSession in root.list(followLinks: false)) {
        if (oldSession is! Directory || oldSession.path == sessionRoot.path) {
          continue;
        }
        final leaseFile = File('${oldSession.path}/.lease');
        if (!await leaseFile.exists()) continue;
        final lease = await leaseFile.open(mode: FileMode.append);
        try {
          await lease.lock(FileLock.exclusive);
          await for (final source in oldSession.list(followLinks: false)) {
            if (source is Directory &&
                !await File('${source.path}/.submitted').exists()) {
              if (source.absolute.path.startsWith(
                '${root.absolute.path}${Platform.pathSeparator}',
              )) {
                await source.delete(recursive: true);
              }
            }
          }
        } on FileSystemException {
          /* Another process still owns its drafts. */
        } finally {
          await lease.close();
        }
      }
    } catch (_) {
      /* Keep files when ownership cannot be established. */
    }
  }();

  Future<void> release(PreparedAttachment item) async {
    if (item.submitted || item.ownedDirectory == null) return;
    await ownership;
    final directory = Directory(item.ownedDirectory!);
    if (!directory.absolute.path.startsWith(
      '${sessionRoot.absolute.path}${Platform.pathSeparator}',
    )) {
      return;
    }
    if (await File('${directory.path}/.submitted').exists()) return;
    try {
      if (await directory.exists()) await directory.delete(recursive: true);
    } catch (_) {}
  }

  return ComposerServices(
    operationId: core.generateClientOperationId,
    readClipboard: PlatformBootstrap.readClipboardContent,
    pick: (kind) async {
      if (bootstrap.platform == 'android') {
        final picked = await PlatformBootstrap.pickSafSource(kind);
        if (picked == null) return [];
        return [
          AttachmentSource(
            reference: picked.sources.first.sourceRef!,
            name: picked.displayName,
            kind: kind,
            manifest: picked,
          ),
        ];
      }
      final path = await PlatformBootstrap.pickSource(kind);
      if (path == null) return [];
      return [
        AttachmentSource(
          reference: path,
          name: File(path).uri.pathSegments.last,
          kind: kind,
        ),
      ];
    },
    cancelPreparation: PlatformBootstrap.cancelComposerPreparation,
    prepare: (source, token, progress) async {
      await cleanup;
      token.check();
      String path = source.reference;
      String? owned;
      rust.ComposerSourceDto manifest;
      try {
        if (source.manifest case final SafPickedSource picked) {
          manifest = rust.ComposerSourceDto(
            displayName: picked.displayName,
            kind: source.kind,
            totalSize: picked.sources.fold(
              BigInt.zero,
              (sum, item) => sum + item.size,
            ),
            sources: picked.sources,
          );
          // SAF selection already holds the user's persisted permission.
          final preview = source.kind == 'image'
              ? await PlatformBootstrap.resolveImagePreview(source.reference)
                    .catchError((Object _) => null)
              : null;
          token.check();
          return PreparedAttachment(
            source: source,
            payload: manifest,
            size: manifest.totalSize.toInt(),
            preview: preview,
          );
        }
        if (path.startsWith('content://')) {
          final subscription = PlatformBootstrap.composerProgress.listen((
            event,
          ) {
            if (event['token'] == token.id) {
              progress(
                (event['bytes'] as num).toInt(),
                (event['total'] as num?)?.toInt(),
              );
            }
          });
          try {
            final prepared = await PlatformBootstrap.prepareComposerUri(
              path,
              source.kind,
              token.id,
              session,
            );
            if (prepared == null) throw StateError('无法读取剪贴板文件，请使用附件选择器');
            path = prepared['path'] as String;
            owned = prepared['ownedDirectory'] as String;
          } finally {
            await subscription.cancel();
          }
          token.check();
        }
        var kind = source.kind;
        final type = await FileSystemEntity.type(path, followLinks: false);
        if (type == FileSystemEntityType.directory) kind = 'folder';
        if (kind == 'file' &&
            RegExp(
              r'\.(png|jpg|jpeg|webp|gif|bmp)$',
              caseSensitive: false,
            ).hasMatch(path)) {
          kind = 'image';
        }
        manifest = await rust.prepareComposerSource(path: path, kind: kind);
        token.check();
        if (source.ephemeral && owned == null) {
          owned = '${sessionRoot.path}${Platform.pathSeparator}${token.id}';
          await Directory(owned).create(recursive: true);
          var copied = 0;
          final clock = Stopwatch()..start();
          var lastProgress = 0;
          for (final entry in manifest.sources) {
            token.check();
            final destination =
                '$owned${Platform.pathSeparator}payload${Platform.pathSeparator}${entry.relativePath.replaceAll('/', Platform.pathSeparator)}';
            if (entry.entryKind == 'directory') {
              await Directory(destination).create(recursive: true);
            } else {
              await File(destination).parent.create(recursive: true);
              final output = await File(destination).open(mode: FileMode.write);
              try {
                await for (final chunk in File(entry.sourceRef!).openRead()) {
                  token.check();
                  await output.writeFrom(chunk);
                  copied += chunk.length;
                  if (clock.elapsedMilliseconds - lastProgress >= 100) {
                    progress(copied, manifest.totalSize.toInt());
                    lastProgress = clock.elapsedMilliseconds;
                  }
                }
                await output.flush();
              } finally {
                await output.close();
              }
            }
          }
          if (copied != manifest.totalSize.toInt()) {
            throw StateError('附件内容已变化，请重新添加');
          }
          progress(copied, manifest.totalSize.toInt());
          path =
              '$owned${Platform.pathSeparator}payload${Platform.pathSeparator}${manifest.displayName}';
          manifest = await rust.prepareComposerSource(
            path: path,
            kind: manifest.kind,
          );
          token.check();
        }
        return PreparedAttachment(
          source: source,
          payload: manifest,
          size: manifest.totalSize.toInt(),
          preview:
              manifest.kind == 'image' &&
                  manifest.totalSize <= BigInt.from(20 * 1024 * 1024)
              ? path
              : null,
          ownedDirectory: owned,
        );
      } catch (_) {
        if (owned != null) {
          await release(
            PreparedAttachment(
              source: source,
              payload: 0,
              size: 0,
              ownedDirectory: owned,
            ),
          );
        }
        rethrow;
      }
    },
    validate: (attachment) async {
      final previous = attachment.payload as rust.ComposerSourceDto;
      final reference = previous.sources.first.sourceRef!;
      if (reference.startsWith('content://')) {
        await PlatformBootstrap.validateComposerUris(
          previous.sources
              .where((item) => item.entryKind == 'file')
              .map((item) => item.sourceRef!)
              .toList(),
        );
      } else {
        final current = await rust.prepareComposerSource(
          path: reference,
          kind: previous.kind,
        );
        if (current.sources.length != previous.sources.length) {
          throw StateError('附件内容已变化，请重新添加');
        }
        for (var index = 0; index < current.sources.length; index++) {
          final before = previous.sources[index];
          final after = current.sources[index];
          if (before.relativePath != after.relativePath ||
              before.size != after.size ||
              before.modifiedAtMs != after.modifiedAtMs) {
            throw StateError('附件内容已变化，请重新添加');
          }
        }
      }
    },
    submitText: (operation, conversation, text) async {
      await rust.submitComposerText(
        clientOperationId: operation,
        conversationId: conversation,
        text: text,
      );
    },
    submitAttachment: (operation, conversation, attachment) async {
      // A failed bridge response may follow a committed transaction: retain the source
      // and replay the same operation ID instead of deleting or duplicating it.
      attachment.submitted = true;
      if (attachment.ownedDirectory != null) {
        await File('${attachment.ownedDirectory}/.submitted')
            .writeAsString(operation, flush: true);
      }
      await rust.submitComposerAttachment(
        clientOperationId: operation,
        conversationId: conversation,
        source: attachment.payload as rust.ComposerSourceDto,
      );
    },
    release: release,
  );
}
