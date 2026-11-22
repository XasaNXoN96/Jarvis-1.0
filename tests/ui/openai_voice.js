// Chromium check: ONE list of voices has the recorded packs (Luna 2.0, Jarvis OG …) and the voices that read any text; choosing a pack saves it (and plays a sample), the row of the voice
// that reads the rest of the text appears and can be changed on its own; choosing a real voice again leaves the pack mode. There is no second list of the recorded voices.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 900 } })).newPage();
  const patches = [], actions = []; let S = { voice_name: '', voice_mode: '', piper: true, tts_enabled: true, voice_rate: 1, mic_devices: [], speech: { enabled: true, piper: true }, openai_key_saved: false, openai_key_from_ai: false, stt_cloud: false };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/voices') return j([{ id: '', name: 'Автоматически' }]);
    if (u.pathname === '/settings' && rq.method() === 'POST') { const pa = JSON.parse(rq.postData() || '{}'); patches.push(Object.assign({}, pa)); if (pa.openai_key) { S.openai_key_saved = true; delete pa.openai_key; } S = Object.assign({}, S, pa); return j({ ok: true }); }
    if (u.pathname === '/settings') return j(S);
    if (u.pathname === '/action') { const n = JSON.parse(rq.postData() || '{}').name; actions.push(n); if (n === 'openai_key_forget') { S.openai_key_saved = false; S.stt_cloud = false; } return j({ ok: true }); }
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'voice'; setView(); loadSettings(); }); await p.waitForTimeout(800);
  ok(await p.$eval('#oai-key', e => e.placeholder) === 'ключ OpenAI', 'no key: the field asks for the key');
  const hint = await p.$eval('#oai-hint', e => e.textContent);
  ok(hint.includes('Текст ответов уходит в OpenAI') && hint.includes('onyx'), 'the page says that the text goes to OpenAI and names the male voices');
  ok(!(await p.$eval('#stt-cloud', e => e.checked)), 'the cloud recognition is off by default');
  ok((await p.$eval('#stt-cloud-row small', e => e.textContent)).includes('Звук уходит в OpenAI'), 'the page says that the sound goes to OpenAI');
  await p.fill('#oai-key', 'sk-test-0123456789'); await p.click('#oai-save'); await p.waitForTimeout(900);
  ok(patches.some(x => x.openai_key === 'sk-test-0123456789'), 'the key is saved with the settings: ' + JSON.stringify(patches));
  ok((await p.$eval('#oai-key', e => e.placeholder)).includes('ключ сохранён') && await p.$eval('#oai-key', e => e.value) === '', 'the key is never shown again');
  await p.check('#stt-cloud'); await p.waitForTimeout(500);
  ok(patches.some(x => x.stt_cloud === true), 'the switch of the cloud recognition is saved');
  await p.click('#oai-forget'); await p.waitForTimeout(900);
  ok(actions.includes('openai_key_forget') && (await p.$eval('#oai-key', e => e.placeholder)) === 'ключ OpenAI', 'the key can be forgotten');
  ok(!(await p.$eval('#stt-cloud', e => e.checked)), 'forgetting the key switches the cloud recognition off');
  S.openai_key_from_ai = true; await p.evaluate(() => loadSettings()); await p.waitForTimeout(600);
  ok((await p.$eval('#oai-key', e => e.placeholder)).includes('«ИИ»'), 'with the OpenAI key of the AI the field says so');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'OPENAI VOICE: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
