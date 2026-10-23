// Chromium check: each diary / note row has a category list; changing it calls notes_category with the row id and the chosen category,
// a refused one shows the reason; an old record without a category shows «Без категории».
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
  const cats = [{ id: 'work', title: 'Работа' }, { id: 'personal', title: 'Личное' }];
  const items = [{ id: 'n1', text: 'Старая запись', kind: 'diary', at: 1700000000 }, { id: 'n2', text: 'Про работу', kind: 'diary', at: 1700000100, category: 'work' }];
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}');
      const j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
      if (body.name === 'notes_list') return j({ ok: true, items, categories: cats });
      if (body.name === 'notes_category') { sent.push(body); return j(body.category === 'bad' ? { ok: false, code: 'refused', message: 'Такой категории нет.' } : { ok: true, code: 'saved', message: 'Категория сохранена.' }); }
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => notesLoad()); await p.waitForTimeout(300);
  ok(await p.$$eval('#notes-list select.ncat', a => a.length) === 2, 'every row has a category list');
  ok(await p.$$eval('#notes-list select.ncat', a => a.map(s => s.value).join(',')) === ',work', 'an old record shows no category, the other shows its own');
  ok(await p.$$eval('#notes-list select.ncat option', a => a.length) === 6, 'each list has «Без категории» and the categories');
  await p.evaluate(() => { const s = document.querySelector('#notes-list .mem-item[data-id=n1] select.ncat'); s.value = 'personal'; s.dispatchEvent(new Event('change', { bubbles: true })); }); await p.waitForTimeout(300);
  ok(sent.length === 1 && sent[0].id === 'n1' && sent[0].category === 'personal', 'the change posts the row id and the chosen category');
  await p.evaluate(() => { const s = document.querySelector('#notes-list select.ncat'); s.innerHTML += '<option value="bad">bad</option>'; s.value = 'bad'; s.dispatchEvent(new Event('change', { bubbles: true })); });
  await p.waitForTimeout(300);
  ok(/Такой категории нет/.test(await p.textContent('#mem-msg')), 'a refused category shows the reason');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'NOTES CATEGORY: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
