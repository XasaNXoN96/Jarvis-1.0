// Chromium check: the phoenix (a sphere of its own) draws, changes its mood by the state, by what is said and by the answers; the moods can be switched off; the tags [злость] never show.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const shots = process.env.PHOENIX_SHOTS;
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 900 } })).newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e)));
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} window.WebSocket = class { constructor() { setTimeout(() => this.onopen && this.onopen(), 50); } send() {} close() {} }; });
  await p.route('**/*', r => r.request().url() === 'http://jarvis.test/' ? r.fulfill({ contentType: 'text/html', body: html }) : r.fulfill({ contentType: 'application/json', body: '{}' }));
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(400);
  await p.evaluate(() => { setState('idle'); setLook('sphere', 'phoenix'); show('home'); });
  await p.waitForTimeout(900);
  const mood = () => p.$eval('#orb', e => e.dataset.mood);
  const paint = () => p.evaluate(() => { const c = document.getElementById('orb'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0, r = 0, g = 0, bl = 0; for (let i = 0; i < d.length; i += 16) if (d[i + 3] > 40) { n++; r += d[i]; g += d[i + 1]; bl += d[i + 2]; } return { n, r: r / Math.max(1, n), g: g / Math.max(1, n), b: bl / Math.max(1, n) }; });
  ok(await p.evaluate(() => sphere) === 'phoenix', 'the phoenix can be chosen as the sphere');
  ok(await mood() === 'calm', 'calm at rest: ' + await mood());
  const calm = await paint(); ok(calm.n > 3000, 'the phoenix is drawn: ' + calm.n);
  if (shots) { await p.waitForTimeout(500); await (await p.$('#orb')).screenshot({ path: path.join(shots, 'calm.png') }); }
  const say = async (fn, expect, note) => { await p.evaluate(fn); await p.waitForTimeout(1100); ok(await mood() === expect, note + ': ' + await mood()); if (shots) await (await p.$('#orb')).screenshot({ path: path.join(shots, expect + '.png') }); };
  await say(() => moodHear('ты дурак и чебурек'), 'angry', 'an insult makes it angry');
  const angry = await paint(); ok(angry.r > angry.b + 40 && angry.n > 3000, 'the angry one is drawn in fire colours');
  await say(() => { clearMood(); moodSay('Не понял, повторите', ''); }, 'confused', 'an unclear answer');
  await say(() => { clearMood(); moodHear('спасибо, молодец'); }, 'happy', 'a thanks');
  await say(() => { clearMood(); moodSay('Ха-ха, вот это шутка', ''); }, 'laugh', 'a joke');
  await say(() => { clearMood(); moodSay('Увы, к сожалению не вышло', ''); }, 'sad', 'a sorry answer');
  await say(() => { clearMood(); moodSay('Хорошо', 'surprised'); }, 'surprised', 'a tag in the answer');
  await say(() => { clearMood(); moodSay('Сам такой', 'shy'); }, 'shy', 'a tag: shy');
  // a weak reaction does not push away a strong one
  await p.evaluate(() => { setMood('angry', 5000); setMood('happy', 1800, true); }); await p.waitForTimeout(300);
  ok(await mood() === 'angry', 'a command that went well does not stop the anger');
  // the tag is cut out and never shown
  const sp = await p.evaluate(() => moodSplit('[злость] Сам такой!'));
  ok(sp[0] === 'Сам такой!' && sp[1] === 'angry', 'the tag is cut out of the text: ' + JSON.stringify(sp));
  ok((await p.evaluate(() => moodSplit('массив [0]')))[0] === 'массив [0]', 'other brackets stay');
  // the state decides when nothing was said
  await p.evaluate(() => { clearMood(); setState('listening'); }); await p.waitForTimeout(300);
  ok(await mood() === 'listening', 'listening state: ' + await mood());
  await p.evaluate(() => { clearMood(); setState('error', 5000); }); await p.waitForTimeout(300);
  ok(await mood() === 'angry', 'an error makes it angry: ' + await mood());
  await p.evaluate(() => { clearMood(); setState('off'); }); await p.waitForTimeout(300);
  ok(await mood() === 'sleep', 'no connection: it sleeps: ' + await mood());
  await p.evaluate(() => { clearMood(); setState('idle'); });
  // the reactions can be switched off
  await p.evaluate(() => { show('settings'); setMoodReact(false); });
  ok(await p.evaluate(() => !!document.querySelector('[data-mood-react="0"].active')), 'the switch shows Off');
  await p.evaluate(() => { show('home'); moodHear('ты дурак'); moodSay('Не понял', ''); }); await p.waitForTimeout(500);
  ok(await mood() === 'calm', 'with the reactions off it stays calm: ' + await mood());
  await p.evaluate(() => setMoodReact(true));
  const own = errs.filter(e => !/download_mb|reading 'map'/.test(e)); // the empty answers of this test server break other pages, not the phoenix
  ok(own.length === 0, 'no errors on the page: ' + own.join(' | '));
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'PHOENIX: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
