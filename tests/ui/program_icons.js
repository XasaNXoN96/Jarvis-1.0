// Chromium check of the icons on the page «Программы»: a program with a real icon shows it over the letter; when the icon is not there
// (404) the letter stays and the page does not ask again; a program of the Store (no file) is never asked for an icon; the page asks by a key,
// never by a path of the page's own making for fixed tools.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const launch = () => chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
(async () => {
  const b = await launch();
  const ctx = await b.newContext({ viewport: { width: 1200, height: 900 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage(); const asked = [];
  const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');
  await p.route('**/*', r => {
    const rq = r.request(), u = new URL(rq.url());
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/prog-icon') { const k = u.searchParams.get('k'); asked.push(k); return k === 'app:C:\\Tools\\Good.exe' || k.startsWith('ess:') ? r.fulfill({ contentType: 'image/png', body: PNG }) : r.fulfill({ status: 404, body: 'no icon' }); }
    if (u.pathname === '/apps') return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ apps: [
      { name: 'Good', launch: 'C:\\Tools\\Good.exe', ok: true, category: 'user' },
      { name: 'NoIcon', launch: 'C:\\Tools\\NoIcon.exe', ok: true, category: 'user' },
      { name: 'StoreApp', launch: 'shell:appsfolder\\Some.App!App', ok: true, category: 'other' } ] }) });
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('apps'); appsList = [
    { name: 'Good', launch: 'C:\\Tools\\Good.exe', ok: true, category: 'user' },
    { name: 'NoIcon', launch: 'C:\\Tools\\NoIcon.exe', ok: true, category: 'user' },
    { name: 'StoreApp', launch: 'shell:appsfolder\\Some.App!App', ok: true, category: 'other' } ]; plLib = { shortcuts: [], hidden: [], launches: {} }; appsRender(); });
  await p.waitForTimeout(800);
  const card = n => p.$$eval('#ap-list .pc-card', (cs, n) => { const c = cs.find(x => x.querySelector('b').textContent === n); return c ? { img: !!c.querySelector('img.ap-ico'), letter: c.querySelector('.pc-mono').firstChild.textContent } : null; }, n);
  const good = await card('Good'), noicon = await card('NoIcon'), store = await card('StoreApp'), calc = await card('Калькулятор');
  ok(good && good.img && good.letter === 'G', 'a program with an icon shows it (the letter is under it)');
  ok(noicon && !noicon.img && noicon.letter === 'N', 'without an icon (404) the letter stays');
  ok(store && !store.img, 'a program of the Store gets no icon request');
  ok(calc && calc.img, 'a fixed tool of Windows asks for its icon');
  ok(!asked.some(k => /shell:/i.test(k)), 'no request for a shell: program: ' + asked.join(' | '));
  ok(asked.every(k => /^(ess|sc|app):/.test(k)), 'every request is a key, not a bare path: ' + asked.join(' | '));
  const before = asked.length;
  await p.evaluate(() => appsRender()); await p.waitForTimeout(500);
  ok(!asked.slice(before).includes('app:C:\\Tools\\NoIcon.exe'), 'a program that gave no icon is not asked again');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'PROGRAM ICONS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
