// Chromium check of the phone page: the pairing code from a link («#c=…») only fills the field — nothing is sent until the owner presses
// the button; the secret of the device is taken from the Android application when it has one, and handed to it after pairing.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'remote_page.html'), 'utf8');
const orb = path.join(src, 'orbkit.js');
html = html.replace('/*ORBKIT*/', fs.readFileSync(orb, 'utf8'));
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const run = async (name, initScript, url, check) => {
    const ctx = await b.newContext({ viewport: { width: 400, height: 800 } });
    await ctx.addInitScript(initScript);
    const p = await ctx.newPage(); const reqs = [];
    await p.route('**/*', async r => {
      const rq = r.request(), u = new URL(rq.url());
      if (u.pathname === '/' && rq.method() === 'GET') return r.fulfill({ contentType: 'text/html', body: html });
      reqs.push(rq.method() + ' ' + u.pathname); if (rq.headers()['x-token']) reqs.push('WITH-TOKEN ' + u.pathname);
      if (u.pathname === '/pair') return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: true, token: 'devsecret' + 'x'.repeat(20) }) });
      return r.fulfill({ contentType: 'application/json', body: '{"msgs":[],"last":0}' });
    });
    await p.goto(url); await p.waitForTimeout(500);
    await check(p, reqs, name);
    await ctx.close();
  };
  // 1. no secret anywhere, a link with a code: the field is filled, no request to /pair, no data request at all
  await run('hint', () => { window.__set = []; }, 'http://pc.test/#pin=sha256/x&c=123456', async (p, reqs, n) => {
    ok(await p.$eval('#pcode', e => e.value) === '123456', n + ': the code from the link fills the field');
    ok(!reqs.some(r => r.includes('/pair')), n + ': nothing is sent to /pair before the button: ' + reqs.join(','));
    ok(!reqs.some(r => r.startsWith('WITH-TOKEN')), n + ': no request carries a secret (there is none yet): ' + reqs.join(','));
    ok(!(await p.url()).includes('c=123456'), n + ': the code is removed from the address bar');
  });
  // 2. a wrong code in the link is not taken
  await run('badhint', () => {}, 'http://pc.test/#c=12ab', async (p, reqs, n) => {
    ok(await p.$eval('#pcode', e => e.value) === '', n + ': a code that is not six digits is ignored');
  });
  // 3. pressing the button sends the code and hands the new secret to the application
  await run('pair', () => { window.__native = []; window.JarvisAndroid = { getToken: () => '', setToken: t => window.__native.push(t), battery: () => '{}', keepScreenOn() {}, version: () => '1' }; }, 'http://pc.test/#c=123456', async (p, reqs, n) => {
    await p.click('#pgo'); await p.waitForTimeout(600);
    ok(reqs.filter(r => r.includes('/pair')).length === 1, n + ': exactly one /pair request after the press');
    // (the page reloads after pairing; the native store got the secret)
  });
  // 4. the application already has the secret of this device (another address of the computer = empty localStorage): no pairing box
  await run('native', () => { window.JarvisAndroid = { getToken: () => 'nativesecret' + 'y'.repeat(20), setToken() {}, battery: () => '{}', keepScreenOn() {}, version: () => '1' }; }, 'http://pc.test/', async (p, reqs, n) => {
    ok(await p.$('#pcode') === null, n + ': no pairing box when the application holds the secret');
    ok(reqs.some(r => r.includes('/poll') || r.includes('/state')), n + ': the page works with the secret from the application: ' + reqs.join(','));
  });
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'REMOTE PAGE PAIRING: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
