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
  const patches = [], actions = []; let S = { voice_name: 'piper', voice_mode: '', reaction_voice: 'jarvis-og', piper: true, tts_enabled: true, voice_rate: 1, mic_devices: [], speech: { enabled: true, piper: true } };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/voices') return j([{ id: '', name: 'Автоматически' }, { id: 'piper', name: 'Джарвис (Piper)' }, { id: 'win:Irina', name: 'Irina (Windows)' }]);
    if (u.pathname === '/reaction-voices') return j([{ id: 'jarvis-og', name: 'Jarvis OG' }, { id: 'luna-2', name: 'Luna 2.0' }]);
    if (u.pathname === '/settings' && rq.method() === 'POST') { const pa = JSON.parse(rq.postData() || '{}'); patches.push(pa); S = Object.assign({}, S, pa); return j({ ok: true }); }
    if (u.pathname === '/settings') return j(S);
    if (u.pathname === '/action') { actions.push(JSON.parse(rq.postData() || '{}').name); return j({ ok: true }); }
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'voice'; setView(); loadSettings(); }); await p.waitForTimeout(800);
  ok(await p.$('#rv-select') === null, 'there is no second list of the recorded voices');
  ok(await p.$eval('#vt-row', e => e.hidden), 'the row of the reader of the text is hidden while a real voice is chosen');
  const opts = await p.$$eval('#voice-select option', a => a.map(o => o.value + '|' + o.textContent));
  ok(opts.some(o => o.startsWith('pack:luna-2|') && o.includes('Luna 2.0')) && opts.some(o => o.startsWith('pack:jarvis-og|')), 'the packs are in the list of voices: ' + opts.join(' ; '));
  ok(await p.$eval('#voice-select', e => e.value) === 'piper', 'at first the real voice is shown');
  await p.selectOption('#voice-select', 'pack:luna-2'); await p.waitForTimeout(1500);
  const last = patches.find(x => x.voice_mode === 'pack');
  ok(last && last.reaction_voice === 'luna-2' && last.voice_name === undefined, 'a pack is saved as the voice of the reactions and the text voice is not touched: ' + JSON.stringify(patches));
  ok(actions.includes('reaction_preview'), 'a sample of the pack is played');
  ok(await p.$eval('#voice-select', e => e.value) === 'pack:luna-2', 'the list keeps showing the pack after the settings are read again');
  const hint = await p.$eval('#voice-hint', e => e.textContent);
  ok(hint.includes('Luna 2.0') && hint.includes('пока вы его не смените'), 'the page says the choice stays until it is changed: ' + hint);
  ok(!(await p.$eval('#vt-row', e => e.hidden)) && await p.$eval('#voice-text-select', e => e.value) === 'piper', 'the row of the reader of the text is shown and keeps the real voice');
  const before = patches.length;
  await p.selectOption('#voice-text-select', 'win:Irina'); await p.waitForTimeout(1500);
  const ch = patches.slice(before).find(x => x.voice_name === 'win:Irina');
  ok(ch && ch.voice_mode === undefined && ch.reaction_voice === undefined, 'the reader of the text changes on its own, the pack stays: ' + JSON.stringify(patches.slice(before)));
  ok(await p.$eval('#voice-select', e => e.value) === 'pack:luna-2', 'the pack is still chosen after that');
  await p.selectOption('#voice-select', 'piper'); await p.waitForTimeout(1500);
  ok(patches.some(x => x.voice_name === 'piper' && x.voice_mode === ''), 'a real voice leaves the pack mode: ' + JSON.stringify(patches));
  ok(await p.$eval('#voice-select', e => e.value) === 'piper' && await p.$eval('#vt-row', e => e.hidden), 'the real voice is shown again and the row of the reader is hidden');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'VOICE PACKS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
