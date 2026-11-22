// Chromium check: ONE list of voices has the recorded packs (Luna 2.0, Jarvis OG …) and the voices that read any text; choosing a pack saves it (and plays a sample), the row of the voice
// that reads the rest of the text appears and can be changed on its own; choosing a real voice again leaves the pack mode. There is no second list of the recorded voices.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 1300, height: 900 } });
  const p = await ctx.newPage();
  const patches = [], actions = []; let S = { voice_name: '', piper: true, tts_enabled: true, voice_rate: 1, mic_devices: [], speech: { enabled: true, piper: true }, google_client_id: '', google_client_saved: false, google_connected: false, google_status: '' };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} window.__opened = []; window.open = u => { window.__opened.push(u); return null; }; });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/voices') return j([{ id: '', name: 'Автоматически' }]);
    if (u.pathname === '/settings' && rq.method() === 'POST') { const pa = JSON.parse(rq.postData() || '{}'); patches.push(Object.assign({}, pa)); if (pa.google_client_secret) { S.google_client_saved = true; delete pa.google_client_secret; } S = Object.assign({}, S, pa); return j({ ok: true }); }
    if (u.pathname === '/settings') return j(S);
    if (u.pathname === '/action') {
      const n = JSON.parse(rq.postData() || '{}').name; actions.push(n);
      if (n === 'google_connect') { if (!S.google_client_saved) return j({ ok: false, message: 'сначала сохраните идентификатор и секрет клиента Google' }); setTimeout(() => { S.google_connected = true; }, 500); return j({ ok: true, url: 'https://accounts.google.com/o/oauth2/v2/auth?x=1' }); }
      if (n === 'google_disconnect') { S.google_connected = false; S.google_status = 'Google отключён'; }
      return j({ ok: true });
    }
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'connect'; setSub['connect'] = 'google-card'; setView(); loadSettings(); }); await p.waitForTimeout(800);
  ok(await p.$('#google-card') !== null, 'the card of Google is on the page');
  ok((await p.$eval('#google-card small', e => e.textContent)).includes('Только чтение') && (await p.$eval('#google-card small', e => e.textContent)).includes('никогда не изменяются'), 'the card says that it only reads');
  ok(await p.$eval('#google-disconnect', e => e.disabled) && !(await p.$eval('#google-connect', e => e.disabled)), 'not connected: only «Подключить» is active');
  await p.click('#google-connect'); await p.waitForTimeout(500);
  ok((await p.$eval('#google-status', e => e.textContent)).includes('сначала сохраните'), 'without the client the page says what to do first');
  ok((await p.evaluate(() => window.__opened)).length === 0, 'no browser window is opened without the client');
  await p.fill('#google-id', 'abc.apps.googleusercontent.com'); await p.fill('#google-secret', 'GOCSPX-secret'); await p.click('#google-save'); await p.waitForTimeout(900);
  ok(patches.some(x => x.google_client_id === 'abc.apps.googleusercontent.com' && x.google_client_secret === 'GOCSPX-secret'), 'the client is saved: ' + JSON.stringify(patches));
  ok(await p.$eval('#google-secret', e => e.value) === '' && (await p.$eval('#google-secret', e => e.placeholder)).includes('секрет сохранён'), 'the secret is never shown again');
  await p.click('#google-connect'); await p.waitForTimeout(3500);
  ok((await p.evaluate(() => window.__opened))[0] === 'https://accounts.google.com/o/oauth2/v2/auth?x=1', 'the sign-in page of Google is opened');
  ok((await p.$eval('#google-status', e => e.textContent)).includes('Google подключён') && !(await p.$eval('#google-disconnect', e => e.disabled)), 'the page notices the connection');
  await p.click('#google-disconnect'); await p.waitForTimeout(900);
  ok(actions.includes('google_disconnect') && await p.$eval('#google-disconnect', e => e.disabled), 'it can be disconnected');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'GOOGLE CARD: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
