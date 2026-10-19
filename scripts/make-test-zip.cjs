// Writes a stored (uncompressed) ZIP of the given files, by base name, so the capture scripts
// can build a synthetic test game without a zip tool.
// Usage: node scripts/make-test-zip.cjs OUT.zip FILE...
"use strict";
const fs = require("node:fs");
const path = require("node:path");

const [out, ...inputs] = process.argv.slice(2);
if (!out || !inputs.length) throw new Error("usage: make-test-zip.cjs OUT.zip FILE...");

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buffer) => {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

const parts = [];
const central = [];
let offset = 0;
for (const input of inputs) {
  const name = Buffer.from(path.basename(input), "utf8");
  const data = fs.readFileSync(input);
  const crc = crc32(data);
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt32LE(crc, 14);
  local.writeUInt32LE(data.length, 18);
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(name.length, 26);
  parts.push(local, name, data);
  const entry = Buffer.alloc(46);
  entry.writeUInt32LE(0x02014b50, 0);
  entry.writeUInt16LE(20, 4);
  entry.writeUInt16LE(20, 6);
  entry.writeUInt32LE(crc, 16);
  entry.writeUInt32LE(data.length, 20);
  entry.writeUInt32LE(data.length, 24);
  entry.writeUInt16LE(name.length, 28);
  entry.writeUInt32LE(offset, 42);
  central.push(entry, name);
  offset += local.length + name.length + data.length;
}
const directory = Buffer.concat(central);
const end = Buffer.alloc(22);
end.writeUInt32LE(0x06054b50, 0);
end.writeUInt16LE(inputs.length, 8);
end.writeUInt16LE(inputs.length, 10);
end.writeUInt32LE(directory.length, 12);
end.writeUInt32LE(offset, 16);
fs.writeFileSync(out, Buffer.concat([...parts, directory, end]));
