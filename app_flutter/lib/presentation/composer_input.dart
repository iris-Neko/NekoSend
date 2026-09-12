import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../application/composer_controller.dart';

class ComposerInput extends StatefulWidget {
  const ComposerInput({
    super.key,
    required this.controller,
    required this.conversationId,
    required this.onSubmitted,
    required this.sendOnEnter,
  });
  final ComposerController controller;
  final String conversationId;
  final VoidCallback onSubmitted;
  final bool sendOnEnter;

  @override
  State<ComposerInput> createState() => _ComposerInputState();
}

class _ComposerInputState extends State<ComposerInput> {
  final _inputFocus = FocusNode();
  ComposerController get controller => widget.controller;
  String get conversationId => widget.conversationId;
  bool get sendOnEnter => widget.sendOnEnter;

  @override
  void dispose() {
    _inputFocus.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final target = conversationId;
    await controller.submit(target);
    if (!mounted) return;
    widget.onSubmitted();
    if (conversationId == target) _inputFocus.requestFocus();
  }

  void _paste() {
    _inputFocus.requestFocus();
    controller.paste(conversationId);
  }

  Future<void> _pick(String kind) async {
    final target = conversationId;
    await controller.pick(target, kind);
    if (mounted && conversationId == target) _inputFocus.requestFocus();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
    animation: controller,
    builder: (context, _) {
      final draft = controller.draft(conversationId);
      final error =
          draft.error ??
          draft.attachments
              .map((item) => item.error)
              .whereType<String>()
              .firstOrNull;
      final media = MediaQuery.of(context);
      final view = View.of(context);
      // Scaffold removes consumed insets from its body MediaQuery.
      final keyboardInset = view.viewInsets.bottom / view.devicePixelRatio;
      final keyboardVisible = keyboardInset > 0 || media.viewInsets.bottom > 0;
      final compact =
          keyboardVisible &&
          view.physicalSize.height / view.devicePixelRatio - keyboardInset <
              280;
      return DecoratedBox(
        decoration: const BoxDecoration(
          color: Colors.white,
          border: Border(top: BorderSide(color: Color(0xFFDCE3E6))),
        ),
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            10,
            keyboardVisible ? 4 : 8,
            10,
            keyboardVisible ? 4 : 10,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (draft.attachments.isNotEmpty && !compact)
                SizedBox(
                  height: 100,
                  child: ListView.separated(
                    key: const ValueKey('composer-attachments'),
                    scrollDirection: Axis.horizontal,
                    itemCount: draft.attachments.length,
                    separatorBuilder: (_, _) => const SizedBox(width: 8),
                    itemBuilder: (context, index) {
                      final item = draft.attachments[index];
                      return Container(
                        width: 176,
                        margin: const EdgeInsets.only(bottom: 8),
                        decoration: BoxDecoration(
                          border: Border.all(
                            color: item.error == null
                                ? const Color(0xFFDCE3E6)
                                : Colors.red.shade300,
                          ),
                          borderRadius: BorderRadius.circular(6),
                        ),
                        child: Column(
                          children: [
                            Row(
                              children: [
                                SizedBox(
                                  width: 46,
                                  height: 46,
                                  child: item.prepared?.preview != null
                                      ? Image.file(
                                          File(item.prepared!.preview!),
                                          cacheWidth: 128,
                                          fit: BoxFit.contain,
                                          errorBuilder: (_, _, _) =>
                                              const Icon(LucideIcons.image),
                                        )
                                      : Icon(
                                          item.source.kind == 'folder'
                                              ? LucideIcons.folder
                                              : item.source.kind == 'image'
                                              ? LucideIcons.image
                                              : LucideIcons.file,
                                        ),
                                ),
                                Expanded(
                                  child: Tooltip(
                                    message: item.error ?? item.source.name,
                                    child: Text(
                                      item.source.name,
                                      maxLines: 2,
                                      overflow: TextOverflow.ellipsis,
                                      style: const TextStyle(fontSize: 12),
                                    ),
                                  ),
                                ),
                                SizedBox(
                                  width: 32,
                                  child: IconButton(
                                    padding: EdgeInsets.zero,
                                    tooltip: '移除附件',
                                    onPressed: draft.submitting
                                        ? null
                                        : () => controller.remove(
                                            conversationId,
                                            item,
                                          ),
                                    icon: const Icon(LucideIcons.x, size: 16),
                                  ),
                                ),
                              ],
                            ),
                            if (!item.ready && item.error == null)
                              LinearProgressIndicator(
                                value: item.total == null || item.total == 0
                                    ? null
                                    : (item.bytes / item.total!).clamp(0, 1),
                                minHeight: 3,
                              ),
                            Padding(
                              padding: const EdgeInsets.all(5),
                              child: Text(
                                item.error != null
                                    ? '无法读取，请移除后重新添加'
                                    : item.ready
                                    ? '${(item.prepared!.size / 1024).toStringAsFixed(1)} KB'
                                    : item.bytes > 0
                                    ? '准备中 ${(item.bytes / 1048576).toStringAsFixed(1)} MB'
                                    : '准备中',
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  fontSize: 11,
                                  color: item.error == null
                                      ? Colors.blueGrey
                                      : Colors.red,
                                ),
                              ),
                            ),
                          ],
                        ),
                      );
                    },
                  ),
                ),
              Row(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  MenuAnchor(
                    menuChildren: [
                      for (final entry in const {
                        'file': '文件',
                        'image': '图片',
                        'folder': '文件夹',
                      }.entries)
                        MenuItemButton(
                          onPressed: draft.submitting || draft.importing
                              ? null
                              : () => _pick(entry.key),
                          child: Text(entry.value),
                        ),
                      if (compact)
                        for (final attachment in draft.attachments)
                          MenuItemButton(
                            leadingIcon: const Icon(LucideIcons.x, size: 16),
                            onPressed: draft.submitting
                                ? null
                                : () {
                                    controller.remove(
                                      conversationId,
                                      attachment,
                                    );
                                    _inputFocus.requestFocus();
                                  },
                            child: Text(
                              '${attachment.source.name}${attachment.error != null
                                  ? '（无法读取）'
                                  : attachment.ready
                                  ? ''
                                  : '（准备中）'}',
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                    ],
                    builder: (context, menu, _) => IconButton(
                      tooltip: '附件',
                      onPressed: () => menu.isOpen ? menu.close() : menu.open(),
                      icon: Badge.count(
                        count: draft.attachments.length,
                        isLabelVisible: compact && draft.attachments.isNotEmpty,
                        child: const Icon(LucideIcons.paperclip, size: 20),
                      ),
                    ),
                  ),
                  IconButton(
                    tooltip: '粘贴',
                    onPressed: draft.submitting || draft.importing
                        ? null
                        : _paste,
                    icon: const Icon(LucideIcons.clipboard, size: 20),
                  ),
                  Expanded(
                    child: Actions(
                      actions: {
                        PasteTextIntent: CallbackAction<PasteTextIntent>(
                          onInvoke: (_) {
                            _paste();
                            return null;
                          },
                        ),
                      },
                      child: Focus(
                        canRequestFocus: false,
                        onKeyEvent: (_, event) {
                          if (!sendOnEnter ||
                              (event.logicalKey != LogicalKeyboardKey.enter &&
                                  event.logicalKey !=
                                      LogicalKeyboardKey.numpadEnter)) {
                            return KeyEventResult.ignored;
                          }
                          final composing = draft.text.value.composing;
                          if (composing.isValid && !composing.isCollapsed) {
                            return KeyEventResult.skipRemainingHandlers;
                          }
                          final keyboard = HardwareKeyboard.instance;
                          if (keyboard.isShiftPressed ||
                              keyboard.isControlPressed ||
                              keyboard.isAltPressed ||
                              keyboard.isMetaPressed) {
                            return KeyEventResult.ignored;
                          }
                          if (event is KeyDownEvent && draft.canSend) _submit();
                          return KeyEventResult.handled;
                        },
                        child: TextField(
                          focusNode: _inputFocus,
                          key: const ValueKey('message-input'),
                          controller: draft.text,
                          selectAllOnFocus: false,
                          readOnly: draft.submitting,
                          minLines: 1,
                          maxLines: keyboardVisible ? 1 : 5,
                          maxLength: 20000,
                          buildCounter: (
                            _, {
                            required currentLength,
                            required isFocused,
                            maxLength,
                          }) => null,
                          onSubmitted: (_) {
                            if (draft.canSend) _submit();
                          },
                          contextMenuBuilder: (context, editable) =>
                              AdaptiveTextSelectionToolbar.buttonItems(
                                anchors: editable.contextMenuAnchors,
                                buttonItems: [
                                  ...editable.contextMenuButtonItems.where(
                                    (item) =>
                                        item.type !=
                                        ContextMenuButtonType.paste,
                                  ),
                                  ContextMenuButtonItem(
                                    type: ContextMenuButtonType.paste,
                                    onPressed: () {
                                      ContextMenuController.removeAny();
                                      _paste();
                                    },
                                  ),
                                ],
                              ),
                          decoration: const InputDecoration(hintText: '输入消息'),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: 6),
                  IconButton.filled(
                    tooltip: '发送',
                    onPressed: draft.canSend ? _submit : null,
                    icon: draft.submitting
                        ? const SizedBox(
                            width: 19,
                            height: 19,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(LucideIcons.send, size: 19),
                  ),
                ],
              ),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(
                      error,
                      maxLines: compact ? 1 : 2,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(color: Colors.red, fontSize: 12),
                    ),
                  ),
                ),
            ],
          ),
        ),
      );
    },
  );
}
