// Chromium check: a click on the sign of the voice says the REASON in words (it was only a tooltip), and an error opens Diagnostics; a working voice only says its state.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 800 } })).newPage();
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', r => r.request().url() === 'http://jarvis.test/' ? r.fulfill({ contentType: 'text/html', body: html }) : r.fulfill({ contentType: 'application/json', body: '{}' }));
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(400);
  await p.evaluate(() => show('settings'));
  await p.evaluate(() => voiceChip({ state: 'ERROR', detail: 'голос не запущен: нет модели распознавания речи' }));
  await p.click('.topbar .vchip'); await p.waitForTimeout(300);
  const t = await p.$eval('#toast', e => e.textContent);
  ok(t.includes('нет модели распознавания речи') && t.includes('Диагностику'), 'an error says its reason: ' + t);
  ok(await p.evaluate(() => currentPage) === 'diag', 'an error opens Diagnostics');
  await p.evaluate(() => show('settings'));
  await p.evaluate(() => voiceChip({ state: 'READY', detail: 'микрофон работает' }));
  await p.click('.topbar .vchip'); await p.waitForTimeout(200);
  ok((await p.$eval('#toast', e => e.textContent)).includes('микрофон работает') && await p.evaluate(() => currentPage) === 'settings', 'a working voice only says its state and stays on the page');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'VOICE CHIP: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
