import 'dart:io';

import 'package:vm_service/vm_service_io.dart';

Future<void> main(List<String> arguments) async {
  if (arguments.length != 3) {
    stderr.writeln(
      'usage: dart run tool/send_debug_message.dart '
      '<vm-service-websocket-uri> <conversation-id> <text>',
    );
    exitCode = 64;
    return;
  }

  final service = await vmServiceConnectUri(arguments[0]);
  try {
    final vm = await service.getVM();
    final isolateRef = vm.isolates?.firstWhere(
      (isolate) => isolate.name == 'main',
    );
    if (isolateRef?.id == null) {
      throw StateError('main isolate is unavailable');
    }
    final result = await service.callServiceExtension(
      'ext.lanChat.sendText',
      isolateId: isolateRef!.id!,
      args: {'conversationId': arguments[1], 'text': arguments[2]},
    );
    stdout.writeln(result.json);
  } finally {
    await service.dispose();
  }
}
