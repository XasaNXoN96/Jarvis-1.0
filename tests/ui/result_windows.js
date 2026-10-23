// Chromium check: a search answer opens by itself as 1-3 small windows beside the sphere; every window has fold / big screen / close / open in browser.
// An old conversation, an ordinary answer and an answer that was already closed do not open anything.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const sent = [];
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) { const body = JSON.parse(rq.postData() || '{}'); sent.push(body); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: true, items: [] }) }); }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  const count = () => p.$$eval('#fw-layer .fw', a => a.length);
  const para = n => Array.from({ length: n }, (_, i) => 'Абзац ' + (i + 1) + ': ' + 'текст про борщ и свёклу. '.repeat(14)).join('\n\n');
  // 1. the first look at a conversation opens nothing, even if it has a search answer
  await p.evaluate(p0 => fwAutoFromChat({ id: 'c1', messages: [{ role: 'user', content: 'старое', rid: 'r0' }, { role: 'assistant', content: 'старый ответ', src: 'search', rid: 'r0' }] }), 0);
  ok(await count() === 0, 'an old conversation opens no windows');
  // 2. a new short search answer: one window, near the sphere (not at the corner), with four buttons + copy
  await p.evaluate(() => fwAutoFromChat({ id: 'c1', messages: [{ role: 'user', content: 'старое', rid: 'r0' }, { role: 'assistant', content: 'старый ответ', src: 'search', rid: 'r0' }, { role: 'user', content: 'найди рецепт борща', rid: 'r1' }, { role: 'assistant', content: 'Короткий ответ про борщ.', src: 'search', rid: 'r1' }] }));
  ok(await count() === 1, 'a short search answer opens one window');
  const btns = await p.$$eval('#fw-layer .fw .fw-bar button', a => a.map(x => x.dataset.a));
  ok(['copy', 'min', 'big', 'web', 'close'].every(x => btns.includes(x)), 'the window has fold, big screen, browser and close buttons: ' + btns);
  const title = await p.textContent('#fw-layer .fw .fw-bar b');
  ok(/найди рецепт борща/.test(title), 'the title names the request: ' + title);
  // 3. fold / unfold, big screen, browser, close
  await p.click('#fw-layer .fw [data-a=min]');
  ok(await p.$eval('#fw-layer .fw', w => w.classList.contains('min')), 'fold');
  await p.click('#fw-layer .fw [data-a=min]');
  ok(await p.$eval('#fw-layer .fw', w => !w.classList.contains('min')), 'unfold');
  await p.click('#fw-layer .fw [data-a=big]');
  const big = await p.$eval('#fw-layer .fw', w => { const r = w.getBoundingClientRect(); return { cls: w.classList.contains('big'), w: r.width, h: r.height }; });
  ok(big.cls && big.w > 1100 && big.h > 600, 'big screen fills the window: ' + JSON.stringify(big));
  await p.click('#fw-layer .fw [data-a=min]');
  ok(await p.$eval('#fw-layer .fw', w => w.classList.contains('min') && !w.classList.contains('big')), 'folding leaves the big screen');
  await p.click('#fw-layer .fw [data-a=min]');
  await p.click('#fw-layer .fw [data-a=web]'); await p.waitForTimeout(200);
  const web = sent.filter(s => s.name === 'web_open');
  ok(web.length === 1 && web[0].query === 'найди рецепт борща', 'the browser button sends the request text only: ' + JSON.stringify(web));
  await p.click('#fw-layer .fw [data-a=close]');
  ok(await count() === 0, 'close');
  // 4. the same answer does not come back after it was closed (a repeated render of the conversation)
  await p.evaluate(() => fwAutoFromChat({ id: 'c1', messages: [{ role: 'user', content: 'старое', rid: 'r0' }, { role: 'assistant', content: 'старый ответ', src: 'search', rid: 'r0' }, { role: 'user', content: 'найди рецепт борща', rid: 'r1' }, { role: 'assistant', content: 'Короткий ответ про борщ.', src: 'search', rid: 'r1' }] }));
  ok(await count() === 0, 'a closed window does not reopen');
  // 5. a long search answer: 2-3 windows; an ordinary answer or a streaming one opens nothing
  const long = para(5);
  await p.evaluate(l => fwAutoFromChat({ id: 'c1', messages: [{ role: 'user', content: 'старое', rid: 'r0' }, { role: 'assistant', content: 'старый ответ', src: 'search', rid: 'r0' }, { role: 'user', content: 'найди рецепт борща', rid: 'r1' }, { role: 'assistant', content: 'Короткий ответ про борщ.', src: 'search', rid: 'r1' }, { role: 'user', content: 'ещё', rid: 'r2' }, { role: 'assistant', content: 'просто ответ', src: 'local', rid: 'r2' }, { role: 'user', content: 'расскажи о Python подробно', rid: 'r3' }, { role: 'assistant', content: l, src: 'search', rid: 'r3', streaming: true }] }), long);
  ok(await count() === 0, 'an ordinary or a still streaming answer opens nothing');
  await p.evaluate(l => fwAutoFromChat({ id: 'c1', messages: [{ role: 'user', content: 'старое', rid: 'r0' }, { role: 'assistant', content: 'старый ответ', src: 'search', rid: 'r0' }, { role: 'user', content: 'найди рецепт борща', rid: 'r1' }, { role: 'assistant', content: 'Короткий ответ про борщ.', src: 'search', rid: 'r1' }, { role: 'user', content: 'ещё', rid: 'r2' }, { role: 'assistant', content: 'просто ответ', src: 'local', rid: 'r2' }, { role: 'user', content: 'расскажи о Python подробно', rid: 'r3' }, { role: 'assistant', content: l, src: 'search', rid: 'r3' }] }), long);
  const n = await count();
  ok(n >= 2 && n <= 3, 'a long answer opens 2-3 windows: ' + n);
  const inside = await p.$$eval('#fw-layer .fw', a => a.every(w => { const r = w.getBoundingClientRect(); return r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight; }));
  ok(inside, 'every window is inside the screen');
  const texts = await p.$$eval('#fw-layer .fw .fw-body', a => a.map(x => x.textContent.length));
  ok(texts.every(x => x > 0), 'no empty window');
  // 6. another conversation: learns, opens nothing
  await p.evaluate(() => fwAutoFromChat({ id: 'c2', messages: [{ role: 'assistant', content: 'старое из другого диалога', src: 'search', rid: 'x' }] }));
  ok(await count() === n, 'switching to another conversation opens nothing');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'RESULT WINDOWS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
