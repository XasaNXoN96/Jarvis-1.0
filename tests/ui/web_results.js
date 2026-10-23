// Chromium check: «найди …» shows what was found sorted by KIND, one small window for each: 📷 photos, ▶ video, 🔗 links, 📝 text. Clicking a picture, a video or a link
// opens THAT one in the browser (the page names it by kind and number: it never sends an address). Only what was found gets a window. An old search never reopens.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1280, height: 800 } })).newPage();
  const sent = [];
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = rq.url();
    if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.endsWith('/action')) { const body = JSON.parse(rq.postData() || '{}'); sent.push(body); return r.fulfill({ contentType: 'application/json', body: JSON.stringify({ ok: true, items: [] }) }); }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  const count = () => p.$$eval('#fw-layer .fw', a => a.length);
  const R = (o) => Object.assign({ id: 4, query: 'Zoom Market', loading: false, photos: [], videos: [], links: [], text: null }, o);
  const photos = [1, 2, 3, 4, 5, 6].map(i => ({ title: 'Фото ' + i, host: 'commons.wikimedia.org', thumb: 'https://upload.wikimedia.org/t' + i + '.jpg', note: '' }));
  const videos = [{ title: 'Обзор Zoom Market', host: 'youtube.com', thumb: 'https://i.ytimg.com/vi/abcDEF12345/mqdefault.jpg', note: 'Канал Один' }, { title: 'Второе видео', host: 'youtube.com', thumb: 'https://i.ytimg.com/vi/zzzzzzzzzzz/mqdefault.jpg', note: 'Канал Два' }];
  const links = [{ title: 'Zoom Market — магазин', host: 'zoommarket.uz', thumb: '', note: 'Электроника и товары для дома.' }, { title: 'Отзывы о Zoom Market', host: 'otzovik.com', thumb: '', note: 'Что пишут покупатели.' }, { title: 'Zoom Market — Википедия', host: 'ru.wikipedia.org', thumb: '', note: 'Статья.' }];
  const text = { title: 'Zoom Market', host: 'ru.wikipedia.org', thumb: '', note: 'Zoom Market — онлайн-магазин электроники и товаров для дома, работающий с 2019 года.' };
  const kindsOf = () => p.$$eval('#fw-layer .fw', a => a.map(w => w.dataset.kind));
  // 1. the first look learns the last search and opens nothing; a running search opens nothing
  await p.evaluate(r => applyWebResults(r), R({ id: 3, photos, videos, links, text }));
  ok(await count() === 0, 'an old search opens nothing');
  await p.evaluate(r => applyWebResults(r), R({ loading: true }));
  ok(await count() === 0, 'a running search opens nothing');
  // 2. ready: four windows, one for each kind, in the order photo, video, links, text
  await p.evaluate(r => applyWebResults(r), R({ photos, videos, links, text }));
  ok(await count() === 4, 'four kinds: four windows: ' + await count());
  const kinds = await kindsOf();
  ok(JSON.stringify(kinds) === JSON.stringify(['photos', 'videos', 'links', 'text']), 'the order is photo, video, links, text: ' + kinds);
  const titles = await p.$$eval('#fw-layer .fw .fw-bar b', a => a.map(x => x.textContent));
  ok(/Фото/.test(titles[0]) && /Видео/.test(titles[1]) && /Ссылки/.test(titles[2]) && /Текст/.test(titles[3]), 'the titles name the kinds: ' + titles);
  ok(await p.$$eval('[data-kind=photos] .fw-body img', a => a.length) === 6, 'six photos');
  ok(await p.$$eval('[data-kind=videos] .fw-body img', a => a.length) === 2 && /Обзор Zoom Market/.test(await p.textContent('[data-kind=videos] .fw-body')), 'videos with a picture and a title');
  ok(await p.$$eval('[data-kind=links] .fw-body .wr-card', a => a.length) === 3 && await p.$$eval('[data-kind=links] .fw-body img', a => a.length) === 0, 'links without pictures');
  ok(/онлайн-магазин/.test(await p.textContent('[data-kind=text] .fw-body')), 'the text window has the text');
  const imgOk = await p.$$eval('#fw-layer img', a => a.every(i => i.referrerPolicy === 'no-referrer' && /^https:/.test(i.getAttribute('src'))));
  ok(imgOk, 'pictures are https and sent without a referrer');
  const boxes = await p.$$eval('#fw-layer .fw', a => a.map(w => { const r = w.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom]; }));
  ok(boxes.every(r => r[0] >= 0 && r[1] >= 0 && r[2] <= 1280 && r[3] <= 800), 'every window is inside the screen: ' + JSON.stringify(boxes));
  const overlap = (a, c) => a[0] < c[2] - 2 && c[0] < a[2] - 2 && a[1] < c[3] - 2 && c[1] < a[3] - 2;
  let any = false; for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) if (overlap(boxes[i], boxes[j])) any = true;
  ok(!any, 'the tiles do not lie on top of each other: ' + JSON.stringify(boxes));
  // 3. a click opens THAT item by kind and number, with no address
  await p.click('[data-kind=photos] .wr-card:nth-child(3)'); await p.waitForTimeout(150);
  await p.click('[data-kind=videos] .wr-card:nth-child(2)'); await p.waitForTimeout(150);
  await p.click('[data-kind=links] .wr-card:nth-child(1)'); await p.waitForTimeout(150);
  await p.click('[data-kind=text] .fw-body a'); await p.waitForTimeout(150);
  const hit = sent.filter(s => s.name === 'web_open_hit').map(s => s.hit_kind + ':' + s.hit_n + ':' + s.hit_id);
  ok(JSON.stringify(hit) === JSON.stringify(['photos:2:4', 'videos:1:4', 'links:0:4', 'text:0:4']), 'each click names kind and number: ' + hit);
  ok(!JSON.stringify(sent).includes('http'), 'the page sends no address');
  // 4. the 🌐 of a window opens the search page of the request in the browser
  await p.click('[data-kind=links] [data-a=web]'); await p.waitForTimeout(150);
  ok(sent.some(s => s.name === 'web_open' && s.query === 'Zoom Market'), 'the browser button opens the search of the request');
  // 5. fold / big screen / close
  await p.click('[data-kind=photos] [data-a=min]');
  ok(await p.$eval('[data-kind=photos]', w => w.classList.contains('min')), 'fold');
  await p.click('[data-kind=photos] [data-a=min]');
  await p.click('[data-kind=photos] [data-a=big]');
  ok(await p.$eval('[data-kind=photos]', w => w.getBoundingClientRect().width > 1100), 'big screen');
  await p.click('[data-kind=photos] [data-a=big]');
  await p.click('[data-kind=photos] [data-a=close]');
  ok(await count() === 3, 'close one of four');
  // 6. the same search does not come back
  await p.evaluate(r => applyWebResults(r), R({ photos, videos, links, text }));
  ok(await count() === 3, 'a seen search does not reopen');
  // 7. only what was found gets a window
  const closeAll = () => p.evaluate(() => { document.querySelectorAll('#fw-layer .fw [data-a=close]').forEach(x => x.click()); });
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 5, query: 'слон', links, text }));
  ok(JSON.stringify(await kindsOf()) === JSON.stringify(['links', 'text']), 'links and text only: ' + await kindsOf());
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 6, query: 'x', photos: photos.slice(0, 1) }));
  ok(JSON.stringify(await kindsOf()) === JSON.stringify(['photos']), 'one photo: one window');
  // 8. nothing found / an error is said in a window with the browser button, never an empty screen
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 7, query: 'qwertyzzz' }));
  ok(await count() === 1 && /Ничего не нашёл/.test(await p.$eval('#fw-layer .fw .fw-body', x => x.textContent)), 'nothing found is said');
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 8, query: 'x', error: 'нет соединения с поиском' }));
  ok(/нет соединения/.test(await p.$eval('#fw-layer .fw .fw-body', x => x.textContent)), 'an error is shown in a window');
  // 9. a restarted Jarvis counts from zero again: the page follows it and opens the next search
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 1, query: 'после перезапуска' }));
  ok(await count() === 0, 'a restart only learns the number');
  await p.evaluate(r => applyWebResults(r), R({ id: 2, query: 'после перезапуска', links }));
  ok(await count() === 1, 'the search after a restart opens');
  // 10. a hostile title or text is shown as text, not as markup
  await closeAll();
  await p.evaluate(r => applyWebResults(r), R({ id: 9, links: [{ title: '<img src=x onerror=window.__x=1>', host: 'a.ru', thumb: '', note: '<b>жирный</b>' }], text: { title: 'T', host: 'a.ru', thumb: '', note: '<script>window.__x=1</script> просто длинный текст про что-то интересное' } }));
  ok(await p.evaluate(() => !window.__x) && await p.$$eval('#fw-layer .fw-body b', a => a.length) === 1, 'markup of the Internet stays text');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'WEB RESULTS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
