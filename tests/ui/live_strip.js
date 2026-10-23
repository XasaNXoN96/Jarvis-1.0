// Chromium check of the red strip of the window: it shows while the phone drives the mouse or watches the camera, names which, stays on top of
// everything, and its button ends both through the action «remote_live_stop».
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const seen = [];
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) { seen.push(JSON.parse(rq.postData() || '{}').name); return r.fulfill({ contentType: 'application/json', body: '{"ok":true,"message":"Управление и камера для телефона выключены."}' }); }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  const shown = () => p.$eval('#live-strip', e => getComputedStyle(e).display !== 'none');
  ok(!(await shown()), 'hidden while nothing runs');
  await p.evaluate(() => liveStrip({ input: true, camera: false }));
  ok(await shown() && (await p.textContent('#live-strip-t')).includes('мышью'), 'the mouse session is named');
  await p.evaluate(() => liveStrip({ input: true, camera: true }));
  const t = await p.textContent('#live-strip-t');
  ok(t.includes('мышью') && t.includes('веб-камеру'), 'both are named: ' + t);
  const top = await p.evaluate(() => { const e = document.elementFromPoint(300, 10); return e && !!e.closest('#live-strip'); });
  ok(top, 'the strip is on top of the window');
  await p.click('#live-strip-off'); await p.waitForTimeout(300);
  ok(seen.includes('remote_live_stop'), 'the button calls remote_live_stop: ' + seen.join(','));
  await p.evaluate(() => liveStrip({ input: false, camera: false }));
  ok(!(await shown()), 'hidden again when both ended');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'LIVE STRIP: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
