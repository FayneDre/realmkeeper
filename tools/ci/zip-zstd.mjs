#!/usr/bin/env node
// Packs a directory into a ZIP whose entries are zstd-compressed (method 93,
// APPNOTE 6.3.7). The launcher reads it with the `zip` crate (`zstd`
// feature): about a third smaller than deflate for server builds and
// database snapshots, with the same extraction speed (measured 2026-11-14).
//
// Deterministic: sorted entries, fixed timestamps, no extra fields, so the
// same tree always gives the same bytes (and the same sha256 in the index).
// Directories get their own entries (empty ones matter in a MySQL datadir).
//
// --unix-modes (Linux builds, ADR 0031): entries record Unix permissions
// (0755 for executables and directories, 0644 otherwise), so that servers
// unpacked on Linux can run; without it the archive is unchanged.
//
// Usage: node tools/ci/zip-zstd.mjs <dir> <out.zip> [--level 19] [--unix-modes]

import { closeSync, openSync, readFileSync, readdirSync, statSync, writeSync } from "node:fs";
import { join } from "node:path";
import { constants, crc32, zstdCompressSync } from "node:zlib";

const args = process.argv.slice(2);
const unixIndex = args.indexOf("--unix-modes");
const unixModes = unixIndex >= 0;
if (unixModes) args.splice(unixIndex, 1);
const levelIndex = args.indexOf("--level");
const level = levelIndex >= 0 ? Number(args.splice(levelIndex, 2)[1]) : 19;
const [dir, out] = args;
if (!dir || !out || !Number.isInteger(level)) {
  console.error("usage: zip-zstd.mjs <dir> <out.zip> [--level 1-22] [--unix-modes]");
  process.exit(2);
}

const ZSTD = 93;
const STORED = 0;
const VERSION = 63; // 6.3: zstd
const UTF8 = 0x0800;
const DOS_TIME = 0; // 00:00:00
const DOS_DATE = (0 << 9) | (1 << 5) | 1; // 1980-01-01
const LIMIT = 0xffffffff;

/** Every directory and file under `root`, relative, `/`-separated, sorted. */
function walk(root, prefix = "") {
  const entries = [];
  for (const name of readdirSync(join(root, prefix)).sort()) {
    const rel = prefix ? `${prefix}/${name}` : name;
    if (statSync(join(root, rel)).isDirectory()) {
      entries.push({ name: `${rel}/`, dir: true });
      entries.push(...walk(root, rel));
    } else {
      entries.push({ name: rel, dir: false, executable: (statSync(join(root, rel)).mode & 0o111) !== 0 });
    }
  }
  return entries;
}

function u16(n) {
  const b = Buffer.alloc(2);
  b.writeUInt16LE(n);
  return b;
}
function u32(n) {
  if (n > LIMIT) throw new Error("archive needs ZIP64, which this packer does not write");
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
}

const fd = openSync(out, "w");
let offset = 0;
const write = (buffer) => {
  writeSync(fd, buffer);
  offset += buffer.length;
};

const central = [];
let raw = 0;
const started = Date.now();
for (const entry of walk(dir)) {
  const name = Buffer.from(entry.name, "utf8");
  const data = entry.dir ? Buffer.alloc(0) : readFileSync(join(dir, entry.name));
  const method = entry.dir ? STORED : ZSTD;
  const packed = entry.dir
    ? data
    : zstdCompressSync(data, { params: { [constants.ZSTD_c_compressionLevel]: level, [constants.ZSTD_c_checksumFlag]: 1 } });
  const crc = crc32(data);
  raw += data.length;
  const fields = [u16(VERSION), u16(UTF8), u16(method), u16(DOS_TIME), u16(DOS_DATE), u32(crc), u32(packed.length), u32(data.length), u16(name.length), u16(0)];
  const headerOffset = offset;
  write(Buffer.concat([u32(0x04034b50), ...fields, name]));
  write(packed);
  central.push(
    Buffer.concat([
      u32(0x02014b50),
      // made by: MS-DOS attributes, or Unix (3) with permissions in the high word
      u16(unixModes ? (3 << 8) | VERSION : VERSION),
      ...fields,
      u16(0), // comment
      u16(0), // disk
      u16(0), // internal attributes
      // external: directory flag, plus the Unix mode with --unix-modes
      u32(
        ((unixModes ? (entry.dir ? 0o40755 : entry.executable ? 0o100755 : 0o100644) * 0x10000 : 0) +
          (entry.dir ? 0x10 : 0)) >>>
          0,
      ),
      u32(headerOffset),
      name,
    ]),
  );
}
const directory = Buffer.concat(central);
const directoryOffset = offset;
write(directory);
if (central.length > 0xffff) throw new Error("too many entries for a non-ZIP64 archive");
write(
  Buffer.concat([
    u32(0x06054b50),
    u16(0),
    u16(0),
    u16(central.length),
    u16(central.length),
    u32(directory.length),
    u32(directoryOffset),
    u16(0),
  ]),
);
closeSync(fd);
console.log(
  `${out}: ${central.length} entries, ${(raw / 1e6).toFixed(1)} MB -> ${(offset / 1e6).toFixed(1)} MB ` +
    `(zstd ${level}, ${((Date.now() - started) / 1000).toFixed(1)} s)`,
);
