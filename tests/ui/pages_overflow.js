// Chromium check: no page of the window scrolls horizontally at the sizes and Windows scales (100/125/150/200 %) that matter.
// Run: node tests/ui/pages_overflow.js  (from the repository root)
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const PAGES = ['home', 'chat', 'tasks', 'messages', 'commands', 'builder', 'apps', 'games', 'devices', 'system', 'diag', 'settings'];
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const bad = [];
  // CSS pixels of a 1920x1080 / 1366x768 / small window at 100, 125, 150 and 200 % scale
  for (const [w, h, dpr] of [[1920, 1000, 1], [1536, 800, 1.25], [1280, 680, 1.5], [960, 540, 2], [800, 600, 1], [700, 600, 1.25]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: dpr });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
    const p = await ctx.newPage();
    await p.route('**/*', r => { const u = r.request().url(); if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html }); return r.fulfill({ contentType: 'application/json', body: '{}' }); });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    for (const page of PAGES) {
      await p.evaluate(n => show(n), page); await p.waitForTimeout(200);
      const o = await p.evaluate(() => { const scrolled = e => { let a = e; while ((a = a.parentElement)) { if (['auto', 'scroll'].includes(getComputedStyle(a).overflowX) && a.id !== 'set-main') return true; } return false; }; const el = document.querySelector('.page.active'); const over = [...el.querySelectorAll('*')].filter(e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.right > innerWidth + 2 && getComputedStyle(e).position !== 'fixed' && !e.closest('.fw,.rg-dlg,#tour,details:not([open]),[hidden]') && !scrolled(e); }).slice(0, 3).map(e => e.tagName + '#' + e.id + '.' + (e.className + '').slice(0, 30)); return { sw: document.documentElement.scrollWidth, cw: innerWidth, over }; });
      if (o.sw > o.cw + 2 || o.over.length) bad.push(`${w}x${h}@${dpr} ${page} sw=${o.sw} over=${o.over.join(',')}`);
    }
    await ctx.close();
  }
  await b.close();
  console.log(bad.length ? 'OVERFLOW:\n' + bad.join('\n') : 'PAGES: no horizontal overflow at any size or scale');
  process.exit(bad.length ? 1 : 0);
})();
