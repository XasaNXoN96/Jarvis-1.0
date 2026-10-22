// Chromium check: deleting and editing a memory entry / a note shows «Отменить», which calls the undo of the action history;
// a refused edit shows the reason and no undo button.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const seen = []; let items = [{ id: 'm1', text: 'Я живу в Ташкенте' }];
  p.on('dialog', d => d.accept(d.type() === 'prompt' ? '' : undefined));
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}'); seen.push(body.name);
      const j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
      if (body.name === 'mem_list') return j({ ok: true, items, auto: true, use_in_ai: true });
      if (body.name === 'mem_delete') { items = []; return j({ ok: true, message: 'Удалено.', undo: 'u1' }); }
      if (body.name === 'mem_edit') return j({ ok: false, message: 'Запись не может быть пустой: чтобы убрать её, нажмите «Удалить».' });
      if (body.name === 'undo_do') { items = [{ id: 'm1', text: 'Я живу в Ташкенте' }]; return j({ ok: true, message: 'Вернул запись в память.' }); }
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => memLoad()); await p.waitForTimeout(300);
  ok(await p.$$eval('#mem-list .mem-item', a => a.length) === 1, 'one entry is shown');
  await p.evaluate(() => document.querySelector('#mem-list button[data-op=edit]').click()); await p.waitForTimeout(300);
  ok(/не может быть пустой/.test(await p.textContent('#mem-msg')) && !(await p.$('#mem-undo')), 'a refused edit shows the reason and no undo button');
  await p.evaluate(() => document.querySelector('#mem-list button[data-op=del]').click()); await p.waitForTimeout(300);
  ok(await p.$$eval('#mem-list .mem-item', a => a.length) === 0, 'the entry is gone after delete');
  ok(!!(await p.$('#mem-undo')), 'the «Отменить» button appears after delete');
  await p.evaluate(() => document.querySelector('#mem-undo').click()); await p.waitForTimeout(400);
  ok(seen.includes('undo_do'), 'the button calls the undo of the action history');
  ok(await p.$$eval('#mem-list .mem-item', a => a.length) === 1, 'the entry is back after undo');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'MEMORY UNDO: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
