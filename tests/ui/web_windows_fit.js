// Chromium check: a search with all four kinds of result (photos, video, links, text) opens FOUR windows (five with «Не нашлось»), and at every screen size,
// the small ones included, every window lies fully inside the screen and no window covers another.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const hit = (k, i) => ({ title: k + ' ' + i, host: 'example.org', note: 'заметка', thumb: '' });
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  for (const [w, h] of [[1920, 1080], [1280, 800], [1000, 640], [800, 560], [640, 480], [520, 420], [400, 700]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h } });
    await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
    const p = await ctx.newPage();
    await p.route('**/*', r => r.request().url() === 'http://jarvis.test/' ? r.fulfill({ contentType: 'text/html', body: html }) : r.fulfill({ contentType: 'application/json', body: '{}' }));
    await p.goto('http://jarvis.test/'); await p.waitForTimeout(400);
    const tag = w + 'x' + h;
    await p.evaluate(([hitS]) => {
      const mk = k => Array.from({ length: 6 }, (_, i) => ({ title: k + ' ' + i, host: 'example.org', note: 'заметка', thumb: '' }));
      applyWebResults({ id: 1, loading: false, query: 'майкл джексон' }); // the first look only learns
      applyWebResults({ id: 2, loading: false, query: 'майкл джексон', photos: mk('фото'), videos: mk('видео'), links: mk('ссылка'), text: { title: 'Статья', note: 'Текст статьи. '.repeat(30), host: 'ru.wikipedia.org' }, notes: [] });
    }, ['']);
    await p.waitForTimeout(300);
    const rects = await p.$$eval('#fw-layer .fw', a => a.map(e => { const r = e.getBoundingClientRect(); return { k: e.dataset.kind, x: r.left, y: r.top, w: r.width, h: r.height }; }));
    ok(rects.length === 4, tag + ': four windows open, got ' + rects.length);
    for (const r of rects) ok(r.x >= -0.5 && r.y >= -0.5 && r.x + r.w <= w + 0.5 && r.y + r.h <= h + 0.5, tag + ': window ' + r.k + ' is inside the screen: ' + JSON.stringify(r));
    for (let i = 0; i < rects.length; i++) for (let j = i + 1; j < rects.length; j++) {
      const a = rects[i], c = rects[j];
      const overlap = a.x < c.x + c.w - 1 && c.x < a.x + a.w - 1 && a.y < c.y + c.h - 1 && c.y < a.y + a.h - 1;
      ok(!overlap, tag + ': windows ' + a.k + ' and ' + c.k + ' do not cover each other');
    }
    await ctx.close();
  }
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'WEB WINDOWS FIT: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
