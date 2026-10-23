// Chromium check of the «Связь» tools of the settings: rename and «revoke all» of the paired devices, the import of a .vcf file into the contacts,
// the search and the state filter of the message history.
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
  const answers = []; p.on('dialog', d => { answers.push(d.type() + ':' + d.message().slice(0, 40)); d.accept(d.type() === 'prompt' ? 'Телефон жены' : undefined); });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url(), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}'); seen.push(body);
      if (body.name === 'pair_list') return j({ ok: true, items: [{ id: 'd1', name: 'Pixel', created: 1700000000, last_seen: 0, revoked: false }, { id: 'd2', name: 'Старый', created: 1690000000, last_seen: 1, revoked: true }], code: null, left: 0 });
      if (body.name === 'ct_import') return j({ ok: true, message: 'Добавлено: 2. Уже были: 1. Не поняты (нет имени или номера): 0.' });
      if (body.name === 'ct_list') return j({ ok: true, contacts: [] });
      if (body.name === 'pair_rename') return j({ ok: true, message: 'Переименовано.' });
      if (body.name === 'pair_revoke_all') return j({ ok: true, message: 'Отозвано устройств: 1.' });
      return j({ ok: true, items: [], contacts: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  // 1. paired devices
  await p.evaluate(() => pairLoad()); await p.waitForTimeout(300);
  ok(await p.$$eval('#pair-list button[data-op="rename"]', a => a.length) === 1 && await p.$$eval('#pair-list button[data-op="revoke"]', a => a.length) === 1, 'only the valid device has rename and revoke');
  seen.length = 0;
  await p.evaluate(() => document.querySelector('#pair-list button[data-op="rename"]').click()); await p.waitForTimeout(300);
  const rn = seen.find(x => x.name === 'pair_rename');
  ok(rn && rn.id === 'd1' && rn.new_name === 'Телефон жены', 'rename sends the id and the new name: ' + JSON.stringify(rn));
  ok(!seen.some(x => x.name === 'pair_revoke'), 'a rename does not revoke');
  seen.length = 0;
  await p.evaluate(() => document.getElementById('pair-revoke-all').click()); await p.waitForTimeout(300);
  ok(seen.some(x => x.name === 'pair_revoke_all') && answers.some(a => a.startsWith('confirm:Отозвать ВСЕ')), 'revoke-all asks first, then sends');
  ok((await p.textContent('#pair-msg')).includes('Отозвано устройств'), 'the answer is shown');
  // 2. the import of a .vcf
  seen.length = 0;
  await p.setInputFiles('#ct-file', { name: 'contacts.vcf', mimeType: 'text/vcard', buffer: Buffer.from('BEGIN:VCARD\nFN:Малика\nTEL:998901234567\nEND:VCARD\n') });
  await p.waitForTimeout(400);
  const imp = seen.find(x => x.name === 'ct_import');
  ok(imp && imp.text.includes('FN:Малика'), 'the text of the file is sent to the program: ' + JSON.stringify(imp));
  ok((await p.textContent('#ct-msg')).includes('Добавлено: 2'), 'the result is shown');
  // 3. the history of the messages
  seen.length = 0;
  await p.evaluate(() => { const q = document.getElementById('out-q'); q.value = 'хлеб'; q.dispatchEvent(new Event('input')); }); await p.waitForTimeout(500);
  let o = seen.filter(x => x.name === 'out_list');
  ok(o.length === 1 && o[0].q === 'хлеб', 'typing searches the history: ' + JSON.stringify(o));
  seen.length = 0;
  await p.evaluate(() => { const s = document.getElementById('out-state'); s.value = 'failed'; s.dispatchEvent(new Event('change')); }); await p.waitForTimeout(300);
  o = seen.filter(x => x.name === 'out_list');
  ok(o.length === 1 && o[0].state === 'failed' && o[0].q === 'хлеб', 'the state is sent with the word: ' + JSON.stringify(o));
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'CONNECT UI: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
