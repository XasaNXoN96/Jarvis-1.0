// Chromium check of the sphere states «распознаю / ищу / выполняю»: they show from the state of the server, the stronger states
// (error, no connection, listening, speaking) stay stronger, an unknown word is ignored and the sphere returns to normal when the work ends.
// Run: node tests/ui/orb_states.js  (from the repository root; needs playwright and a chromium)
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = [];
const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 1100, height: 760 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage();
  await p.route('**/*', r => {
    const u = r.request().url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  const kind = (st, conn, act, extra) => p.evaluate(([st, conn, act, extra]) => { state = st; connected = conn; lastState = Object.assign({ activity: act }, extra || {}); return orbKind(); }, [st, conn, act, extra]);
  ok(await kind('idle', true, 'recognizing') === 'recognizing', 'recognizing shows');
  ok(await kind('idle', true, 'searching') === 'searching', 'searching shows');
  ok(await kind('idle', true, 'executing') === 'executing', 'executing shows');
  ok(await kind('idle', true, '') === 'idle', 'back to the normal state when the work ended');
  ok(await kind('idle', true, undefined) === 'idle', 'an old server without the field changes nothing');
  ok(await kind('idle', true, 'error') === 'idle' && await kind('idle', true, 'off') === 'idle' && await kind('idle', true, 'bogus') === 'idle', 'only the three words count');
  ok(await kind('error', true, 'executing') === 'error', 'an error is stronger than executing');
  ok(await kind('idle', false, 'executing') === 'off', 'no connection is stronger');
  ok(await kind('listening', true, 'executing') === 'listening', 'listening is stronger');
  ok(await kind('speaking', true, 'searching') === 'speaking', 'speaking is stronger');
  ok(await kind('busy', true, 'executing') === 'executing', 'a named activity is clearer than the general «думаю»');
  ok(await kind('busy', true, '') === 'thinking', 'the general «думаю» stays when there is no activity');
  // the title and the data attribute of the canvas follow
  await p.evaluate(() => { state = 'idle'; connected = true; lastState = { activity: 'searching' }; });
  await p.waitForTimeout(500);
  const t = await p.evaluate(() => { const c = document.querySelector('canvas[data-kind]'); return c ? [c.dataset.kind, c.title] : null; });
  ok(t && t[0] === 'searching' && /ищу/.test(t[1]), 'the canvas shows «ищу»: ' + JSON.stringify(t));
  await p.evaluate(() => { lastState = { activity: '' }; });
  await p.waitForTimeout(500);
  const t2 = await p.evaluate(() => document.querySelector('canvas[data-kind]').dataset.kind);
  ok(t2 === 'idle', 'the canvas is back to idle: ' + t2);
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'ORB STATES: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
