// Chromium check of the touchpad of the phone page: one finger moves the pointer of the computer, a short touch is a left click, two fingers
// moving up or down turn the wheel. Touches are real multi-touch events (the DevTools protocol). The answers of the computer are faked here.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'remote_page.html'), 'utf8');
html = html.replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 800, height: 1100 }, hasTouch: true, isMobile: true });
  await ctx.addInitScript(() => { window.JarvisAndroid = { getToken: () => 'nativesecret' + 'y'.repeat(20), setToken() {}, battery: () => '{}', keepScreenOn() {}, version: () => '1' }; });
  const p = await ctx.newPage(); const sent = [];
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.pathname === '/' && rq.method() === 'GET') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/input') { sent.push(JSON.parse(rq.postData() || '{}')); return j({ ok: true }); }
    if (u.pathname === '/live') return j({ input: { allowed: true, live: false }, camera: { allowed: true, live: false } });
    if (['/input-start', '/input-stop'].includes(u.pathname)) return j({ ok: true });
    if (u.pathname === '/screen') return j({ ok: false, message: 'снимок экрана не получился' });
    return j({ msgs: [], last: 0 });
  });
  await p.goto('http://pc.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { document.querySelectorAll('.tab').forEach(t => t.classList.remove('on')); document.getElementById('t-media').classList.add('on'); });
  await p.click('#rc-open'); await p.waitForSelector('#rc-pad', { timeout: 4000 });
  const cdp = await ctx.newCDPSession(p);
  const bb = await p.$eval('#rc-pad', e => { const r = e.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; });
  const touch = (type, pts) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: pts.map((q, i) => ({ x: q[0], y: q[1], id: i + 1 })) });
  const cx = bb.x + bb.w / 2, cy = bb.y + bb.h / 2;
  const sum = k => sent.filter(s => s.kind === k).reduce((a, s) => [a[0] + (s.dx || 0), a[1] + (s.dy || 0), a[2] + (s.lines || 0)], [0, 0, 0]);
  // 1. one finger moving right and down: the pointer moves the same way, a bit faster than the finger (normal speed), no click
  await touch('touchStart', [[cx, cy]]);
  for (let i = 1; i <= 10; i++) { await touch('touchMove', [[cx + i * 10, cy + i * 5]]); await p.waitForTimeout(30); }
  await touch('touchEnd', []);
  await p.waitForTimeout(300);
  let m = sum('move');
  ok(m[0] > 100 && m[0] <= 200 && m[1] > 50 && m[1] <= 100, 'the pointer moved with the finger: ' + m);
  ok(Math.abs(m[0] / m[1] - 2) < 0.3, 'the direction is kept: ' + m);
  ok(!sent.some(s => s.kind === 'tap'), 'a drag is not a click');
  ok(sent.filter(s => s.kind === 'move').every(s => Math.abs(s.dx) <= 300 && Math.abs(s.dy) <= 300), 'every step is within the limit');
  // 2. a short touch: one left click where the pointer stands
  sent.length = 0;
  await touch('touchStart', [[cx, cy]]); await p.waitForTimeout(60); await touch('touchEnd', []);
  await p.waitForTimeout(300);
  ok(sent.filter(s => s.kind === 'tap').length === 1 && sent.length === 1, 'a short touch is one tap: ' + JSON.stringify(sent));
  // 3. two quick touches: two taps (Windows makes the double click of them)
  sent.length = 0;
  for (let i = 0; i < 2; i++) { await touch('touchStart', [[cx, cy]]); await p.waitForTimeout(50); await touch('touchEnd', []); await p.waitForTimeout(120); }
  await p.waitForTimeout(200);
  ok(sent.filter(s => s.kind === 'tap').length === 2, 'two quick touches are two taps: ' + JSON.stringify(sent));
  // 4. two fingers moving down turn the wheel (the page follows the fingers: positive = up), two fingers up turn it the other way; no pointer move, no click
  sent.length = 0;
  await touch('touchStart', [[cx - 40, cy], [cx + 40, cy]]);
  for (let i = 1; i <= 8; i++) { await touch('touchMove', [[cx - 40, cy + i * 20], [cx + 40, cy + i * 20]]); await p.waitForTimeout(30); }
  await touch('touchEnd', []);
  await p.waitForTimeout(400);
  let sc = sum('scroll');
  ok(sc[2] >= 5 && sc[2] <= 9, 'two fingers down: lines of the wheel: ' + sc);
  ok(!sent.some(s => s.kind === 'move' || s.kind === 'tap'), 'two fingers neither move the pointer nor click: ' + JSON.stringify(sent.filter(s => s.kind !== 'scroll')));
  sent.length = 0;
  await touch('touchStart', [[cx - 40, cy + 100], [cx + 40, cy + 100]]);
  for (let i = 1; i <= 8; i++) { await touch('touchMove', [[cx - 40, cy + 100 - i * 20], [cx + 40, cy + 100 - i * 20]]); await p.waitForTimeout(30); }
  await touch('touchEnd', []);
  await p.waitForTimeout(400);
  sc = sum('scroll');
  ok(sc[2] <= -5 && sc[2] >= -9, 'two fingers up: the other way: ' + sc);
  // 5. the pad does not scroll the page under it and the old buttons are still there
  ok(await p.$('#rc-mode-click') && await p.$('#rc-up') && await p.$('#rc-key-enter') && await p.$('#rc-text'), 'the buttons stay as a second way');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'TOUCHPAD: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
