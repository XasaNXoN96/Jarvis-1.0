// Chromium check of the pictures card: the shape choice is saved, the gallery lists what Jarvis made, a picture is shown by its name, a delete asks first
// and sends only the file name.
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
  const p = await ctx.newPage(); const seen = []; const dialogs = [];
  p.on('dialog', d => { dialogs.push(d.type() + ':' + d.message()); d.accept(); });
  const PNG1 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
  let items = [{ name: '20260102-000001-пёс.png', bytes: 2048, modified: 1767312000 }, { name: '20260101-000001-кот.png', bytes: 150000, modified: 1767225600 }];
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url(), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}'); seen.push(body);
      if (body.name === 'img_gallery') return j({ ok: true, size: 'wide', folder: 'C:\\Users\\Я\\Pictures\\Jarvis', items });
      if (body.name === 'img_picture') return j({ ok: true, data: PNG1 });
      if (body.name === 'img_delete') { items = items.filter(x => x.name !== body.file); return j({ ok: true, message: 'Картинка удалена.' }); }
      if (body.name === 'img_size') return j({ ok: true });
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => imgGallery()); await p.waitForTimeout(300);
  ok(await p.$$eval('#img-gallery .mem-item', a => a.length) === 2, 'two pictures are listed');
  ok((await p.textContent('#img-gallery')).includes('пёс.png') && (await p.textContent('#img-gallery')).includes('Папка: C:\\Users'), 'names and the folder are shown');
  ok(await p.inputValue('#img-size') === 'wide', 'the saved shape is selected');
  seen.length = 0;
  await p.evaluate(() => { const s = document.getElementById('img-size'); s.value = 'tall'; s.dispatchEvent(new Event('change')); }); await p.waitForTimeout(300);
  const sz = seen.find(x => x.name === 'img_size');
  ok(sz && sz.value === 'tall', 'a shape change is sent: ' + JSON.stringify(sz));
  seen.length = 0;
  await p.evaluate(() => document.querySelector('#img-gallery button[data-op="view"]').click()); await p.waitForTimeout(300);
  const v = seen.find(x => x.name === 'img_picture');
  ok(v && v.file === '20260102-000001-пёс.png', 'view asks by the file name: ' + JSON.stringify(v));
  ok(await p.$eval('#img-out', e => !e.hidden && e.src.startsWith('data:image')), 'the picture is shown');
  seen.length = 0; dialogs.length = 0;
  await p.evaluate(() => document.querySelectorAll('#img-gallery button[data-op="del"]')[1].click()); await p.waitForTimeout(400);
  ok(dialogs.some(d => d.startsWith('confirm:') && d.includes('кот.png') && d.includes('нельзя')), 'a delete asks first and says it cannot be undone: ' + dialogs.join('|'));
  const dl = seen.find(x => x.name === 'img_delete');
  ok(dl && dl.file === '20260101-000001-кот.png' && !('path' in dl), 'only the file name is sent: ' + JSON.stringify(dl));
  ok(await p.$$eval('#img-gallery .mem-item', a => a.length) === 1, 'the list is refreshed');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'PICTURES UI: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
