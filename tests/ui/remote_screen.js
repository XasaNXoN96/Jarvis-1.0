// Chromium check of «Экран ПК» on the phone page: the picture is asked for again and again while the window is open; when the computer cannot take it,
// the real reason is written in the window (not an empty or unreadable alert); the window closes and the asking stops.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'remote_page.html'), 'utf8');
html = html.replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const PIX = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const run = async (mode, check) => {
    const ctx = await b.newContext({ viewport: { width: 400, height: 900 } });
    await ctx.addInitScript(() => { window.JarvisAndroid = { getToken: () => 'nativesecret' + 'y'.repeat(20), setToken() {}, battery: () => '{}', keepScreenOn() {}, version: () => '1' }; });
    const p = await ctx.newPage(); let screens = 0;
    await p.route('**/*', async r => {
      const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
      if (u.pathname === '/' && rq.method() === 'GET') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.pathname === '/screen') {
        screens++;
        if (mode === 'ok') return j({ ok: true, jpeg: PIX });
        if (mode === 'denied') return r.fulfill({ status: 403, contentType: 'text/plain', body: 'Это выключено в центре разрешений' });
        return r.fulfill({ status: 503, contentType: 'text/plain', body: 'экран не открывается для снимка (возможно, компьютер заблокирован)' });
      }
      if (u.pathname === '/live') return j({ input: { allowed: true, live: false }, camera: { allowed: true, live: false } });
      return j({ msgs: [], last: 0 });
    });
    await p.goto('http://pc.test/'); await p.waitForTimeout(400);
    await p.evaluate(() => { document.querySelectorAll('.tab').forEach(t => t.classList.remove('on')); document.getElementById('t-media').classList.add('on'); });
    await check(p, () => screens);
    await ctx.close();
  };
  await run('ok', async (p, n) => {
    await p.click('#scr-open'); await p.waitForSelector('#scr-img[src]', { timeout: 4000 });
    ok(await p.$eval('#scr-img', e => e.style.display !== 'none'), 'the picture is shown');
    await p.waitForTimeout(1800);
    ok(n() >= 2, 'the picture is asked for again while the window is open: ' + n());
    await p.click('#scr-close'); const k = n(); await p.waitForTimeout(2000);
    ok(await p.$('#scrbox') === null && n() === k, 'closing stops the asking: ' + k + ' / ' + n());
  });
  await run('fail', async (p, n) => {
    await p.click('#scr-open'); await p.waitForTimeout(600);
    const t = await p.textContent('#scr-msg');
    ok(/Экран пока не виден: экран не открывается для снимка/.test(t), 'the real reason is written: ' + t);
    ok(!/CLIXML/.test(t), 'no log header');
    await p.click('#scr-close');
  });
  await run('denied', async (p, n) => {
    await p.click('#scr-open'); await p.waitForTimeout(600);
    ok(/центре разрешений/.test(await p.textContent('#scr-msg')), 'a refused permission says where to switch it on');
    const k = n(); await p.waitForTimeout(1800);
    ok(n() === k, 'no more asking after a refusal');
  });
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'REMOTE SCREEN: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
