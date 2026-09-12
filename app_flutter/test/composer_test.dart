import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lan_chat/application/composer_controller.dart';
import 'package:lan_chat/presentation/composer_input.dart';

class Fixture {
  int sequence = 0;
  ClipboardContent content = const ClipboardContent();
  Completer<ClipboardContent>? read;
  Completer<PreparedAttachment>? preparation;
  String? failing;
  final sent = <String>[];
  final operations = <String>[];
  final released = <PreparedAttachment>[];
  late final services = ComposerServices(
    operationId: () => 'op-${sequence++}',
    readClipboard: () async => read == null ? content : await read!.future,
    pick: (_) async => content.sources,
    prepare: (source, token, progress) async {
      if (preparation != null) return preparation!.future;
      return PreparedAttachment(
        source: source,
        payload: source.name,
        size: 123,
        preview: source.kind == 'image' ? source.reference : null,
      );
    },
    submitText: (id, conversation, text) async {
      operations.add(id);
      if (failing == text) throw StateError('submit failed');
      sent.add('$conversation:$text');
    },
    submitAttachment: (id, conversation, item) async {
      operations.add(id);
      if (failing == item.source.name) throw StateError('submit failed');
      item.submitted = true;
      sent.add('$conversation:${item.source.name}');
    },
    release: (item) async {
      released.add(item);
    },
  );
  late final controller = ComposerController(services);
}

const first = AttachmentSource(reference: '/tmp/a.txt', name: 'a.txt');
const second = AttachmentSource(reference: '/tmp/b.txt', name: 'b.txt');
const third = AttachmentSource(reference: '/tmp/c.txt', name: 'c.txt');

Future<void> settle() async {
  for (var i = 0; i < 8; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

void main() {
  for (final width in [360.0, 1000.0]) {
    testWidgets('composer layout preview at $width', (tester) async {
      final output = Platform.environment['NEKOSEND_CAPTURE_COMPOSER'];
      final fontPath = Platform.environment['NEKOSEND_COMPOSER_FONT'];
      if (fontPath != null) {
        await tester.runAsync(() async {
          await (FontLoader('ComposerPreview')..addFont(
                File(fontPath).readAsBytes().then(ByteData.sublistView),
              ))
              .load();
          await (FontLoader('packages/lucide_icons_flutter/Lucide')..addFont(
                rootBundle.load(
                  'packages/lucide_icons_flutter/assets/lucide.ttf',
                ),
              ))
              .load();
        });
      }
      final f = Fixture();
      addTearDown(f.controller.dispose);
      f.content = ClipboardContent(
        sources: [
          AttachmentSource(
            reference: File('../packaging/icons/app-icon.png').absolute.path,
            name: 'photo.png',
            kind: 'image',
          ),
          first,
          second,
        ],
      );
      await f.controller.paste('a');
      await tester.pump();
      f.controller.draft('a').text.text = '文件和图片已经整理好了';
      await tester.binding.setSurfaceSize(Size(width, 250));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      final capture = GlobalKey();
      await tester.pumpWidget(
        MaterialApp(
          theme: ThemeData(
            fontFamily: fontPath == null ? null : 'ComposerPreview',
            colorScheme: ColorScheme.fromSeed(
              seedColor: const Color(0xFF087F72),
            ),
          ),
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: RepaintBoundary(
                key: capture,
                child: ComposerInput(
                  controller: f.controller,
                  conversationId: 'a',
                  onSubmitted: () {},
                  sendOnEnter: true,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.runAsync(() async {
        await precacheImage(
          ResizeImage(
            FileImage(File('../packaging/icons/app-icon.png').absolute),
            width: 128,
          ),
          tester.element(find.byType(ComposerInput)),
        );
      });
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      if (output != null) {
        await tester.runAsync(() async {
          final image =
              await (capture.currentContext!.findRenderObject()!
                      as RenderRepaintBoundary)
                  .toImage();
          final data = await image.toByteData(format: ui.ImageByteFormat.png);
          await Directory(output).create(recursive: true);
          await File('$output/composer-${width.toInt()}.png')
              .writeAsBytes(data!.buffer.asUint8List());
          image.dispose();
        });
      }
    });
  }
  test(
    'files take precedence over path text and repeated paste deduplicates',
    () async {
      final f = Fixture();
      addTearDown(f.controller.dispose);
      f.content = const ClipboardContent(
        text: 'must not become a message',
        sources: [first, second],
      );
      f.controller.draft('a').text.text = 'caption';
      await f.controller.paste('a');
      await f.controller.paste('a');
      await settle();
      expect(f.controller.draft('a').attachments, hasLength(2));
      expect(f.sent, isEmpty);
      expect(await f.controller.submit('a'), isTrue);
      expect(f.sent, ['a:caption', 'a:a.txt', 'a:b.txt']);
    },
  );

  test(
    'partial submission retains remaining items and their operation IDs',
    () async {
      final f = Fixture();
      addTearDown(f.controller.dispose);
      f.content = const ClipboardContent(sources: [first, second, third]);
      await f.controller.paste('a');
      await settle();
      f.controller.draft('a').text.text = 'caption';
      f.failing = 'b.txt';
      expect(await f.controller.submit('a'), isFalse);
      final retryId = f.operations.last;
      expect(f.controller.draft('a').text.text, isEmpty);
      expect(f.controller.draft('a').attachments.map((e) => e.source.name), [
        'b.txt',
        'c.txt',
      ]);
      f.failing = null;
      expect(await f.controller.submit('a'), isTrue);
      expect(f.operations.where((id) => id == retryId), hasLength(2));
      expect(f.sent, ['a:caption', 'a:a.txt', 'a:b.txt', 'a:c.txt']);
    },
  );

  test('late paste belongs to its original conversation', () async {
    final f = Fixture();
    addTearDown(f.controller.dispose);
    f.read = Completer();
    final pending = f.controller.paste('a');
    f.controller.draft('b').text.text = 'other draft';
    f.read!.complete(const ClipboardContent(sources: [first]));
    await pending;
    await settle();
    expect(f.controller.draft('a').attachments, hasLength(1));
    expect(f.controller.draft('b').attachments, isEmpty);
    expect(f.controller.draft('b').text.text, 'other draft');
  });

  test('deleted conversation discards a late clipboard result', () async {
    final f = Fixture();
    addTearDown(f.controller.dispose);
    f.read = Completer();
    final pending = f.controller.paste('a');
    f.controller.forget('a');
    f.read!.complete(const ClipboardContent(sources: [first]));
    await pending;
    await settle();
    expect(f.controller.draft('a').attachments, isEmpty);
  });

  test(
    'removing a preparing attachment releases the late prepared source',
    () async {
      final f = Fixture();
      addTearDown(f.controller.dispose);
      f.preparation = Completer();
      f.content = const ClipboardContent(sources: [first]);
      await f.controller.paste('a');
      await settle();
      final item = f.controller.draft('a').attachments.single;
      expect(f.controller.draft('a').canSend, isFalse);
      f.controller.remove('a', item);
      f.preparation!.complete(
        PreparedAttachment(source: first, payload: 0, size: 4),
      );
      await settle();
      expect(item.token.cancelled, isTrue);
      expect(f.released, hasLength(1));
      expect(f.sent, isEmpty);
    },
  );

  test('text paste uses selection and never reads a path as a file', () async {
    final f = Fixture();
    addTearDown(f.controller.dispose);
    final draft = f.controller.draft('a');
    draft.text.value = const TextEditingValue(
      text: 'one two',
      selection: TextSelection(baseOffset: 4, extentOffset: 7),
    );
    f.content = const ClipboardContent(text: 'C:\\private\\file.txt');
    await f.controller.paste('a');
    expect(draft.text.text, 'one C:\\private\\file.txt');
    expect(draft.attachments, isEmpty);
  });

  test('late text paste cannot overwrite newer input', () async {
    final f = Fixture();
    addTearDown(f.controller.dispose);
    f.read = Completer();
    final pending = f.controller.paste('a');
    f.controller.draft('a').text.text = 'newer';
    f.read!.complete(const ClipboardContent(text: 'old'));
    await pending;
    expect(f.controller.draft('a').text.text, 'newer');
    expect(f.controller.draft('a').error, isNotNull);
  });

  test(
    'drafts are session-only and preparation errors keep the attachment',
    () async {
      final f = Fixture();
      f.preparation = Completer();
      f.content = const ClipboardContent(sources: [first]);
      await f.controller.paste('a');
      await settle();
      f.preparation!.completeError(StateError('permission lost'));
      await settle();
      expect(
        f.controller.draft('a').attachments.single.error,
        contains('permission lost'),
      );
      expect(f.controller.draft('a').canSend, isFalse);
      f.controller.dispose();
      final next = Fixture();
      addTearDown(next.controller.dispose);
      expect(next.controller.draft('a').attachments, isEmpty);
    },
  );

  for (final platform in [
    TargetPlatform.windows,
    TargetPlatform.macOS,
    TargetPlatform.linux,
    TargetPlatform.android,
  ]) {
    testWidgets('paste keyboard shortcut stages files on $platform', (
      tester,
    ) async {
      final f = Fixture();
      addTearDown(f.controller.dispose);
      f.content = const ClipboardContent(sources: [first, second]);
      await tester.binding.setSurfaceSize(const Size(360, 640));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomCenter,
              child: ComposerInput(
                controller: f.controller,
                conversationId: 'a',
                onSubmitted: () {},
                sendOnEnter: true,
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.byKey(const ValueKey('message-input')));
      await tester.pump();
      final modifier = platform == TargetPlatform.macOS
          ? LogicalKeyboardKey.metaLeft
          : LogicalKeyboardKey.controlLeft;
      await tester.sendKeyDownEvent(modifier);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
      await tester.sendKeyUpEvent(modifier);
      await tester.pumpAndSettle();
      expect(f.controller.draft('a').attachments, hasLength(2));
      expect(f.sent, isEmpty);
      expect(tester.takeException(), isNull);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(f.sent, ['a:a.txt', 'a:b.txt']);
    }, variant: TargetPlatformVariant.only(platform));
  }
}
