// Chromium check of the security journal page: search, filters, empty state, counter, keyboard, sizes and scales.
// Run: node tests/ui/audit_page.js  (from the repository root; needs playwright and a chromium)
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const now = Math.floor(Date.now() / 1000);
const DATA = [
  { at: now - 30, door: 'phone', kind: 'command', subject: 'shutdown', risk: 'high', decision: 'refused', confirmed: false, reason: 'с телефона нельзя', op: 5 },
  { at: now - 28, door: 'phone', kind: 'policy', subject: 'shutdown', risk: 'high', decision: 'asked', confirmed: false, reason: '', op: 5 },
  { at: now - 90, door: 'voice', kind: 'command', subject: 'volume_up', risk: 'low', decision: 'allowed', confirmed: false, reason: '' },
  { at: now - 7200, door: 'window', kind: 'policy', subject: 'Shutdown', risk: 'high', decision: 'asked', confirmed: false, reason: '' },
];
const fails = [];
const ok = (c, m) => { if (!c) fails.push(m); };
let calls = [];
function serve(body) {
  if (body.op) return { ok: true, log: { entries: DATA.filter(x => x.op === body.op), total: DATA.length } };
  const q = (body.q || '').toLowerCase();
  const e = DATA.filter(x => (!body.door || x.door === body.door) && (!body.risk || x.risk === body.risk) && (!body.decision || x.decision === body.decision) && (!body.within || now - x.at <= body.within) && (!q || (x.subject + x.kind + x.reason).toLowerCase().includes(q)));
  return { ok: true, log: { entries: e, total: DATA.length } };
}
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  for (const [w, h, dpr] of [[1280, 800, 1], [1280, 800, 1.5], [960, 600, 2], [640, 700, 1.25], [400, 800, 1], [320, 568, 2]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: dpr });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); localStorage.setItem('jarvis.setCat', 'more'); localStorage.setItem('jarvis.setSub', JSON.stringify({ more: 'audit-card' })); } catch (e) {} });
    const p = await ctx.newPage();
    await p.route('**/*', async r => {
      const rq = r.request(), u = rq.url();
      if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
      if (u.endsWith('/action')) { const body = JSON.parse(rq.postData() || '{}'); if (body.name === 'audit_get') { calls.push(body); return r.fulfill({ contentType: 'application/json', body: JSON.stringify(serve(body)) }); } }
      return r.fulfill({ contentType: 'application/json', body: '{}' });
    });
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
    await p.evaluate(() => show('settings')); await p.waitForTimeout(300);
    // (the journal block is taken out of the window; the page code is still checked: the block is put back for the test)
    await p.evaluate(() => { const c = document.getElementById('audit-card'); c.dataset.cat = 'main'; c.classList.remove('acc'); setView(); });
    await p.evaluate(() => auLoad()); await p.waitForTimeout(300);
    const tag = `${w}x${h}@${dpr}`;
    const n = () => p.$$eval('#au-list .hi-item', a => a.length);
    ok(await n() === 4, tag + ': 4 entries at the start');
    await p.click('#au-list .au-op'); await p.waitForTimeout(300);
    ok(await n() === 2 && !(await p.$eval('#au-opbar', e => e.hidden)), tag + ': «вся операция» shows the 2 entries of one operation and the bar');
    await p.click('#au-opoff'); await p.waitForTimeout(300);
    ok(await n() === 4 && await p.$eval('#au-opbar', e => e.hidden), tag + ': «Показать все» returns to the full list');
    await p.fill('#au-q', 'shut'); await p.waitForTimeout(600);
    ok(await n() === 3, tag + ': search «shut» finds 3');
    await p.selectOption('#au-door', 'phone'); await p.waitForTimeout(300);
    ok(await n() === 2, tag + ': combined with the door filter finds 2');
    await p.selectOption('#au-dec', 'allowed'); await p.waitForTimeout(300);
    ok(await n() === 0, tag + ': nothing found');
    const empty = await p.textContent('#au-list');
    ok(/ничего не найдено/.test(empty) && /4/.test(empty), tag + ': empty state with the counter: ' + empty.trim());
    await p.fill('#au-q', ''); await p.selectOption('#au-door', ''); await p.selectOption('#au-dec', ''); await p.selectOption('#au-within', '3600'); await p.waitForTimeout(600);
    ok(await n() === 3, tag + ': the last hour has 3');
    // horizontal overflow of the page and of the card
    const o = await p.evaluate(() => { const c = document.querySelector('#audit-card').getBoundingClientRect(); const bad = [...document.querySelectorAll('#au-filters *, #au-list *')].filter(e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.right > innerWidth + 1; }).map(e => e.tagName + '#' + e.id); return { sw: document.documentElement.scrollWidth, cw: innerWidth, bad, vis: c.width > 0 }; });
    ok(o.vis, tag + ': the card is visible');
    ok(o.sw <= o.cw + 2, tag + ': no horizontal scroll of the page (' + o.sw + '>' + o.cw + ')');
    ok(o.bad.length === 0, tag + ': filters inside the window: ' + o.bad.join(','));
    // keyboard: Tab reaches the search and every filter, the list is not a trap
    if (dpr === 1 && w >= 1000) {
      await p.focus('#au-refresh'); await p.keyboard.press('Tab');
      const seen = [];
      for (let i = 0; i < 8; i++) { seen.push(await p.evaluate(() => document.activeElement.id)); await p.keyboard.press('Tab'); }
      ok(['au-q', 'au-door', 'au-risk', 'au-dec', 'au-within'].every(id => seen.includes(id)), tag + ': Tab reaches the search and all filters: ' + seen.join(','));
      await p.focus('#au-q'); await p.keyboard.type('vol'); await p.waitForTimeout(600);
      ok(await n() === 1, tag + ': typing in the search by keyboard filters');
      await p.focus('#au-within'); await p.keyboard.press('ArrowDown'); await p.waitForTimeout(400);
      ok(calls.length > 0 && calls[calls.length - 1].within !== undefined, tag + ': a select changed by keys sends the filter');
      const ids = await p.evaluate(() => ['au-q', 'au-door', 'au-risk', 'au-dec', 'au-within'].map(i => document.getElementById(i).tabIndex));
      ok(ids.every(t => t >= 0), tag + ': nothing is removed from the tab order');
    }
    await p.screenshot({ path: path.join(process.env.SHOTS || '/tmp', `audit_${w}x${h}_${dpr}.png`) });
    await ctx.close();
  }
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'AUDIT PAGE: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
