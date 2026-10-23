// Chromium check of the security page: every permission shows what it does and its risk (and «off by default»), a switch sends its id and value,
// and the owner state tells how long the program stays unlocked.
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
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url(), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) {
      const body = JSON.parse(rq.postData() || '{}'); seen.push(body);
      if (body.name === 'perms_list') return j({ ok: true, items: [
        { id: 'memory', title: 'Запоминать и вести дневник', on: true, about: 'Записи памяти и дневника.', risk: 'low', default: true },
        { id: 'remote_camera', title: 'Показывать веб-камеру', on: false, about: 'Телефон видит вашу веб-камеру.', risk: 'high', default: false },
        { id: 'messenger', title: 'Писать сообщения контактам', on: true, about: 'Открывает чат.', risk: 'medium', default: true } ] });
      if (body.name === 'owner_status') return j({ ok: true, set: true, unlocked: true, unlock_left: 600, message: 'Ты не мой создатель.' });
      if (body.name === 'perms_set') return j({ ok: true });
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { permLoad(); ownerLoad(); }); await p.waitForTimeout(400);
  const t = await p.textContent('#perm-list');
  ok(t.includes('Телефон видит вашу веб-камеру') && t.includes('риск: высокий') && t.includes('По умолчанию выключено'), 'the high-risk off-by-default permission is described: ' + t.slice(0, 300));
  ok(t.includes('риск: средний') && (t.match(/риск: высокий/g) || []).length === 1, 'the risk is shown only where it exists');
  ok(!/Записи памяти и дневника\.[^]*?По умолчанию выключено[^]*?Открывает чат/.test(t.replace('Телефон видит вашу веб-камеру. По умолчанию выключено.', '')), 'a permission that is on by default does not say «off by default»');
  seen.length = 0;
  await p.evaluate(() => { const c = document.querySelector('input[data-perm="remote_camera"]'); c.checked = true; c.dispatchEvent(new Event('change', { bubbles: true })); }); await p.waitForTimeout(300);
  const s = seen.find(x => x.name === 'perms_set');
  ok(s && s.id === 'remote_camera' && s.value === true, 'a switch sends its id and value: ' + JSON.stringify(s));
  ok((await p.textContent('#ow-state')).includes('ещё 10 мин'), 'the time left is shown: ' + await p.textContent('#ow-state'));
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'SECURITY UI: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
