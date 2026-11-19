// Draw the RustCade icon procedurally and write PNGs for the web manifest.
// No image libraries: raw RGBA scanlines + zlib deflate.

import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, '..', 'web', 'icons');

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function encodePng(width, height, rgba) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0; // filter: none
    rgba.copy(raw, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

class Canvas {
  constructor(size) {
    this.size = size;
    this.data = Buffer.alloc(size * size * 4);
  }

  set(x, y, [r, g, b, a]) {
    if (x < 0 || y < 0 || x >= this.size || y >= this.size) return;
    const i = (y * this.size + x) * 4;
    if (a === 255) {
      this.data[i] = r;
      this.data[i + 1] = g;
      this.data[i + 2] = b;
      this.data[i + 3] = 255;
      return;
    }
    const src = a / 255;
    const dst = this.data[i + 3] / 255;
    const out = src + dst * (1 - src);
    if (out === 0) return;
    this.data[i] = Math.round((r * src + this.data[i] * dst * (1 - src)) / out);
    this.data[i + 1] = Math.round((g * src + this.data[i + 1] * dst * (1 - src)) / out);
    this.data[i + 2] = Math.round((b * src + this.data[i + 2] * dst * (1 - src)) / out);
    this.data[i + 3] = Math.round(out * 255);
  }

  roundRect(x, y, w, h, r, color) {
    for (let py = Math.floor(y); py < Math.ceil(y + h); py++) {
      for (let px = Math.floor(x); px < Math.ceil(x + w); px++) {
        const cx = Math.min(Math.max(px + 0.5, x + r), x + w - r);
        const cy = Math.min(Math.max(py + 0.5, y + r), y + h - r);
        const dx = px + 0.5 - cx;
        const dy = py + 0.5 - cy;
        if (dx * dx + dy * dy <= r * r) this.set(px, py, color);
      }
    }
  }

  circle(cx, cy, r, color) {
    for (let py = Math.floor(cy - r); py <= Math.ceil(cy + r); py++) {
      for (let px = Math.floor(cx - r); px <= Math.ceil(cx + r); px++) {
        const dx = px + 0.5 - cx;
        const dy = py + 0.5 - cy;
        if (dx * dx + dy * dy <= r * r) this.set(px, py, color);
      }
    }
  }
}

const MINT = [139, 255, 180, 255];
const INK = [7, 16, 9, 255];
const GLASS = [13, 26, 18, 255];
const ACCENT = [70, 207, 131, 255];

// Design coordinates live in a 512x512 box, like web/icon.svg.
const BLOCKS = [
  [120, 192, 32, 32],
  [152, 192, 32, 32],
  [120, 224, 96, 32],
  [120, 256, 32, 64],
  [184, 288, 32, 32],
  [312, 208, 64, 32],
  [296, 240, 96, 32],
  [312, 272, 64, 32],
];

function draw(size, { maskable = false } = {}) {
  const canvas = new Canvas(size);
  const scale = (size / 512) * (maskable ? 0.74 : 1);
  const offset = (size - 512 * scale) / 2;
  const tx = (v) => offset + v * scale;
  const ts = (v) => v * scale;
  const rounded = (color) => (maskable ? 4 : 112);

  canvas.roundRect(0, 0, size, size, ts(rounded()), INK);

  // Screen shell with a mint bezel.
  canvas.roundRect(tx(72), tx(136), ts(368), ts(240), ts(44), MINT);
  canvas.roundRect(tx(80), tx(144), ts(352), ts(224), ts(38), GLASS);

  for (const [x, y, w, h] of BLOCKS) {
    canvas.roundRect(tx(x), tx(y), ts(w), ts(h), ts(3), MINT);
  }
  canvas.circle(tx(256), tx(424), ts(14), ACCENT);
  return canvas;
}

mkdirSync(outDir, { recursive: true });
for (const { file, size, maskable } of [
  { file: 'icon-192.png', size: 192 },
  { file: 'icon-512.png', size: 512 },
  { file: 'icon-maskable-512.png', size: 512, maskable: true },
]) {
  const canvas = draw(size, { maskable });
  const png = encodePng(size, size, canvas.data);
  writeFileSync(join(outDir, file), png);
  console.log(`${file}: ${size}x${size}, ${png.length} bytes`);
}
