// Run with sharp available through NODE_PATH. The source artwork is user supplied.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require('sharp');

async function main() {
  const root = path.resolve(__dirname, '..');
  const source = path.join(root, 'packaging/icons/source.png');
  const {data, info} = await sharp(source).ensureAlpha().raw().toBuffer({resolveWithObject: true});
  const seen = new Uint8Array(info.width * info.height);
  const queue = [];
  function visit(x, y) {
    if (x < 0 || y < 0 || x >= info.width || y >= info.height) return;
    const index = y * info.width + x;
    if (seen[index]) return;
    seen[index] = 1;
    const offset = index * 4;
    if (Math.max(data[offset], data[offset + 1], data[offset + 2]) > 32) return;
    data[offset + 3] = 0;
    queue.push(index);
  }
  for (let x = 0; x < info.width; x++) { visit(x, 0); visit(x, info.height - 1); }
  for (let y = 0; y < info.height; y++) { visit(0, y); visit(info.width - 1, y); }
  for (let i = 0; i < queue.length; i++) {
    const x = queue[i] % info.width, y = Math.floor(queue[i] / info.width);
    visit(x - 1, y); visit(x + 1, y); visit(x, y - 1); visit(x, y + 1);
  }
  const master = await sharp(data, {raw: info}).png().toBuffer();
  async function png(relative, size) {
    const file = path.join(root, relative);
    await fs.mkdir(path.dirname(file), {recursive: true});
    await sharp(master).resize(size, size).png().toFile(file);
  }
  await png('packaging/icons/app-icon.png', 1024);
  for (const size of [16,32,64,128,256,512,1024]) {
    await png(`app_flutter/macos/Runner/Assets.xcassets/AppIcon.appiconset/app_icon_${size}.png`, size);
  }
  for (const [density,size] of Object.entries({mdpi:48,hdpi:72,xhdpi:96,xxhdpi:144,xxxhdpi:192})) {
    await png(`app_flutter/android/app/src/main/res/mipmap-${density}/ic_launcher.png`, size);
  }
  await png('packaging/linux/io.github.iris_neko.NekoSend.png', 512);
  const sizes = [16,24,32,48,64,128,256];
  const images = await Promise.all(sizes.map(size => sharp(master).resize(size,size).png().toBuffer()));
  const header = Buffer.alloc(6 + 16 * sizes.length);
  header.writeUInt16LE(1,2); header.writeUInt16LE(sizes.length,4);
  let offset = header.length;
  images.forEach((image,i) => {
    const entry = 6 + i * 16;
    header[entry] = sizes[i] % 256; header[entry + 1] = sizes[i] % 256;
    header.writeUInt16LE(1,entry+4); header.writeUInt16LE(32,entry+6);
    header.writeUInt32LE(image.length,entry+8); header.writeUInt32LE(offset,entry+12);
    offset += image.length;
  });
  await fs.writeFile(path.join(root,'app_flutter/windows/runner/resources/app_icon.ico'), Buffer.concat([header,...images]));
  console.log('Generated Android, Windows, macOS and Linux icons from the same artwork.');
}
main().catch(error => { console.error(error); process.exitCode = 1; });
