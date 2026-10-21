// Writes a synthetic test cover: a 300 × 450 PNG painted here, pixel by pixel, as a small
// illustrated scene (a sky, a glowing sun or moon, layered ridges, a lake, and stars), with no text
// or third-party art. Used by the capture scripts to show a custom cover and an automatic one.
// With WIDTH and HEIGHT it paints the same kind of scene in another shape (the store fixtures use
// wide ones as stand-ins for Steam's hero and header art). With --logo it writes a transparent PNG
// with a plain geometric emblem instead, a stand-in for a game's logo.
// Usage: node scripts/make-test-cover.cjs OUT.png [HUE] [WIDTH HEIGHT] [--logo]
"use strict";
const fs = require("node:fs");
const zlib = require("node:zlib");

const args = process.argv.slice(2);
const logo = args.includes("--logo");
const [out, hueArg, widthArg, heightArg] = args.filter((arg) => arg !== "--logo");
if (!out) throw new Error("usage: make-test-cover.cjs OUT.png [HUE] [WIDTH HEIGHT] [--logo]");
const hue = Number(hueArg || 120) % 360;
const width = Number(widthArg) || 300;
const height = Number(heightArg) || 450;

function hsl(h, s, l) {
  const a = s * Math.min(l, 1 - l);
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return 255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)));
  };
  return [f(0), f(8), f(4)];
}
const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);
const smooth = (e0, e1, x) => { const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0))); return t * t * (3 - 2 * t); };

// Deterministic per-hue details.
let seed = hue * 7919 + 17;
const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
const sunX = width * (0.3 + random() * 0.4);
const sunY = height * 0.36;
const sunR = Math.min(width, height * 0.66) * 0.13;
const phases = Array.from({ length: 6 }, () => random() * Math.PI * 2);
const stars = Array.from({ length: 70 }, () => [random() * width, random() * height * 0.45, random()]);
const horizon = height * 0.68;

const skyTop = hsl((hue + 230) % 360, 0.55, 0.1);
const skyLow = hsl((hue + 10) % 360, 0.85, 0.55);
const sun = hsl((hue + 40) % 360, 1, 0.85);
const far = hsl((hue + 300) % 360, 0.35, 0.32);
const near = hsl((hue + 260) % 360, 0.45, 0.1);
const water = hsl((hue + 220) % 360, 0.5, 0.14);

function ridge(x, base, amp, freq, k) {
  const t = x / width;
  return base - amp * (0.6 * Math.sin(t * freq + phases[k]) + 0.3 * Math.sin(t * freq * 2.3 + phases[k + 1]) + 0.1 * Math.sin(t * freq * 6.1 + phases[k + 2]));
}

const rows = [];
for (let y = 0; y < height && !logo; y += 1) {
  const row = Buffer.alloc(1 + width * 3);
  for (let x = 0; x < width; x += 1) {
    // Mirror the sky in the lake below the horizon, darkened and rippled.
    const reflected = y > horizon;
    const sy = reflected ? horizon - (y - horizon) * 0.9 + Math.sin(y * 0.6 + x * 0.05) * 2 : y;
    let color = mix(skyTop, skyLow, smooth(0, horizon, sy));
    for (const [sx, syy, b] of stars) if (!reflected && Math.abs(x - sx) < 1 && Math.abs(y - syy) < 1) color = mix(color, [255, 255, 255], 0.4 + b * 0.6);
    const d = Math.hypot(x - sunX, sy - sunY);
    color = mix(color, sun, Math.max(smooth(sunR * 3.2, 0, d) * 0.45, d < sunR ? 1 : 0));
    if (sy > ridge(x, horizon - 40, 55, 7, 0)) color = mix(color, far, 0.85);
    if (sy > ridge(x, horizon - 8, 30, 11, 3)) color = near;
    if (reflected) color = mix(color, water, 0.55);
    // A soft vignette.
    const v = Math.hypot((x - width / 2) / width, (y - height / 2) / height);
    color = color.map((c) => c * (1 - smooth(0.35, 0.75, v) * 0.5));
    row[1 + x * 3] = Math.round(Math.min(255, Math.max(0, color[0])));
    row[2 + x * 3] = Math.round(Math.min(255, Math.max(0, color[1])));
    row[3 + x * 3] = Math.round(Math.min(255, Math.max(0, color[2])));
  }
  rows.push(row);
}

// The logo stand-in: a ring and a row of blocks, opaque on a transparent background.
if (logo) {
  const color = hsl((hue + 40) % 360, 0.9, 0.62);
  const edge = hsl((hue + 40) % 360, 0.6, 0.12);
  const unit = height;
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(1 + width * 4);
    for (let x = 0; x < width; x += 1) {
      const d = Math.abs(Math.hypot(x - unit * 0.5, y - unit * 0.5) - unit * 0.34);
      const bx = x - unit * 1.05;
      const block = Math.floor(bx / (unit * 0.42));
      const inBlock = bx > 0 && x < width - unit * 0.08 && bx - block * unit * 0.42 < unit * 0.34;
      const bar = inBlock && Math.abs(y - unit * 0.5) < unit * (block % 2 ? 0.14 : 0.26);
      const fill = d < unit * 0.07 || bar;
      const outline = !fill && (d < unit * 0.1 || (inBlock && Math.abs(y - unit * 0.5) < unit * (block % 2 ? 0.17 : 0.29)));
      const rgb = fill ? color : edge;
      row[1 + x * 4] = Math.round(rgb[0]);
      row[2 + x * 4] = Math.round(rgb[1]);
      row[3 + x * 4] = Math.round(rgb[2]);
      row[4 + x * 4] = fill || outline ? 255 : 0;
    }
    rows.push(row);
  }
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
header[9] = logo ? 6 : 2; // RGBA for the logo, else RGB
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", header),
  chunk("IDAT", zlib.deflateSync(Buffer.concat(rows))),
  chunk("IEND", Buffer.alloc(0)),
]);
fs.writeFileSync(out, png);
