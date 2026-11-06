// Chromium check: the card «Фразы пакета» lists the clips of a recorded pack with what they are played on and which commands use them; ▶ plays one (by its name only);
// the search finds a clip by its name or by the command; another pack loads its own list.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const PACKS = { 'luna-2': [{ name: 'ok1', reaction: 'общее «выполнено»', commands: [] }, { name: 'reply1', reaction: 'слово-активатор («Слушаю»)', commands: [] }, { name: 'cmd_volume_up', reaction: '', commands: ['assistant_volume_up'] }, { name: 'cmd_open_telegram', reaction: '', commands: ['«открой телеграм»', '«открой telegram»'] }, { name: 'timer_done', reaction: '', commands: [] }], 'jarvis-og': [{ name: 'run', reaction: 'запуск программы', commands: [] }] };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 1000 } })).newPage();
  const actions = [];
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/reaction-voices') return j([{ id: 'jarvis-og', name: 'Jarvis OG' }, { id: 'luna-2', name: 'Luna 2.0' }]);
    if (u.pathname === '/settings') return j({ reaction_voice: 'luna-2', voice_name: 'piper', mic_devices: [] });
    if (u.pathname === '/commands') return j({ commands: [{ id: 'assistant_volume_up', phrases: ['громче', 'сделай погромче'] }] });
    if (u.pathname === '/action') { const bd = JSON.parse(rq.postData() || '{}'); actions.push(bd); if (bd.name === 'voice_clips') return j({ ok: true, clips: PACKS[bd.voice] || [] }); return j({ ok: true }); }
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'voice'; setView(); }); await p.waitForTimeout(900);
  const rows = async () => p.$$eval('#cl-list .cl-row', a => a.map(r => r.querySelector('b').textContent + '|' + r.querySelector('small').textContent));
  let r = await rows();
  ok(r.length === 5, 'five clips of Luna 2.0 are listed: ' + r.length);
  ok(r.some(x => x.startsWith('ok1|') && x.includes('общее «выполнено»')), 'a general clip says what it is played on');
  ok(r.some(x => x.startsWith('cmd_volume_up|') && x.includes('«громче»')), 'a clip of a command names the phrase of the command (not its id): ' + r.find(x => x.startsWith('cmd_volume_up')));
  ok(r.some(x => x.startsWith('cmd_open_telegram|') && x.includes('«открой телеграм»')), 'a clip of «открой …» names what is said');
  ok(r.some(x => x.startsWith('timer_done|') && x.includes('Пока ни на что не привязана')), 'a clip that nothing uses says so');
  await p.fill('#cl-q', 'громче'); await p.waitForTimeout(200);
  ok((await rows()).length === 1, 'the search finds a clip by the command that uses it');
  await p.fill('#cl-q', '');
  await p.click('#cl-list .cl-row:nth-child(3) button'); await p.waitForTimeout(300);
  const play = actions.filter(a => a.name === 'clip_play');
  ok(play.length === 1 && play[0].voice === 'luna-2' && play[0].clip === 'cmd_volume_up', 'a click plays a clip by its name: ' + JSON.stringify(play));
  ok(!JSON.stringify(play).includes('/') && !JSON.stringify(play).includes('.mp3'), 'no path or file name goes from the page');
  await p.selectOption('#cl-pack', 'jarvis-og'); await p.waitForTimeout(500);
  r = await rows();
  ok(r.length === 1 && r[0].startsWith('run|'), 'another pack loads its own list: ' + r.join(';'));
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'VOICE CLIPS: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
