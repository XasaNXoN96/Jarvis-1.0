// Chromium check of «Копировать ссылку привязки» in the card «Система → Пульт телефона»:
// the link comes from the program on press (same link as the QR), is copied as it is, is not left in the page, and every failure is said aloud.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const PIN = 'sha256/AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde';
const LINK_CODE = 'https://192.168.1.10:9713/#pin=' + PIN + '&c=123456';
const LINK_PIN = 'https://192.168.1.10:9713/#pin=' + PIN;
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const run = async (name, reply, clip, check) => {
    const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
    await ctx.addInitScript(clip => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} window.__copied = [];
      Object.defineProperty(navigator, 'clipboard', { value: { writeText: t => clip === 'deny' ? Promise.reject(new Error('denied')) : (window.__copied.push(t), Promise.resolve()) }, configurable: true }); }, clip);
    const p = await ctx.newPage(); const posted = [];
    await p.route('**/*', async r => {
      const rq = r.request(), u = rq.url();
      if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.endsWith('/action')) { const body = JSON.parse(rq.postData() || '{}'); if (body.name === 'pair_link') { posted.push({ url: u, body }); return r.fulfill({ contentType: 'application/json', body: JSON.stringify(reply) }); } }
      return r.fulfill({ contentType: 'application/json', body: '{}' });
    });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => document.querySelector('#rm-copypair').click()); await p.waitForTimeout(500);
    await check(p, posted, name);
    await ctx.close();
  };
  // 1. a link with a code: copied as it is, the confirmation says so, the link is not left in the page
  await run('with code', { ok: true, link: LINK_CODE, has_code: true }, 'ok', async (p, posted, n) => {
    ok((await p.evaluate(() => window.__copied)).join('|') === LINK_CODE, n + ': exactly the link of the program is copied');
    ok(/скопирована \(с кодом/.test(await p.textContent('#rm-pairmsg')), n + ': the confirmation: ' + await p.textContent('#rm-pairmsg'));
    ok(await p.$eval('#rm-pairfallback', e => e.hidden), n + ': no fallback field');
    ok(!(await p.evaluate(() => document.body.innerText)).includes('pin=sha256') && !(await p.evaluate(() => document.body.innerHTML)).includes('c=123456'), n + ': the link is not left in the page');
    ok(posted.length === 1 && !posted[0].url.includes('?') && !JSON.stringify(posted[0].body).includes('123456') && !JSON.stringify(posted[0].body).includes('pin='), n + ': the request carries no code and no link: ' + JSON.stringify(posted));
  });
  // 2. no code shown: the pin-only link is copied and the owner is told what is missing
  await run('no code', { ok: true, link: LINK_PIN, has_code: false }, 'ok', async (p, posted, n) => {
    ok((await p.evaluate(() => window.__copied)).join('|') === LINK_PIN, n + ': the pin-only link is copied');
    ok(/кода привязки сейчас нет/.test(await p.textContent('#rm-pairmsg')), n + ': the owner is told there is no code: ' + await p.textContent('#rm-pairmsg'));
  });
  // 3. the remote is off or the key is unusable: nothing is copied, the reason is shown
  await run('remote off', { ok: false, message: 'Пульт выключен: включите его, чтобы получить ссылку привязки.' }, 'ok', async (p, posted, n) => {
    ok((await p.evaluate(() => window.__copied)).length === 0, n + ': nothing is copied');
    ok((await p.textContent('#rm-pairmsg')).includes('Пульт выключен') && (await p.$eval('#rm-pairmsg', e => e.className)).includes('bad'), n + ': the reason is shown as an error');
  });
  // 4. the program is locked by the owner's password
  await run('locked', { ok: false, locked: true, message: 'Программа заблокирована.' }, 'ok', async (p, posted, n) => {
    ok((await p.evaluate(() => window.__copied)).length === 0 && (await p.textContent('#rm-pairmsg')).includes('заблокирована'), n + ': locked: nothing is copied, the reason is shown');
  });
  // 5. the clipboard refuses: an honest message and the link in a read-only selected field (only now)
  await run('clipboard refuses', { ok: true, link: LINK_CODE, has_code: true }, 'deny', async (p, posted, n) => {
    ok(!(await p.$eval('#rm-pairfallback', e => e.hidden)), n + ': the fallback field is shown');
    ok(await p.$eval('#rm-pairlink', e => e.readOnly && e.value) === LINK_CODE, n + ': it holds the same link, read-only');
    ok(/не сработало/.test(await p.textContent('#rm-pairmsg')) && (await p.$eval('#rm-pairmsg', e => e.className)).includes('bad'), n + ': the failure is said aloud (no false «скопировано»): ' + await p.textContent('#rm-pairmsg'));
  });
  // 6. no answer at all
  await run('no answer', null, 'ok', async (p, posted, n) => { ok(/Нет ответа/.test(await p.textContent('#rm-pairmsg')) && (await p.evaluate(() => window.__copied)).length === 0, n + ': no answer: nothing is copied, the failure is said'); });
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'PAIR LINK COPY: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
