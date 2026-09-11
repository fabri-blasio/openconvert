import sharp from "sharp";
import fs from "node:fs/promises";

const source = "brand/logo/openconvert-mole-logo.svg";
const desktopSource = "brand/logo/desktop/app-icon.svg";
const website = "brand/logo/website";
const desktop = "brand/logo/desktop";
const background = { r: 252, g: 252, b: 251, alpha: 1 };

for (const size of [16, 32, 48, 180, 192, 512]) {
  await sharp(source)
    .resize(size, size, { fit: "contain", background: background })
    .png()
    .toFile(`${website}/favicon-${size}.png`);
}

await sharp(source).resize({ width: 1200 }).png().toFile(`${website}/logo-1200.png`);
await sharp(source).resize({ width: 1200 }).png().toFile(`${website}/og-image.png`);

const sizes = [16, 32, 48, 64, 128, 256, 512, 1024];
const buffers = [];
for (const size of sizes) {
  const buffer = await sharp(desktopSource)
    .resize(size, size, { fit: "contain", background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png()
    .toBuffer();
  buffers.push(buffer);
  await fs.writeFile(`${desktop}/app-icon-${size}.png`, buffer);
}

// Build a standard PNG-backed Windows ICO with all common sizes.
const directory = Buffer.alloc(6);
directory.writeUInt16LE(0, 0);
directory.writeUInt16LE(1, 2);
directory.writeUInt16LE(sizes.length, 4);
const entries = [];
let offset = 6 + sizes.length * 16;
for (let i = 0; i < sizes.length; i += 1) {
  const size = sizes[i];
  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size;
  entry[1] = size >= 256 ? 0 : size;
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  entry.writeUInt32LE(buffers[i].length, 8);
  entry.writeUInt32LE(offset, 12);
  entries.push(entry);
  offset += buffers[i].length;
}
const ico = Buffer.concat([directory, ...entries, ...buffers]);
await fs.writeFile(`${desktop}/app-icon.ico`, ico);
await fs.writeFile(`${website}/favicon.ico`, ico);
