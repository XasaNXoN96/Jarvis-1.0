// Chromium check of the new tools on the settings page: the search and the state filter of the action history, the edit of a scheduled job
// (asks for a new phrase, sends the id and the phrase), and the search in a project (shows the lines, a click puts the file into the editor field).
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage(); const seen = [];
  p.on('dialog', d => d.accept(d.type() === 'prompt' ? 'каждый день в 10:00 открой блокнот' : undefined));
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url(), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}'); seen.push(body);
      if (body.name === 'undo_list') return j({ ok: true, items: [{ id: 'u1', at: 1, what: 'Запомнил: кот', undone: false, can_undo: true }] });
      if (body.name === 'sched_list') return j({ ok: true, items: [{ id: 'j1', text: 'открой почту', enabled: true, runs: 2, line: 'Каждый день в 9:00 — открой почту' }] });
      if (body.name === 'sched_edit') return j({ ok: true, message: 'Задание изменено.', undo: 'u2' });
      if (body.name === 'ca_find') return j({ ok: true, cut: false, items: [{ path: 'C:\\Проекты\\app\\main.py', line: 7, text: 'def load_config():' }] });
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  // 1. the history
  await p.evaluate(() => undoLoad()); await p.waitForTimeout(200);
  seen.length = 0;
  await p.evaluate(() => { const q = document.getElementById('undo-q'); q.value = 'кот'; q.dispatchEvent(new Event('input')); }); await p.waitForTimeout(500);
  let h = seen.filter(x => x.name === 'undo_list');
  ok(h.length === 1 && h[0].q === 'кот', 'typing searches the history: ' + JSON.stringify(h));
  seen.length = 0;
  await p.evaluate(() => { const s = document.getElementById('undo-state'); s.value = 'undone'; s.dispatchEvent(new Event('change')); }); await p.waitForTimeout(300);
  h = seen.filter(x => x.name === 'undo_list');
  ok(h.length === 1 && h[0].state === 'undone' && h[0].q === 'кот', 'the state filter is sent with the word: ' + JSON.stringify(h));
  // 2. the scheduler
  await p.evaluate(() => schLoad()); await p.waitForTimeout(200);
  ok(await p.$$eval('#sch-list button[data-op="edit"]', a => a.length) === 1, 'a job has an edit button');
  seen.length = 0;
  await p.evaluate(() => document.querySelector('#sch-list button[data-op="edit"]').click()); await p.waitForTimeout(400);
  const e = seen.find(x => x.name === 'sched_edit');
  ok(e && e.id === 'j1' && e.text === 'каждый день в 10:00 открой блокнот', 'the edit sends the id and the new phrase: ' + JSON.stringify(e));
  ok((await p.textContent('#sch-msg')).includes('изменено'), 'the answer is shown');
  // 3. the project search
  await p.evaluate(() => { document.getElementById('cf-path').value = 'C:\\Проекты\\app'; document.getElementById('cf-q').value = 'load_config'; document.getElementById('cf-go').click(); }); await p.waitForTimeout(400);
  const f = seen.find(x => x.name === 'ca_find');
  ok(f && f.path === 'C:\\Проекты\\app' && f.q === 'load_config', 'the search sends the folder and the word');
  ok((await p.textContent('#cf-hits')).includes('main.py:7') && (await p.textContent('#cf-hits')).includes('def load_config()'), 'the line is shown');
  await p.evaluate(() => document.querySelector('#cf-hits button').click());
  ok(await p.inputValue('#ca-path') === 'C:\\Проекты\\app\\main.py', 'a click puts the file into the editor field');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'DATA TOOLS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
