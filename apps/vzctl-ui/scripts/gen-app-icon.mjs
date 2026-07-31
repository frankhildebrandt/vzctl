#!/usr/bin/env node
/**
 * Transparent vzctl app icon (1024² RGBA PNG).
 * Teal mark on alpha — no background plate.
 */
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const R = 15;
const G = 106;
const B = 90;

const pixels = new Float32Array(SIZE * SIZE); // alpha coverage 0..1

function addAlpha(x, y, a) {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE || a <= 0) return;
  const i = y * SIZE + x;
  pixels[i] = Math.min(1, pixels[i] + a);
}

function distSeg(px, py, ax, ay, bx, by) {
  const dx = bx - ax;
  const dy = by - ay;
  const len2 = dx * dx + dy * dy || 1;
  let t = ((px - ax) * dx + (py - ay) * dy) / len2;
  t = Math.max(0, Math.min(1, t));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
}

function strokePoly(points, width) {
  const half = width / 2;
  const pad = Math.ceil(half) + 2;
  let minX = SIZE, minY = SIZE, maxX = 0, maxY = 0;
  for (const [x, y] of points) {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  }
  minX = Math.max(0, Math.floor(minX - pad));
  minY = Math.max(0, Math.floor(minY - pad));
  maxX = Math.min(SIZE - 1, Math.ceil(maxX + pad));
  maxY = Math.min(SIZE - 1, Math.ceil(maxY + pad));

  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      let d = Infinity;
      for (let i = 0; i < points.length - 1; i++) {
        d = Math.min(d, distSeg(x, y, ...points[i], ...points[i + 1]));
      }
      if (d <= half) {
        const edge = half - d;
        addAlpha(x, y, edge < 1.25 ? edge / 1.25 : 1);
      }
    }
  }
}

function strokeRoundRect(x0, y0, x1, y1, radius, width) {
  const half = width / 2;
  const cx = (x0 + x1) / 2;
  const cy = (y0 + y1) / 2;
  const hx = (x1 - x0) / 2 - radius;
  const hy = (y1 - y0) / 2 - radius;
  const pad = Math.ceil(half) + 2;
  const minX = Math.max(0, Math.floor(x0 - pad));
  const minY = Math.max(0, Math.floor(y0 - pad));
  const maxX = Math.min(SIZE - 1, Math.ceil(x1 + pad));
  const maxY = Math.min(SIZE - 1, Math.ceil(y1 + pad));

  for (let y = minY; y <= maxY; y++) {
    for (let x = minX; x <= maxX; x++) {
      const dx = Math.abs(x - cx) - hx;
      const dy = Math.abs(y - cy) - hy;
      const outside = Math.hypot(Math.max(dx, 0), Math.max(dy, 0));
      const inside = Math.min(Math.max(dx, dy), 0);
      const sdf = Math.abs(outside + inside - radius);
      if (sdf <= half) {
        const edge = half - sdf;
        addAlpha(x, y, edge < 1.25 ? edge / 1.25 : 1);
      }
    }
  }
}

// Outer host frame + inner guest frame + V
strokeRoundRect(196, 196, 828, 828, 108, 48);
strokeRoundRect(310, 310, 714, 714, 78, 36);
strokePoly(
  [
    [330, 370],
    [512, 720],
    [694, 370],
  ],
  58,
);

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = c & 1 ? (0xedb88320 ^ (c >>> 1)) : c >>> 1;
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const typeBuf = Buffer.from(type);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])));
  return Buffer.concat([len, typeBuf, data, crcBuf]);
}

const rgba = Buffer.alloc(SIZE * SIZE * 4);
for (let i = 0; i < SIZE * SIZE; i++) {
  const a = Math.round(pixels[i] * 255);
  rgba[i * 4] = R;
  rgba[i * 4 + 1] = G;
  rgba[i * 4 + 2] = B;
  rgba[i * 4 + 3] = a;
}

const raw = Buffer.alloc((SIZE * 4 + 1) * SIZE);
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0;
  rgba.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8;
ihdr[9] = 6;

const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

const out = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../src-tauri/app-icon.png",
);
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, png);

// verify corners transparent
const corner = rgba.readUInt8(3);
console.log("wrote", out, "bytes", png.length, "cornerA", corner);
