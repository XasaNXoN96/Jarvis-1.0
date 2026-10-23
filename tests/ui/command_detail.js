// Chromium check of the detail card of a command: opens from the button, shows every phrase and only the facts the command has, the phrases
// put text into the input field and send nothing, an unavailable command's phrases are disabled, Escape closes.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const launch = () => chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
const DATA = { language: 'ru', groups: [{ key: 'apps', emoji: '🚀', title: 'Программы', about: 'Запуск программ' }], commands: [
  { id: 'open_notepad', pack: 'apps-pack', group: 'apps', phrases: ['открой блокнот', 'запусти блокнот'], description: 'Открывает Блокнот', keywords: 'notepad текст', status: 'on', note: '', risk: 'high', level: 'premium' },
  { id: 'weather', pack: 'w', group: 'apps', phrases: ['покажи погоду'], description: '', keywords: '', status: 'unavailable', note: 'нужен город в настройках', risk: 'low', level: 'free' } ] };
(async () => {
  const b = await launch();
  for (const [w, h] of [[1280, 800], [400, 800]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h } });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
    const p = await ctx.newPage(); const sent = [];
    await p.route('**/*', r => { const rq = r.request(), u = rq.url();
      if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.endsWith('/commands')) return r.fulfill({ contentType: 'application/json', body: JSON.stringify(DATA) });
      if (rq.method() === 'POST') sent.push(u + ' ' + (rq.postData() || ''));
      return r.fulfill({ contentType: 'application/json', body: '{}' }); });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => { show('commands'); return loadCommands(true); }); await p.waitForTimeout(400);
    const t = w + 'x' + h;
    ok(await p.$$eval('#commands .cdet', a => a.length) === 2, t + ': every card has a detail button');
    ok(!(await p.$eval('#cd-dlg', e => e.classList.contains('open'))), t + ': closed at first');
    await p.click('#commands .ccard:nth-child(1) .cdet'); await p.waitForTimeout(200);
    ok(await p.$eval('#cd-dlg', e => e.classList.contains('open')), t + ': opens');
    ok(await p.textContent('#cd-title') === 'открой блокнот', t + ': title');
    const marks = await p.textContent('#cd-marks'), facts = await p.textContent('#cd-facts');
    ok(/РАБОТАЕТ/.test(marks) && /Premium/.test(marks) && /нужно подтверждение/.test(marks), t + ': marks: ' + marks);
    ok(/Программы/.test(facts) && /apps-pack/.test(facts) && /open_notepad/.test(facts) && /notepad текст/.test(facts) && /спросит/.test(facts), t + ': facts: ' + facts);
    ok(!/Пояснение/.test(facts), t + ': a fact the command does not have is not shown');
    ok(await p.$$eval('#cd-ph button', a => a.length) === 2, t + ': both phrases are listed');
    const o = await p.evaluate(() => { const r = document.querySelector('#cd-dlg .cd-box').getBoundingClientRect(); return [r.left >= 0, r.right <= innerWidth]; });
    ok(o[0] && o[1], t + ': the card fits the window');
    sent.length = 0;
    await p.click('#cd-ph button:nth-child(3)'); await p.waitForTimeout(250);
    ok(await p.inputValue('#h-input') === 'запусти блокнот', t + ': a phrase goes to the input field');
    ok(!(await p.$eval('#cd-dlg', e => e.classList.contains('open'))), t + ': the card closes after that');
    ok(!sent.some(x => /\/cmd|"name":"(cmd|run|command)/.test(x)), t + ': nothing was sent: ' + sent.join(';'));
    await p.evaluate(() => { show('commands'); });
    await p.click('#commands .ccard:nth-child(2) .cdet'); await p.waitForTimeout(200);
    ok(await p.$$eval('#cd-ph button:disabled', a => a.length) === 1 && await p.$eval('#cd-use', e => e.disabled), t + ': an unavailable command cannot be put in the field');
    ok(/нужен город/.test(await p.textContent('#cd-facts')), t + ': the reason is shown');
    await p.keyboard.press('Escape'); await p.waitForTimeout(150);
    ok(!(await p.$eval('#cd-dlg', e => e.classList.contains('open'))), t + ': Escape closes');
    await ctx.close();
  }
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'COMMAND DETAIL: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
