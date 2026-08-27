import 'dart:convert';
import 'dart:io';

import 'package:vm_service/vm_service_io.dart';

Future<void> main(List<String> arguments) async {
  if (arguments.length < 2) {
    stderr.writeln(
      'usage: dart run tool/debug_command.dart '
      '<vm-service-websocket-uri> <action> [name=value ...]',
    );
    exitCode = 64;
    return;
  }

  final parameters = <String, String>{'action': arguments[1]};
  for (final argument in arguments.skip(2)) {
    final separator = argument.indexOf('=');
    if (separator < 1) {
      stderr.writeln('invalid parameter: $argument');
      exitCode = 64;
      return;
    }
    parameters[argument.substring(0, separator)] = argument.substring(
      separator + 1,
    );
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
    final extension = switch (arguments[1]) {
      'send_text' => 'ext.lanChat.sendText',
      'send_source' => 'ext.lanChat.sendSourcePath',
      _ => 'ext.lanChat.debugCommand',
    };
    final result = await service.callServiceExtension(
      extension,
      isolateId: isolateRef!.id!,
      args: parameters,
    );
    stdout.writeln(jsonEncode(result.json));
  } finally {
    await service.dispose();
  }
}
