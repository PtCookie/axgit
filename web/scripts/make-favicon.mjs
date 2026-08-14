#!/usr/bin/env node
// Regenerates public/favicon.ico as a real ICO container (16x16 + 32x32)
// from public/favicon.svg — the previous file was a 32x32 PNG with an .ico
// extension, not an actual ICO (docs/DECISIONS.md #84). Rasterizes with the
// Playwright already installed as a devDependency here (no new dependency
// for a one-time asset build) rather than writing a second SVG renderer;
// the ICO container itself is hand-built since modern ICO/Windows/browsers
// accept PNG-format icon entries directly, which needs only a small
// ICONDIR/ICONDIRENTRY header — see build_ico below. Lives under web/
// (rather than the repo-root scripts/, which has no package.json of its
// own) so plain `node` resolves the `playwright` import via web/node_modules
// without extra flags.
//
// Run: pnpm --filter web gen:favicon

import { chromium } from "playwright";
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const WEB_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SVG_PATH = path.join(WEB_ROOT, "public/favicon.svg");
const ICO_PATH = path.join(WEB_ROOT, "public/favicon.ico");
const SIZES = [16, 32];

async function rasterize(browser, svg, size) {
  const page = await browser.newPage({ viewport: { width: size, height: size } });
  try {
    await page.setContent(
      `<!doctype html><html><body style="margin:0;width:${size}px;height:${size}px">${svg}</body></html>`,
    );
    await page.locator("svg").evaluate((el, s) => {
      el.setAttribute("width", String(s));
      el.setAttribute("height", String(s));
    }, size);
    return await page.screenshot({ omitBackground: true });
  } finally {
    await page.close();
  }
}

// Builds an ICO file embedding each PNG buffer directly as its icon entry
// (`ICONDIRENTRY.bitCount = 32`, `bytesInRes` = the PNG's own byte length) —
// the format every current browser/OS accepts, so there's no need to also
// implement the older raw-BMP-DIB icon encoding.
function buildIco(pngsBySize) {
  const count = pngsBySize.length;
  const headerSize = 6 + 16 * count;
  let offset = headerSize;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(count, 4);

  const entries = [];
  for (const { size, png } of pngsBySize) {
    const entry = Buffer.alloc(16);
    entry.writeUInt8(size >= 256 ? 0 : size, 0); // width (0 means 256)
    entry.writeUInt8(size >= 256 ? 0 : size, 1); // height
    entry.writeUInt8(0, 2); // color count (0 = no palette, true color)
    entry.writeUInt8(0, 3); // reserved
    entry.writeUInt16LE(1, 4); // color planes
    entry.writeUInt16LE(32, 6); // bits per pixel
    entry.writeUInt32LE(png.length, 8); // size of the image data
    entry.writeUInt32LE(offset, 12); // offset from the start of the file
    offset += png.length;
    entries.push(entry);
  }

  return Buffer.concat([header, ...entries, ...pngsBySize.map(({ png }) => png)]);
}

async function main() {
  const svg = await readFile(SVG_PATH, "utf8");
  const browser = await chromium.launch();
  try {
    const pngsBySize = [];
    for (const size of SIZES) {
      const png = await rasterize(browser, svg, size);
      pngsBySize.push({ size, png });
    }
    const ico = buildIco(pngsBySize);
    await writeFile(ICO_PATH, ico);
    console.log(`wrote ${ICO_PATH} (${SIZES.join("x, ")}x — ${ico.length} bytes)`);
  } finally {
    await browser.close();
  }
}

await main();
