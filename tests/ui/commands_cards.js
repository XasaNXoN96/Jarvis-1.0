// Chromium check of the cards of the Commands page: name, short description (from the pack or what to say), an explicit action that only puts the
// phrase into the input field, a disabled action with a reason for unavailable commands, no overflow.
// Run: node tests/ui/commands_cards.js  (from the repository root; needs playwright and a chromium)
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const DATA = { language: 'ru', groups: [{ key: 'apps', emoji: '🚀', title: 'Программы', about: 'Запуск программ' }], commands: [
  { id: 'a', pack: 'p1', group: 'apps', phrases: ['открой блокнот', 'запусти блокнот'], description: 'Открывает Блокнот', keywords: '', status: 'on', note: '', risk: 'low', level: 'free' },
  { id: 'b', pack: 'p2', group: 'apps', phrases: ['громкость {число}'], description: '', keywords: '', status: 'on', note: '', risk: 'low', level: 'free' },
  { id: 'c', pack: 'p3', group: 'apps', phrases: ['покажи погоду'], description: 'Погода', keywords: '', status: 'unavailable', note: 'нужен город в настройках', risk: 'low', level: 'free' },
  { id: 'd', pack: 'p4', group: 'apps', phrases: ['закрой блокнот'], description: 'Закрывает', keywords: '', status: 'on', note: '', risk: 'high', level: 'free' },
] };
const fails = [];
const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  for (const [w, h] of [[1280, 800], [400, 800]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h } });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
    const p = await ctx.newPage();
    const sent = [];
    await p.route('**/*', r => {
      const rq = r.request(), u = rq.url();
      if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.endsWith('/commands')) return r.fulfill({ contentType: 'application/json', body: JSON.stringify(DATA) });
      if (rq.method() === 'POST') sent.push(u + ' ' + (rq.postData() || ''));
      return r.fulfill({ contentType: 'application/json', body: '{}' });
    });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => { show('commands'); return loadCommands(true); }); await p.waitForTimeout(400);
    const cards = await p.$$eval('#commands .ccard', a => a.map(c => ({ name: c.querySelector('b').textContent, desc: c.querySelector('.cd').textContent, btn: c.querySelector('.cact') ? { dis: c.querySelector('.cact').disabled, title: c.querySelector('.cact').title, label: c.querySelector('.cact').getAttribute('aria-label') } : null })));
    const tag = `${w}x${h}`;
    ok(cards.length === 4, tag + ': 4 cards');
    ok(cards[0].desc.startsWith('Открывает Блокнот'), tag + ': the description of the pack is kept: ' + cards[0].desc);
    ok(cards[1].desc.startsWith('Скажите: «громкость …»') && !/Пакет:/.test(cards[1].desc), tag + ': without a description it says what to say: ' + cards[1].desc);
    ok(cards.every(c => c.btn && c.btn.label.includes(c.name)), tag + ': every card has an action with a clear label');
    ok(cards[2].btn.dis && /город/.test(cards[2].btn.title), tag + ': an unavailable command has a disabled action with the reason');
    ok(!cards[0].btn.dis && !cards[3].btn.dis, tag + ': the others are enabled');
    // the action only fills the field: nothing is sent
    sent.length = 0; // (the page makes its own background calls such as owner_status; only what the click does matters)
    await p.click('#commands .ccard:nth-child(1) .cact'); await p.waitForTimeout(300);
    ok(await p.inputValue('#h-input') === 'открой блокнот', tag + ': the phrase is in the input field');
    ok(!sent.some(x => /\/cmd|\/command|"name":"(cmd|run|command)/.test(x)), tag + ': the click did not run or send a command: ' + sent.join(';'));
    await p.evaluate(() => { show('commands'); });
    await p.click('#commands .ccard:nth-child(2) .cact'); await p.waitForTimeout(300);
    ok(await p.inputValue('#h-input') === 'громкость', tag + ': the slot is cut from the phrase');
    await p.evaluate(() => { show('commands'); });
    const o = await p.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: innerWidth }));
    ok(o.sw <= o.cw + 2, tag + ': no horizontal scroll');
    await ctx.close();
  }
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'COMMAND CARDS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
