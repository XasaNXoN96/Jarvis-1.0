// Chromium check of the interface language: English shows the fixed words of the window in English (menu, titles, buttons, placeholders),
// the choice is kept, Russian comes back exactly, texts that the program sets later are translated when they are fixed words and are never
// overwritten by an old translation, and nothing overflows sideways in English.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage();
  await p.route('**/*', r => { const u = r.request().url(); if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html }); return r.fulfill({ contentType: 'application/json', body: '{}' }); });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  const nav = () => p.$$eval('nav button[data-page] span:last-child', a => a.map(x => x.textContent.trim()));
  const ru = await nav();
  ok(ru.includes('Главная') && ru.includes('Настройки'), 'Russian by default: ' + ru.join(','));
  ok(await p.$('#uilang-seg [data-uilang="en"]') !== null, 'the language switch exists');
  await p.evaluate(() => setUiLang('en')); await p.waitForTimeout(200);
  const en = await nav();
  ok(en.includes('Home') && en.includes('Settings') && en.includes('Commands') && !en.includes('Главная'), 'the menu is English: ' + en.join(','));
  ok(await p.evaluate(() => localStorage.getItem('jarvis-ui-lang')) === 'en', 'the choice is kept');
  ok(await p.evaluate(() => document.documentElement.lang) === 'en', 'the page language is set');
  ok((await p.$eval('#h-input', e => e.placeholder)).startsWith('Type a message'), 'a placeholder is translated: ' + await p.$eval('#h-input', e => e.placeholder));
  ok(await p.$eval('#h-send', e => e.textContent.trim()) === 'Send', 'a button is translated');
  // a text that the program sets later: a fixed word is translated, a free text is left alone, and a new text replaces the old translation
  await p.evaluate(() => { const d = document.createElement('div'); d.id = 'dyn'; d.textContent = 'Загрузка…'; document.body.appendChild(d); });
  await p.waitForTimeout(150);
  ok(await p.$eval('#dyn', e => e.textContent) === 'Loading…', 'a later fixed text is translated');
  await p.evaluate(() => { document.getElementById('dyn').firstChild.textContent = 'Нет данных об устройствах.'; }); await p.waitForTimeout(150);
  ok(await p.$eval('#dyn', e => e.textContent) === 'No device data.', 'a new text is not overwritten by the old translation: ' + await p.$eval('#dyn', e => e.textContent));
  await p.evaluate(() => { document.getElementById('dyn').firstChild.textContent = 'Произвольный текст сервера'; }); await p.waitForTimeout(150);
  ok(await p.$eval('#dyn', e => e.textContent) === 'Произвольный текст сервера', 'a free text stays as it is');
  // no overflow sideways in English on the pages
  for (const pg of ['home', 'commands', 'apps', 'settings', 'diag', 'system']) {
    await p.evaluate(pg => { const x = document.querySelector('nav button[data-page="' + pg + '"]'); if (x) x.click(); }, pg); await p.waitForTimeout(250);
    const o = await p.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: innerWidth }));
    ok(o.sw <= o.cw + 2, pg + ': no horizontal scroll in English (' + o.sw + ' > ' + o.cw + ')');
  }
  // back to Russian: exactly the old words
  await p.evaluate(() => setUiLang('ru')); await p.waitForTimeout(200);
  ok((await nav()).join(',') === ru.join(','), 'Russian comes back exactly');
  ok((await p.$eval('#h-input', e => e.placeholder)).startsWith('Напишите сообщение'), 'the placeholder comes back');
  // a page that starts with English
  const c2 = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await c2.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); localStorage.setItem('jarvis-ui-lang', 'en'); } catch (e) {} });
  const q = await c2.newPage();
  await q.route('**/*', r => { const u = r.request().url(); if (u === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html }); return r.fulfill({ contentType: 'application/json', body: '{}' }); });
  await q.goto('http://jarvis.test/'); await q.waitForTimeout(600);
  ok((await q.$$eval('nav button[data-page] span:last-child', a => a.map(x => x.textContent.trim()))).includes('Home'), 'a saved English choice applies at start');
  ok(await q.$eval('#uilang-seg [data-uilang="en"]', e => e.classList.contains('active')), 'the switch shows the saved language');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'UI LANGUAGE: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
