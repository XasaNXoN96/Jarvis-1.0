// Chromium check: the recorded packs (Luna 2.0, Jarvis OG …) are in the list of voices; choosing one saves it as the voice of the reactions and commands (and plays a sample),
// the voice that reads the text is NOT changed and the page says so; choosing a real voice again leaves the pack mode.
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
  const opts = await p.$$eval('#voice-select option', a => a.map(o => o.value + '|' + o.textContent));
  ok(opts.some(o => o.startsWith('pack:luna-2|') && o.includes('Luna 2.0')) && opts.some(o => o.startsWith('pack:jarvis-og|')), 'the packs are in the list of voices: ' + opts.join(' ; '));
  ok(await p.$eval('#voice-select', e => e.value) === 'piper', 'at first the real voice is shown');
  await p.selectOption('#voice-select', 'pack:luna-2'); await p.waitForTimeout(1500);
  const last = patches.find(x => x.voice_mode === 'pack');
  ok(last && last.reaction_voice === 'luna-2' && last.voice_name === undefined, 'a pack is saved as the voice of the reactions and the text voice is not touched: ' + JSON.stringify(patches));
  ok(actions.includes('reaction_preview'), 'a sample of the pack is played');
  ok(await p.$eval('#voice-select', e => e.value) === 'pack:luna-2', 'the list keeps showing the pack after the settings are read again');
  const hint = await p.$eval('#voice-hint', e => e.textContent);
  ok(hint.includes('Luna 2.0') && hint.includes('Джарвис (Piper)') && hint.includes('Остальной текст читает'), 'the page says who reads the rest of the text: ' + hint);
  await p.selectOption('#voice-select', 'win:Irina'); await p.waitForTimeout(1500);
  ok(patches.some(x => x.voice_name === 'win:Irina' && x.voice_mode === ''), 'a real voice leaves the pack mode: ' + JSON.stringify(patches));
  ok(await p.$eval('#voice-select', e => e.value) === 'win:Irina', 'the real voice is shown again');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'VOICE PACKS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
