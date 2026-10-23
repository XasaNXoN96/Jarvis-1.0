// Chromium check of the fix button on the diagnostics page: the preview is shown first, a refusal changes nothing, only «ОК» sends the confirmation.
// Run: node tests/ui/diag_fix.js  (from the repository root; needs playwright and a chromium)
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = [];
const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  for (const accept of [false, true]) {
    const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
    const p = await ctx.newPage();
    const sent = [];
    const diag = { running: false, at: Math.floor(Date.now() / 1000), checks: [
      { key: 'mic', name: 'Микрофон', status: 'warn', detail: 'Микрофон выключен пользователем.', fix: 'Включите микрофон.', fix_id: 'mic_unmute' },
      { key: 'net', name: 'Интернет', status: 'error', detail: 'Нет сети.', fix: 'Проверьте кабель.', fix_id: '' },
    ], events: [{ time: Math.floor(Date.now() / 1000), kind: 'error', text: 'Шаг 2 остановлен', op: 42 }] };
    const dialogs = [];
    p.on('dialog', async d => { dialogs.push(d.message()); accept ? await d.accept() : await d.dismiss(); });
    await p.route('**/*', async r => {
      const rq = r.request(), u = rq.url();
      if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.endsWith('/diag')) return r.fulfill({ contentType: 'application/json', body: JSON.stringify(diag) });
      if (u.endsWith('/action')) {
        const body = JSON.parse(rq.postData() || '{}'); sent.push(body);
        if (body.name === 'diag_fix_preview') return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: true, preview: 'Включить микрофон в Jarvis.' }) });
        if (body.name === 'diag_fix_apply') return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: true, applied: true, message: 'Готово.' }) });
      }
      return r.fulfill({ contentType: 'application/json', body: '{}' });
    });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => show('diag')); await p.waitForTimeout(500);
    await p.evaluate(() => diagLoad()); await p.waitForTimeout(400);
    await p.evaluate(() => document.querySelectorAll('#diag-list details').forEach(d => d.open = true));
    const btns = await p.$$('#diag-list button');
    ok(btns.length === 1, `accept=${accept}: only the check with a fix has the button (${btns.length})`);
    ok(/операция №42/.test(await p.textContent('#diag-events')), `accept=${accept}: the event shows its operation number`);
    await btns[0].click(); await p.waitForTimeout(500);
    const names = sent.map(s => s.name).filter(n => n.startsWith('diag_fix'));
    ok(dialogs.length === 1 && /Включить микрофон/.test(dialogs[0]), `accept=${accept}: the preview is shown before anything`);
    if (accept) ok(names.join() === 'diag_fix_preview,diag_fix_apply' && sent.find(s => s.name === 'diag_fix_apply').confirm === true, `accept: preview then apply with confirm:true (${names})`);
    else ok(names.join() === 'diag_fix_preview', `refused: nothing is applied (${names})`);
    await ctx.close();
  }
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'DIAG FIX: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
