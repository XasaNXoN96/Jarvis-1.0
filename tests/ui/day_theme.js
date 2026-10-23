// Chromium check of the light theme «День»: it can be picked (window and phone page), is kept, and the main texts have a readable contrast
// against their background on the main pages; the other themes are not changed (dark text is not used on them).
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
const orb = fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', orb);
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const remote = fs.readFileSync(path.join(src, 'remote_page.html'), 'utf8').replace('/*ORBKIT*/', orb);
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const lum = c => { const [r, g, b] = c.map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  // 1. the window
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage();
  await p.route('**/*', r => { const u = r.request().url(); if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html }); return r.fulfill({ contentType: 'application/json', body: '{}' }); });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  ok(await p.$('[data-theme-pick="day"]') !== null, 'the window offers the light theme');
  await p.evaluate(() => setLook('theme', 'day')); await p.waitForTimeout(800);
  ok(await p.evaluate(() => document.documentElement.dataset.theme) === 'day', 'the theme is applied');
  ok(await p.evaluate(() => localStorage.getItem('jarvis-theme')) === 'day', 'the choice is kept');
  const rgb = s => s.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number);
  for (const pg of ['home', 'commands', 'apps', 'settings', 'diagnostics', 'system']) {
    await p.evaluate(pg => { const x = document.querySelector('nav button[data-page="' + pg + '"]'); if (x) x.click(); }, pg); await p.waitForTimeout(300);
    const r = await p.evaluate(() => {
      const bg = getComputedStyle(document.body).backgroundColor, fg = getComputedStyle(document.body).color;
      const cs = x => getComputedStyle(x), out = { bgBody: bg, fgBody: fg };
      const h = document.querySelector('.page.active h1, .page.active h2, main h1, h1'); out.head = h ? cs(h).color : null;
      const dim = document.querySelector('.dim, small'); out.dim = dim ? cs(dim).color : null;
      const nav = document.querySelector('nav'); out.navBg = nav ? cs(nav).backgroundColor : null; out.navFg = nav ? cs(nav.querySelector('button')).color : null;
      return out;
    });
    const L = c => c && c !== 'rgba(0, 0, 0, 0)';
    ok(ratio(rgb(r.fgBody), [244, 247, 251]) >= 7, pg + ': body text contrast ' + ratio(rgb(r.fgBody), [244, 247, 251]).toFixed(1));
    if (L(r.dim)) ok(ratio(rgb(r.dim), [244, 247, 251]) >= 4.5, pg + ': secondary text contrast ' + ratio(rgb(r.dim), [244, 247, 251]).toFixed(1));
    if (L(r.navBg) && L(r.navFg)) ok(ratio(rgb(r.navFg), [255, 255, 255]) >= 4.5, pg + ': menu text contrast ' + ratio(rgb(r.navFg), [255, 255, 255]).toFixed(1));
  }
  const ov = await p.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: innerWidth }));
  ok(ov.sw <= ov.cw + 2, 'no horizontal scroll in the light theme');
  // the dark themes keep light text on a dark ground
  await p.evaluate(() => setLook('theme', 'jarvis')); await p.waitForTimeout(800);
  const dark = await p.evaluate(() => getComputedStyle(document.body).color);
  ok(rgb(dark).every(v => v > 150), 'the dark theme keeps light text: ' + dark);
  await ctx.close();
  // 2. the phone page
  const c2 = await b.newContext({ viewport: { width: 400, height: 800 } });
  await c2.addInitScript(() => { window.JarvisAndroid = { getToken: () => 'nativesecret' + 'y'.repeat(20), setToken() {}, battery: () => '{}', keepScreenOn() {}, version: () => '1' }; try { localStorage.setItem('jv_theme', 'day'); } catch (e) {} });
  const q = await c2.newPage();
  await q.route('**/*', r => { const u = new URL(r.request().url()); if (u.pathname === '/' ) return r.fulfill({ contentType: 'text/html', body: remote }); if (u.pathname === '/look') return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ theme: 'day', sphere: '', speed: '', glow: '' }) }); return r.fulfill({ contentType: 'application/json', body: '{"msgs":[],"last":0}' }); });
  await q.goto('http://pc.test/'); await q.waitForTimeout(800);
  await q.evaluate(() => { document.querySelectorAll('.tab').forEach(t => t.classList.remove('on')); document.getElementById('t-home').classList.add('on'); });
  const th = await q.evaluate(() => ({ t: document.documentElement.dataset.theme, fg: getComputedStyle(document.body).color, bg: getComputedStyle(document.body).backgroundColor, sw: document.documentElement.scrollWidth, cw: innerWidth }));
  ok(th.t === 'day', 'the phone page applies the light theme: ' + th.t);
  ok(ratio(rgb(th.fg), rgb(th.bg)) >= 7, 'phone text contrast ' + ratio(rgb(th.fg), rgb(th.bg)).toFixed(1) + ' (' + th.fg + ' on ' + th.bg + ')');
  ok(th.sw <= th.cw + 2, 'no horizontal scroll on the phone page');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'DAY THEME: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
