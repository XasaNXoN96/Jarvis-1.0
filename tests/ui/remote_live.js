// Chromium check of the phone page: the mouse/keyboard and camera of the computer start only on a press, send what the user touched, and say plainly
// when the computer refuses (the ability is off by default). Answers of the computer are faked here; the real routes are tested in remote.rs.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'remote_page.html'), 'utf8');
html = html.replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
// a 200x100 grey picture (a PNG is made here; the browser reads it whatever the label says)
const zlib = require('zlib');
const crcT = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = b => { let c = 0xffffffff; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(200, 0); ihdr.writeUInt32BE(100, 4); ihdr[8] = 8; ihdr[9] = 0;
const raw = Buffer.alloc(100 * 201, 128); for (let y = 0; y < 100; y++) raw[y * 201] = 0;
const PIX = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]).toString('base64');
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const run = async (name, allowed, check) => {
    const ctx = await b.newContext({ viewport: { width: 400, height: 900 } });
    await ctx.addInitScript(() => { window.JarvisAndroid = { getToken: () => 'nativesecret' + 'y'.repeat(20), setToken() {}, battery: () => '{}', keepScreenOn() {}, version: () => '1' }; });
    const p = await ctx.newPage(); const reqs = [];
    await p.route('**/*', async r => {
      const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
      if (u.pathname === '/' && rq.method() === 'GET') return r.fulfill({ contentType: 'text/html', body: html });
      reqs.push({ m: rq.method(), p: u.pathname, body: rq.postData() || '', h: rq.headers() });
      const deny = () => r.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ ok: false, message: 'Это выключено в центре разрешений: «тест». Включите в Настройки.' }) });
      if (['/input-start', '/camera-list', '/camera-start', '/input', '/camera-frame'].includes(u.pathname) && !allowed) return deny();
      if (u.pathname === '/input-start') return j({ ok: true });
      if (u.pathname === '/input') return j({ ok: true });
      if (u.pathname === '/input-stop' || u.pathname === '/camera-stop') return j({ ok: true });
      if (u.pathname === '/screen') return j({ ok: true, jpeg: PIX });
      if (u.pathname === '/camera-list') return j({ ok: true, cameras: ['Встроенная камера', 'USB камера'] });
      if (u.pathname === '/camera-start') return j({ ok: true, camera: 'USB камера' });
      if (u.pathname === '/camera-frame') return j({ ok: true, jpeg: PIX });
      return j({ msgs: [], last: 0 });
    });
    await p.goto('http://pc.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => { document.querySelectorAll('.tab').forEach(t => t.classList.remove('on')); document.getElementById('t-media').classList.add('on'); });
    await check(p, reqs, name);
    await ctx.close();
  };
  // 1. nothing starts by itself; the two buttons exist
  await run('idle', true, async (p, reqs, n) => {
    ok(await p.$('#rc-open') && await p.$('#cam-open'), n + ': both buttons exist');
    ok(!reqs.some(r => /input|camera/.test(r.p)), n + ': nothing is sent before a press: ' + reqs.map(r => r.p).join(','));
  });
  // 2. the ability is off: a plain refusal, no picture, no further requests
  await run('off', false, async (p, reqs, n) => {
    await p.click('#rc-open'); await p.waitForTimeout(400);
    ok((await p.textContent('#rc-msg')).includes('центре разрешений'), n + ': the refusal is told: ' + await p.textContent('#rc-msg'));
    ok(await p.$('#rc-img') === null, n + ': no picture of the screen when refused');
    await p.click('#rc-close'); await p.waitForTimeout(200);
    await p.click('#cam-open'); await p.waitForTimeout(400);
    ok((await p.textContent('#cam-msg')).includes('центре разрешений'), n + ': camera refusal: ' + await p.textContent('#cam-msg'));
    ok(!reqs.some(r => r.p === '/camera-start' || r.p === '/camera-frame'), n + ': the camera is not started when refused');
  });
  // 3. control: start, touch → click with the place, keys, text, scroll; every change carries the time and the number; close ends the session
  await run('control', true, async (p, reqs, n) => {
    await p.click('#rc-open'); await p.waitForSelector('#rc-img[src]', { timeout: 4000 });
    ok(reqs.some(r => r.p === '/input-start' && r.m === 'POST'), n + ': start was sent');
    const bb = await p.$eval('#rc-img', e => { const r = e.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; });
    await p.mouse.click(bb.x + bb.w * 0.25, bb.y + bb.h * 0.75); await p.waitForTimeout(250);
    let c = reqs.filter(r => r.p === '/input').map(r => JSON.parse(r.body));
    ok(c.length === 1 && c[0].kind === 'click' && Math.abs(c[0].x - 0.25) < 0.05 && Math.abs(c[0].y - 0.75) < 0.05, n + ': the touch gives the place: ' + JSON.stringify(c));
    await p.click('#rc-mode-right'); await p.mouse.click(bb.x + 5, bb.y + 5); await p.waitForTimeout(250);
    await p.click('#rc-key-enter'); await p.waitForTimeout(250);
    await p.click('#rc-down'); await p.waitForTimeout(250);
    await p.fill('#rc-text', 'привет'); await p.click('#rc-type'); await p.waitForTimeout(250);
    c = reqs.filter(r => r.p === '/input').map(r => JSON.parse(r.body));
    ok(c.some(x => x.kind === 'right') && c.some(x => x.kind === 'key' && x.key === 'enter') && c.some(x => x.kind === 'scroll' && x.lines === 3) && c.some(x => x.kind === 'text' && x.text === 'привет'), n + ': all kinds are sent: ' + JSON.stringify(c));
    ok(reqs.filter(r => r.m === 'POST' && /input/.test(r.p)).every(r => r.h['x-rid'] && r.h['x-ts']), n + ': each change carries the time and the number');
    await p.click('#rc-close'); await p.waitForTimeout(300);
    ok(reqs.some(r => r.p === '/input-stop'), n + ': closing ends the session on the computer');
    ok(await p.$('#rc-box') === null, n + ': the window is closed');
  });
  // 4. camera: choose, nothing runs until «Включить», frames come, closing switches it off
  await run('camera', true, async (p, reqs, n) => {
    await p.click('#cam-open'); await p.waitForSelector('#cam-start', { timeout: 3000 });
    ok((await p.$$eval('#cam-sel option', o => o.length)) === 2, n + ': both cameras are offered');
    ok(!reqs.some(r => r.p === '/camera-start' || r.p === '/camera-frame'), n + ': nothing runs before the press');
    await p.selectOption('#cam-sel', '1'); await p.selectOption('#cam-q', 'low'); await p.click('#cam-start');
    await p.waitForSelector('#cam-img[src]', { timeout: 4000 });
    const s = reqs.find(r => r.p === '/camera-start'); const sb = JSON.parse(s.body);
    ok(sb.index === 1 && sb.quality === 'low' && s.h['x-rid'], n + ': the choice is sent: ' + s.body);
    ok((await p.textContent('#cam-msg')).includes('включена'), n + ': the page says the camera is on');
    await p.click('#cam-close'); await p.waitForTimeout(300);
    ok(reqs.some(r => r.p === '/camera-stop'), n + ': closing switches the camera off');
  });
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'REMOTE LIVE: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
