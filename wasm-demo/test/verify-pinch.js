// Pinch-zoom verification. Loads the demo, waits for initial tiles
// at zoom 12, dispatches a synthetic pinch-out gesture via direct
// PointerEvent dispatch, then waits for tile fetches at a higher zoom
// level to confirm the demo actually zoomed in.
//
// Run with a static server on :8765:
//   node verify-pinch.js

import { chromium } from 'playwright';

const URL = process.env.DEMO_URL ?? 'http://localhost:8765/';
const SCREENSHOT_BEFORE = process.env.SCREENSHOT_BEFORE ?? 'pinch-before.png';
const SCREENSHOT_AFTER = process.env.SCREENSHOT_AFTER ?? 'pinch-after.png';

const browser = await chromium.launch({ headless: process.env.HEADED ? false : true });
const ctx = await browser.newContext({ viewport: { width: 900, height: 700 } });
const page = await ctx.newPage();

// Bucket tile requests by zoom level so we can prove the pinch
// actually changed the camera zoom (more z=13 tiles after pinching
// in from z=12).
const tilesByZoom = { initial: new Set(), afterPinch: new Set() };
let pinchHappened = false;
page.on('response', (resp) => {
    const u = resp.url();
    const m = u.match(/tile\.openstreetmap\.org\/(\d+)\/(\d+)\/(\d+)\.png/);
    if (m) {
        const bucket = pinchHappened ? tilesByZoom.afterPinch : tilesByZoom.initial;
        bucket.add(`${m[1]}/${m[2]}/${m[3]}`);
    }
});

page.on('console', (msg) => {
    const t = msg.text();
    if (t.includes('[pinch]') || t.includes('[wheel') || t.includes('[refresh]') || t.includes('[on_zoom_by]') || msg.type() === 'error') {
        console.log(`  [browser/${msg.type()}] ${t}`);
    }
});

console.log(`→ loading ${URL}`);
await page.goto(URL, { waitUntil: 'load' });
await page.waitForSelector('#loading', { state: 'detached', timeout: 15000 });
await page.waitForSelector('canvas', { state: 'attached', timeout: 10000 });
console.log('→ waiting 8s for initial z=12 tiles');
await page.waitForTimeout(8000);

await page.screenshot({ path: SCREENSHOT_BEFORE });
console.log(`✓ before screenshot: ${SCREENSHOT_BEFORE}`);
console.log(`  initial tile fetches: ${tilesByZoom.initial.size}`);
const initialZooms = new Set([...tilesByZoom.initial].map((k) => k.split('/')[0]));
console.log(`  initial zoom levels seen: ${[...initialZooms].join(', ')}`);

// Sanity check: confirm Playwright's mouse.wheel() actually moves
// the camera at all. If THIS doesn't trigger tile fetches, the issue
// is upstream of our pinch path (Slint's wheel handler isn't picking
// up wheel events, or zoom-by isn't wired, etc.).
console.log('→ sanity 1: dispatching real mouse-wheel via Playwright (3× -120 deltaY)');
await page.mouse.move(450, 350);
for (let i = 0; i < 3; i++) {
    await page.mouse.wheel(0, -120);
    await page.waitForTimeout(200);
}
await page.waitForTimeout(2000);
console.log(`  initial bucket: ${tilesByZoom.initial.size} tiles, zooms ${[...new Set([...tilesByZoom.initial].map((k) => k.split('/')[0]))].join(',')}`);

console.log('→ sanity 2: dispatching WheelEvent from page.evaluate (synthetic)');
await page.evaluate(() => {
    const canvas = document.getElementById('canvas');
    for (let i = 0; i < 3; i++) {
        canvas.dispatchEvent(new WheelEvent('wheel', {
            deltaY: -120,
            deltaMode: WheelEvent.DOM_DELTA_PIXEL,
            clientX: window.innerWidth / 2,
            clientY: window.innerHeight / 2,
            bubbles: true,
            cancelable: true,
        }));
    }
});
await page.waitForTimeout(2000);
console.log(`  initial bucket (still): ${tilesByZoom.initial.size} tiles`);

console.log('→ dispatching synthetic pinch-out from the centre');
pinchHappened = true;
await page.evaluate(async () => {
    const canvas = document.getElementById('canvas');
    const cx = window.innerWidth / 2;
    const cy = window.innerHeight / 2;
    const fire = (type, id, x, y) => canvas.dispatchEvent(new PointerEvent(type, {
        pointerId: id,
        pointerType: 'touch',
        clientX: x,
        clientY: y,
        bubbles: true,
        cancelable: true,
        isPrimary: id === 1,
    }));
    // Start with the pointers 80px apart, end 360px apart -> ~4.5×
    // expansion, which translates to ~2.2 zoom levels of zoom-in
    // (log2(4.5)). Plenty to push us from z=12 into z=14 territory.
    fire('pointerdown', 1, cx - 40, cy);
    fire('pointerdown', 2, cx + 40, cy);
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    for (let step = 1; step <= 16; step++) {
        await wait(16);
        const half = 40 + step * 10;
        fire('pointermove', 1, cx - half, cy);
        fire('pointermove', 2, cx + half, cy);
    }
    fire('pointerup', 1, cx - 200, cy);
    fire('pointerup', 2, cx + 200, cy);
});

console.log('→ waiting 8s for post-pinch tile fetches');
await page.waitForTimeout(8000);

// Pump several animation frames so any pending paints settle, and
// poke the canvas via a property the renderer watches. Slint on
// wasm sometimes leaves the WebGL drawing buffer un-presented until
// something nudges it; without this the screenshot can capture
// stale pixels even though set_tiles already ran with the new
// model.
await page.evaluate(async () => {
    const raf = () => new Promise((r) => requestAnimationFrame(r));
    for (let i = 0; i < 10; i++) await raf();
});

await page.screenshot({ path: SCREENSHOT_AFTER });
console.log(`✓ after screenshot: ${SCREENSHOT_AFTER}`);
console.log(`  post-pinch tile fetches: ${tilesByZoom.afterPinch.size}`);
const afterZooms = new Set([...tilesByZoom.afterPinch].map((k) => k.split('/')[0]));
console.log(`  post-pinch zoom levels seen: ${[...afterZooms].join(', ')}`);

const maxBefore = Math.max(...[...initialZooms].map(Number), 0);
const maxAfter = Math.max(...[...afterZooms].map(Number), 0);
const ok = maxAfter > maxBefore;
console.log(
    ok
        ? `✓ pinch zoomed in: max zoom went from ${maxBefore} to ${maxAfter}`
        : `✗ pinch had no effect: max zoom stayed at ${maxBefore} (after: ${maxAfter})`
);

await browser.close();
process.exit(ok ? 0 : 1);
