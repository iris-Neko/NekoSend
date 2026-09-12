import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    await const FlutterRustBridgeNativeAssetsBuilder(cratePath: '../core_rust')
        .run(input: input, output: output);
    for (final file in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']) {
      output.dependencies.add(input.packageRoot.resolve('../$file'));
    }
  });
}
