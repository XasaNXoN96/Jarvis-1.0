// Chromium check of two screens of the automation group: the earlier versions of a scenario (a list that appears only when there are some;
// choosing one asks, then restores through the program) and the comparison of a game session with the one before it (only what both measured).
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
const SC = { id: 's1', name: 'Утро', phrase: 'доброе утро сценарий', trigger: 'voice', enabled: true, steps: [{ kind: 'if_day', param: 'пн-пт', window: '' }, { kind: 'volume', param: '50', window: '' }],
  versions: [{ at: 1700000000, name: 'Утро', phrase: 'доброе утро сценарий', steps: [{ kind: 'volume', param: '30', window: '' }] }] };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
  await ctx.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  const p = await ctx.newPage(); const seen = [];
  p.on('dialog', d => d.accept());
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/scenarios') return j({ scenarios: [SC], kinds: [{ kind: 'volume', title: 'Громкость', hint: '' }, { kind: 'if_day', title: 'Условие: день недели', hint: 'пн-пт' }, { kind: 'if_time', title: 'Условие: время', hint: '09:00-18:00' }], templates: [], running: null });
    if (u.pathname === '/scenario') { const body = JSON.parse(rq.postData() || '{}'); seen.push(body); return j(body.op === 'restore_version' ? { ok: true, scenario: SC } : { ok: true }); }
    if (u.pathname === '/action') {
      const body = JSON.parse(rq.postData() || '{}');
      if (body.name === 'gs_summary') return j({ ok: true, stats: { games: [], sessions: [
        { exe: 'game.exe', start: 1700000000, secs: 1800, fps_avg: 75, fps_low1: 50, cpu_avg: 55, gpu_avg: null, ram_max_gb: 8, vram_max_gb: null, gpu_temp_max: null, vs_prev: { prev_start: 1699990000, mins: -30, fps_avg: 15, fps_low1: -5, cpu_avg: 0, gpu_avg: null, gpu_temp_max: null } },
        { exe: 'game.exe', start: 1699990000, secs: 3600, fps_avg: 60, fps_low1: 55, cpu_avg: 40, gpu_avg: null, ram_max_gb: 8, vram_max_gb: null, gpu_temp_max: null, vs_prev: null } ] } });
      return j({ ok: true, items: [] });
    }
    return r.fulfill({ contentType: 'application/json', body: '{}' });
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  // 1. versions
  await p.evaluate(() => { show('builder'); return builderLoad('s1'); }); await p.waitForTimeout(500);
  ok(await p.$eval('#b-versions', e => !e.hidden), 'the versions list shows when a scenario has earlier versions');
  ok(await p.$$eval('#b-versions option', o => o.length) === 2, 'the list has the hint and one version');
  ok((await p.$$eval('#b-versions option', o => o[1].textContent)).includes('Утро') && (await p.$$eval('#b-versions option', o => o[1].textContent)).includes('1 шаг'), 'the version is described');
  seen.length = 0;
  await p.selectOption('#b-versions', '0'); await p.waitForTimeout(400);
  const rv = seen.find(x => x.op === 'restore_version');
  ok(rv && rv.id === 's1' && rv.index === 0, 'choosing a version restores it through the program: ' + JSON.stringify(seen));
  ok(await p.evaluate(() => { const s = { ...bCur, versions: [] }; bCur = s; renderBuilder(); return document.getElementById('b-versions').hidden; }), 'the list is hidden when there is nothing to put back');
  ok(await p.evaluate(() => bData.kinds.some(k => k.kind === 'if_day') && !!document.querySelector('#b-kinds button[title="пн-пт"]')), 'the condition steps are offered in the editor');
  // 2. the comparison of game sessions
  await p.evaluate(() => gsLoad()); await p.waitForTimeout(400);
  const t = await p.textContent('#gs-list');
  ok(t.includes('К прошлой сессии этой игры') && t.includes('FPS +15') && t.includes('1% low −5') && t.includes('ЦП ±0') && t.includes('время −30 мин'), 'the comparison is shown: ' + t.slice(0, 300));
  ok(!/ВК [+−±]/.test(t) && !/темп\. [+−±]/.test(t), 'a number that one side did not measure is not compared');
  ok((t.match(/К прошлой сессии/g) || []).length === 1, 'the first session of a game has no comparison');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'AUTOMATION UI: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
