// Writes a synthetic test cover: a 300 × 450 PNG with a gradient and rings, no text or
// third-party art. Used by the capture scripts to show a custom cover and an automatic one.
// Usage: node scripts/make-test-cover.cjs OUT.png HUE
"use strict";
const fs = require("node:fs");
const zlib = require("node:zlib");

const [out, hueArg] = process.argv.slice(2);
if (!out) throw new Error("usage: make-test-cover.cjs OUT.png [HUE]");
const hue = Number(hueArg || 120) % 360;
const width = 300;
const height = 450;

function hsl(h, s, l) {
  const a = s * Math.min(l, 1 - l);
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
  };
  return [f(0), f(8), f(4)];
}

const rows = [];
for (let y = 0; y < height; y += 1) {
  const row = Buffer.alloc(1 + width * 3);
  for (let x = 0; x < width; x += 1) {
    const dx = x - width * 0.5;
    const dy = y - height * 0.42;
    const ring = Math.sin(Math.hypot(dx, dy) / 9) > 0.86 ? 0.18 : 0;
    const light = 0.12 + 0.38 * (1 - y / height) + ring;
    const [r, g, b] = hsl((hue + y / 6) % 360, 0.8, Math.min(0.85, light));
    row[1 + x * 3] = r;
    row[2 + x * 3] = g;
    row[3 + x * 3] = b;
  }
  rows.push(row);
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

const header = Buffer.alloc(13);
header.writeUInt32BE(width, 0);
header.writeUInt32BE(height, 4);
header[8] = 8; // bit depth
header[9] = 2; // RGB
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", header),
  chunk("IDAT", zlib.deflateSync(Buffer.concat(rows))),
  chunk("IEND", Buffer.alloc(0)),
]);
fs.writeFileSync(out, png);
