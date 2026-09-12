import 'dart:async';

import 'package:flutter/widgets.dart';

class AttachmentSource {
  const AttachmentSource({
    required this.reference,
    required this.name,
    this.kind = 'file',
    this.ephemeral = false,
    this.fingerprint,
    this.manifest,
  });
  final String reference;
  final String name;
  final String kind;
  final bool ephemeral;
  final String? fingerprint;
  final Object? manifest;
  String get identity => fingerprint ?? reference;
}

class ClipboardContent {
  const ClipboardContent({this.text, this.sources = const []});
  final String? text;
  final List<AttachmentSource> sources;
}

class PreparationToken {
  PreparationToken(this.id);
  final String id;
  bool _cancelled = false;
  final _signal = Completer<void>();
  bool get cancelled => _cancelled;
  set cancelled(bool value) {
    if (value && !_cancelled) {
      _cancelled = true;
      _signal.complete();
    }
  }

  Future<void> get cancellation => _signal.future;
  void check() {
    if (cancelled) throw StateError('附件已移除');
  }
}

class PreparedAttachment {
  PreparedAttachment({
    required this.source,
    required this.payload,
    required this.size,
    this.preview,
    this.ownedDirectory,
  });
  final AttachmentSource source;
  final Object payload;
  final int size;
  final String? preview;
  final String? ownedDirectory;
  bool submitted = false;
}

class ComposerServices {
  const ComposerServices({
    required this.operationId,
    required this.readClipboard,
    required this.pick,
    required this.prepare,
    required this.submitAttachment,
    required this.submitText,
    required this.release,
    this.cancelPreparation,
    this.validate,
  });
  final String Function() operationId;
  final Future<ClipboardContent> Function() readClipboard;
  final Future<List<AttachmentSource>> Function(String kind) pick;
  final Future<PreparedAttachment> Function(
    AttachmentSource source,
    PreparationToken token,
    void Function(int bytes, int? total) progress,
  )
  prepare;
  final Future<void> Function(
    String operationId,
    String conversationId,
    PreparedAttachment attachment,
  )
  submitAttachment;
  final Future<void> Function(
    String operationId,
    String conversationId,
    String text,
  )
  submitText;
  final Future<void> Function(PreparedAttachment attachment) release;
  final void Function(String token)? cancelPreparation;
  final Future<void> Function(PreparedAttachment attachment)? validate;
}

class DraftAttachment {
  DraftAttachment(this.source, this.token);
  final AttachmentSource source;
  final PreparationToken token;
  PreparedAttachment? prepared;
  String? error;
  int bytes = 0;
  int? total;
  bool get ready => prepared != null && error == null;
}

class ComposerDraft {
  final text = TextEditingController();
  final attachments = <DraftAttachment>[];
  bool submitting = false;
  bool importing = false;
  bool removed = false;
  String? textOperationId;
  String? submittedText;
  String? error;
  bool get canSend =>
      !submitting &&
      !importing &&
      attachments.every((item) => item.ready) &&
      (text.text.trim().isNotEmpty || attachments.isNotEmpty);
}

class ComposerController extends ChangeNotifier {
  ComposerController(this.services);
  final ComposerServices? services;
  final _drafts = <String, ComposerDraft>{};
  bool _disposed = false;
  Future<void> _preparing = Future.value();
  ComposerDraft draft(String conversation) =>
      _drafts.putIfAbsent(conversation, () {
        final value = ComposerDraft();
        value.text.addListener(_changed);
        return value;
      });
  void _changed() {
    if (!_disposed) notifyListeners();
  }

  Future<void> paste(String conversation) async {
    final api = services;
    if (api == null || conversation.isEmpty) return;
    final value = draft(conversation);
    if (value.importing || value.submitting) return;
    final editing = value.text.value;
    value.importing = true;
    value.error = null;
    _changed();
    try {
      final content = await api.readClipboard();
      if (_disposed || value.removed) return;
      if (content.sources.isNotEmpty) {
        await _add(value, content.sources);
      } else if (content.text != null && content.text!.isNotEmpty) {
        if (value.text.value != editing) throw StateError('输入已变化，请重新粘贴');
        final selection = editing.selection.isValid
            ? editing.selection
            : TextSelection.collapsed(offset: editing.text.length);
        final pasted = editing.text.replaceRange(
          selection.start,
          selection.end,
          content.text!,
        );
        if (pasted.length > 20000) throw StateError('消息不能超过 20000 个字符');
        value.text.value = TextEditingValue(
          text: pasted,
          selection: TextSelection.collapsed(
            offset: selection.start + content.text!.length,
          ),
        );
      } else {
        throw StateError('剪贴板中没有可粘贴的内容');
      }
    } catch (error) {
      if (!value.removed) value.error = error.toString();
    } finally {
      value.importing = false;
      _changed();
    }
  }

  Future<void> pick(String conversation, String kind) async {
    final api = services;
    if (api == null || conversation.isEmpty) return;
    final value = draft(conversation);
    if (value.importing || value.submitting) return;
    value.importing = true;
    value.error = null;
    _changed();
    try {
      final sources = await api.pick(kind);
      if (!_disposed && !value.removed) await _add(value, sources);
    } catch (error) {
      if (!value.removed) value.error = error.toString();
    } finally {
      value.importing = false;
      _changed();
    }
  }

  Future<void> _add(ComposerDraft value, List<AttachmentSource> sources) async {
    final known = value.attachments.map((item) => item.source.identity).toSet();
    var added = 0;
    for (final source in sources) {
      if (_disposed || value.removed) break;
      if (!known.add(source.identity)) continue;
      final item = DraftAttachment(
        source,
        PreparationToken(services!.operationId()),
      );
      value.attachments.add(item);
      _preparing = _preparing.then((_) => _prepare(value, item));
      if (++added % 64 == 0) {
        _changed();
        await Future<void>.delayed(Duration.zero);
      }
    }
    _changed();
  }

  Future<void> _prepare(ComposerDraft value, DraftAttachment item) async {
    if (_disposed || value.removed || item.token.cancelled) return;
    try {
      final task = services!.prepare(item.source, item.token, (bytes, total) {
        if (_disposed || item.token.cancelled) return;
        item.bytes = bytes;
        item.total = total;
        _changed();
      });
      final prepared = await Future.any<PreparedAttachment?>([
        task,
        item.token.cancellation.then<PreparedAttachment?>((_) => null),
      ]);
      if (prepared == null) {
        unawaited(
          task
              .then((completed) => services!.release(completed))
              .catchError((Object _) {}),
        );
        return;
      }
      if (_disposed || value.removed || item.token.cancelled) {
        await services!.release(prepared);
      } else {
        item.prepared = prepared;
      }
    } catch (error) {
      if (!item.token.cancelled && !value.removed) {
        item.error = error.toString();
      }
    }
    _changed();
  }

  void remove(String conversation, DraftAttachment item) {
    final value = draft(conversation);
    if (value.submitting) return;
    value.attachments.remove(item);
    value.error = null;
    item.token.cancelled = true;
    services?.cancelPreparation?.call(item.token.id);
    if (item.prepared != null) unawaited(services!.release(item.prepared!));
    _changed();
  }

  Future<bool> submit(String conversation) async {
    final api = services;
    final value = draft(conversation);
    if (api == null || !value.canSend) return false;
    value.submitting = true;
    value.error = null;
    _changed();
    try {
      for (final item in value.attachments) {
        if (!item.prepared!.submitted) {
          try {
            await api.validate?.call(item.prepared!);
          } catch (error) {
            item.error = error.toString();
            rethrow;
          }
        }
      }
      if (_disposed) throw StateError('应用已退出');
      final text = value.text.text;
      if (text.trim().isNotEmpty) {
        if (value.submittedText != text) {
          value.textOperationId = api.operationId();
          value.submittedText = text;
        }
        await api.submitText(value.textOperationId!, conversation, text);
        value.text.clear();
        value.textOperationId = null;
        value.submittedText = null;
      }
      while (value.attachments.isNotEmpty) {
        if (_disposed) throw StateError('应用已退出');
        final item = value.attachments.first;
        await api.submitAttachment(item.token.id, conversation, item.prepared!);
        value.attachments.removeAt(0);
        _changed();
      }
      return true;
    } catch (error) {
      value.error = error.toString();
      return false;
    } finally {
      value.submitting = false;
      if (_disposed) forget(conversation);
      _changed();
    }
  }

  void forget(String conversation) {
    final value = _drafts[conversation];
    if (value == null || value.submitting) return;
    for (final item in value.attachments.toList()) {
      remove(conversation, item);
    }
    value.removed = true;
    value.text.removeListener(_changed);
    value.text.dispose();
    _drafts.remove(conversation);
  }

  @override
  void dispose() {
    _disposed = true;
    for (final key in _drafts.keys.toList()) {
      forget(key);
    }
    super.dispose();
  }
}
