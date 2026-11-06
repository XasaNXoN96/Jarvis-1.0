// Chromium check: a click on a message of the page «Сообщения» asks Jarvis to open the program it came from (by the number of the message only);
// the button 🔊 reads it aloud and does NOT open anything; the answer of Jarvis is said in a toast.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 800 } })).newPage();
  const calls = [];
  const DATA = { messages: [{ id: 7, at: 1793000000, net: 'telegram', app: 'Telegram', title: 'Telegram · Мама', text: 'Привет!' }, { id: 8, at: 1793000100, net: '', app: 'Почта', title: 'Почта · Банк', text: 'Код 1234' }], networks: [{ key: 'telegram', title: 'Telegram' }], counts: { telegram: 1, other: 1 } };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url());
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/messages' && rq.method() === 'GET') return r.fulfill({ contentType: 'application/json', body: JSON.stringify(DATA) });
    if (u.pathname === '/messages' && rq.method() === 'POST') { const body = JSON.parse(rq.postData() || '{}'); calls.push(body); return r.fulfill({ contentType: 'application/json', body: JSON.stringify(body.op === 'open' ? { ok: true, message: 'Открываю Telegram Desktop' } : { ok: true }) }); }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(400);
  await p.evaluate(() => show('messages'));
  await p.waitForTimeout(200);
  await p.click('#msg-list .msg-row:first-child .x'); await p.waitForTimeout(300);
  ok(calls.length === 1 && calls[0].op === 'open' && calls[0].id === 7, 'a click on the message asks to open it by its number: ' + JSON.stringify(calls));
  ok(!JSON.stringify(calls).includes('Telegram') && !/launch|path|url/.test(JSON.stringify(calls)), 'the page sends no program name, path or address');
  ok((await p.$eval('#toast', e => e.textContent)).includes('Открываю Telegram Desktop'), 'the answer is said in a toast');
  calls.length = 0;
  await p.click('#msg-list .msg-row:first-child button'); await p.waitForTimeout(300);
  ok(calls.length === 1 && calls[0].op === 'speak' && calls[0].id === 7, 'the button of the voice only reads aloud and does not open: ' + JSON.stringify(calls));
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'MESSAGE OPEN: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
