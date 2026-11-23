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
  const p = await (await b.newContext({ viewport: { width: 1300, height: 900 } })).newPage();
  const patches = []; let S = { voice_name: '', piper: true, tts_enabled: true, voice_rate: 1, mic_devices: [], speech: { enabled: true, piper: true }, mcp_enabled: false };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/voices') return j([{ id: '', name: 'Автоматически' }]);
    if (u.pathname === '/settings' && rq.method() === 'POST') { const pa = JSON.parse(rq.postData() || '{}'); patches.push(pa); S = Object.assign({}, S, pa); return j({ ok: true }); }
    if (u.pathname === '/settings') return j(S);
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'connect'; setSub['connect'] = 'mcp-card'; setView(); loadSettings(); }); await p.waitForTimeout(800);
  ok(await p.$('#mcp-card') !== null, 'the card is on the page');
  ok(!(await p.$eval('#mcp-on', e => e.checked)), 'off by default');
  const hint = await p.$eval('#mcp-card small', e => e.textContent);
  ok(hint.includes('Выключено по умолчанию') && hint.includes('--mcp') && hint.includes('30 в минуту') && hint.includes('удаление'), 'the card says how it starts and what is limited: ' + hint.slice(0, 80));
  await p.check('#mcp-on'); await p.waitForTimeout(500);
  ok(patches.some(x => x.mcp_enabled === true), 'the switch is saved');
  await p.evaluate(() => loadSettings()); await p.waitForTimeout(500);
  ok(await p.$eval('#mcp-on', e => e.checked), 'it stays on after the settings are read again');
  await p.uncheck('#mcp-on'); await p.waitForTimeout(500);
  ok(patches.some(x => x.mcp_enabled === false), 'it can be switched off');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'MCP CARD: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
