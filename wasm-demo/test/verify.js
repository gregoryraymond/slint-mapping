// Browser smoke test for the wasm-demo. Loads the page, waits for
// the wasm module + the first round of OSM tiles to settle, takes a
// screenshot, and verifies the screenshot isn't just one flat
// colour (the failure mode we hit last time: dark grey background
// with no canvas rendered).
//
// Run from this directory:
//   node verify.js
//
// Requires a running static server (python3 -m http.server 8765
// from the dist/ directory).

import { chromium } from 'playwright';
import { writeFile } from 'fs/promises';
import { PNG } from 'pngjs';
import { createReadStream } from 'fs';

const URL = process.env.DEMO_URL ?? 'http://localhost:8765/';
const SCREENSHOT = process.env.SCREENSHOT_PATH ?? 'demo.png';
// How long to give the page after `init()` resolves so OSM tiles can
// fetch + decode + paint. 8s is plenty over localhost; the OSM CDN
// is the slow part.
const TILE_SETTLE_MS = 15000;

// Headless Chromium in Playwright defaults to SwiftShader for WebGL,
// but on some Windows boxes the context can get lost mid-init. The
// `--use-angle=swiftshader` flag forces ANGLE→SwiftShader explicitly
// which is more stable for WebGL in headless. Disabling the gpu
// process sandbox helps too.
const browser = await chromium.launch({
    headless: process.env.HEADED ? false : true,
});
const ctx = await browser.newContext({ viewport: { width: 900, height: 700 } });
const page = await ctx.newPage();

const consoleLines = [];
const pageErrors = [];
const tileRequests = []; // { url, status?, durationMs }

page.on('console', (msg) => {
    consoleLines.push(`[${msg.type()}] ${msg.text()}`);
});
page.on('pageerror', (err) => {
    pageErrors.push(`${err.name}: ${err.message}`);
});

const requestStarted = new Map();
page.on('request', (req) => {
    if (req.url().includes('tile.openstreetmap.org')) {
        requestStarted.set(req.url(), Date.now());
    }
});
page.on('response', (resp) => {
    const u = resp.url();
    if (u.includes('tile.openstreetmap.org')) {
        const t0 = requestStarted.get(u);
        tileRequests.push({
            url: u.replace('https://tile.openstreetmap.org/', ''),
            status: resp.status(),
            durationMs: t0 ? Date.now() - t0 : null,
        });
    }
});

console.log(`→ loading ${URL}`);
await page.goto(URL, { waitUntil: 'load' });

// The init() shim resolves once the wasm module is instantiated and
// the #[wasm_bindgen(start)] entry has run. After that we wait for
// the loading element to vanish (our index.html removes it on
// success) before letting tiles settle.
try {
    await page.waitForSelector('#loading', { state: 'detached', timeout: 15000 });
    console.log('✓ loading element removed (wasm init succeeded)');
} catch {
    console.error('✗ loading element never disappeared — wasm init likely failed');
}

// Wait for a <canvas> to appear, polling for up to 20s. Slint's
// winit-web backend may take a few frames to actually attach the
// canvas after init() resolves, and our previous fixed wait could
// miss it.
console.log('→ polling for canvas (up to 20s)');
try {
    await page.waitForSelector('canvas', { state: 'attached', timeout: 20000 });
    console.log('✓ canvas appeared in DOM');
} catch {
    console.error('✗ canvas never appeared within 20s');
}

console.log(`→ waiting ${TILE_SETTLE_MS}ms for tile fetches + paint`);
await page.waitForTimeout(TILE_SETTLE_MS);

// Slint inserts its own canvas into the body. Verify one exists and
// reports a non-zero size. Also dump a snapshot of what DID end up
// in <body> so we can see what Slint inserted (or didn't).
const canvasInfo = await page.evaluate(() => {
    // Look everywhere — document root, body, head, even shadow roots.
    // Old Slint examples sometimes wrap canvas in a div / put it in
    // documentElement, and we don't want to claim "no canvas" if it
    // just isn't directly under body.
    const allCanvases = Array.from(document.querySelectorAll('canvas')).map((c) => {
        const r = c.getBoundingClientRect();
        return {
            id: c.id || null,
            parentTag: c.parentElement?.tagName,
            width: r.width,
            height: r.height,
            drawingBufferWidth: c.width,
            drawingBufferHeight: c.height,
        };
    });
    const docOutline = document.documentElement.outerHTML
        .replace(/\s+/g, ' ')
        .slice(0, 600);
    return { allCanvases, docOutline };
});
console.log(`  canvases found: ${canvasInfo.allCanvases.length}`);
if (canvasInfo.allCanvases.length) {
    canvasInfo.allCanvases.forEach((c, i) =>
        console.log(`    [${i}] id=${c.id ?? '(none)'} parent=${c.parentTag} css=${c.width}×${c.height} buffer=${c.drawingBufferWidth}×${c.drawingBufferHeight}`)
    );
}
console.log(`  doc outline (first 600 chars): ${canvasInfo.docOutline}`);
// (The detailed canvas inspection already happens above.)

await page.screenshot({ path: SCREENSHOT, fullPage: false });
console.log(`✓ screenshot written to ${SCREENSHOT}`);

// Pixel-content check: count distinct colours in the screenshot. A
// fully blank canvas / dark background will have very few; a
// rendered map (OSM tiles + markers + polyline) will have hundreds.
const png = await new Promise((resolve, reject) => {
    const p = new PNG();
    createReadStream(SCREENSHOT).pipe(p)
        .on('parsed', () => resolve(p))
        .on('error', reject);
});
const colours = new Set();
for (let i = 0; i < png.data.length; i += 4) {
    // Quantise to 5-bit per channel so JPEG-ish noise doesn't make
    // every pixel "unique". 32^3 = 32768 max distinct colours.
    const r = png.data[i] >> 3;
    const g = png.data[i + 1] >> 3;
    const b = png.data[i + 2] >> 3;
    colours.add((r << 10) | (g << 5) | b);
}
console.log(`→ screenshot has ${colours.size} distinct quantised colours`);
// Verdict bands:
//   <10     totally blank canvas / single background colour
//   10-150  overlay layers painted (markers, polyline, placeholders)
//           but no actual tile imagery → fetch problem
//   >150    OSM imagery is on-screen with overlays
let verdict;
if (colours.size < 10) {
    verdict = `✗ canvas is essentially blank (${colours.size} colours) — Slint isn't painting at all`;
} else if (colours.size < 150) {
    verdict = `~ overlays render but tiles haven't loaded (${colours.size} colours) — fetch issue or just need more wait time`;
} else {
    verdict = `✓ tiles + overlays rendered (${colours.size} colours)`;
}
console.log(verdict);

console.log(`\n--- OSM tile fetches (${tileRequests.length}) ---`);
const statusCounts = tileRequests.reduce((m, r) => {
    m[r.status] = (m[r.status] ?? 0) + 1;
    return m;
}, {});
console.log('  by status:', JSON.stringify(statusCounts));
tileRequests.slice(0, 5).forEach((r) => {
    console.log(`  ${r.status} ${r.url} (${r.durationMs}ms)`);
});
if (tileRequests.length > 5) console.log(`  (+${tileRequests.length - 5} more)`);

console.log(`\n--- captured page console (${consoleLines.length} lines) ---`);
consoleLines.slice(0, 60).forEach((l) => console.log(l));
if (consoleLines.length > 60) console.log(`(+${consoleLines.length - 60} more lines)`);

console.log(`\n--- page errors (${pageErrors.length}) ---`);
pageErrors.forEach((l) => console.log(l));

await browser.close();

// Exit non-zero if any of the load-success checks failed.
const ok = canvasInfo.allCanvases.length > 0
    && colours.size > 150
    && pageErrors.length === 0
    && tileRequests.some((r) => r.status === 200);
process.exit(ok ? 0 : 1);
