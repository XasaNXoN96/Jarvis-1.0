// Chromium check: the card «Сеть и обход блокировок» saves the mode and the proxy, shows the proxies found on the computer and the result of the network check.
const { chromium } = require(process.env.PW_MODULE || 'playwright');
const fs = require('fs'), path = require('path');
const src = path.join(__dirname, '../../crates/jarvis-core/src');
let html = fs.readFileSync(path.join(src, 'ui_page.html'), 'utf8').replace('/*ORBKIT*/', fs.readFileSync(path.join(src, 'orbkit.js'), 'utf8'));
html = html.replace(/<!--ADMIN-BEGIN-->|<!--ADMIN-END-->/g, '').replace(/\/\*ADMIN-BEGIN\*\/|\/\*ADMIN-END\*\//g, '');
const fails = []; const ok = (c, m) => { if (!c) fails.push(m); };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROMIUM || (fs.existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined), args: ['--no-sandbox'] });
  const p = await (await b.newContext({ viewport: { width: 1300, height: 900 } })).newPage();
  const patches = []; const actions = [];
  let S = { voice_name: '', piper: true, tts_enabled: true, voice_rate: 1, mic_devices: [], speech: { enabled: true, piper: true }, net_mode: 'auto', net_proxy_saved: false, net_proxy_masked: '' };
  await p.addInitScript(() => { try { localStorage.setItem('jv_tour', '1'); } catch (e) {} });
  await p.route('**/*', async r => {
    const rq = r.request(), u = new URL(rq.url()), j = o => r.fulfill({ contentType: 'application/json', body: JSON.stringify(o) });
    if (u.href === 'http://jarvis.test/') return r.fulfill({ contentType: 'text/html', body: html });
    if (u.pathname === '/voices') return j([{ id: '', name: 'Автоматически' }]);
    if (u.pathname === '/settings' && rq.method() === 'POST') {
      const pa = JSON.parse(rq.postData() || '{}'); patches.push(pa); S = Object.assign({}, S, pa);
      if (pa.net_proxy) { S.net_proxy_saved = true; S.net_proxy_masked = 'socks5://***@h:1080'; }
      if (pa.net_proxy === '') { S.net_proxy_saved = false; S.net_proxy_masked = ''; }
      return j({ ok: true });
    }
    if (u.pathname === '/settings') return j(S);
    if (u.pathname === '/action') {
      const body = JSON.parse(rq.postData() || '{}'); actions.push(body.action || body.name || '');
      const a = body.action || body.name;
      if (a === 'net_status') return j({ ok: true, mode: 'auto', way: 'как настроено в системе' });
      if (a === 'net_detect') return j({ ok: true, found: [{ url: 'socks5h://127.0.0.1:10808', who: 'v2rayN / Xray (SOCKS)' }] });
      if (a === 'net_check') return j({ ok: true, mode: 'auto', way: 'через прокси http://127.0.0.1:7890', rows: [{ name: 'Telegram', direct: false, proxy: true }, { name: 'Погода', direct: true, proxy: null }] });
    }
    return j({});
  });
  await p.goto('http://jarvis.test/'); await p.waitForTimeout(500);
  await p.evaluate(() => { show('settings'); setCat = 'connect'; setSub['connect'] = 'net-card'; setView(); loadSettings(); }); await p.waitForTimeout(800);
  ok(await p.$('#net-card') !== null, 'the card is on the page');
  ok(await p.$eval('#net-mode', e => e.value) === 'auto', 'auto by default');
  const hint = await p.$eval('#net-card small', e => e.textContent);
  ok(hint.includes('Авто') && hint.includes('сервер вне вашей сети') && hint.includes('Домашняя сеть'), 'the card says what it does and its honest limit');
  await p.selectOption('#net-mode', 'proxy'); await p.waitForTimeout(400);
  ok(patches.some(x => x.net_mode === 'proxy'), 'the mode is saved');
  await p.fill('#net-proxy', 'socks5://u:p@h:1080'); await p.click('#net-save'); await p.waitForTimeout(700);
  ok(patches.some(x => x.net_proxy === 'socks5://u:p@h:1080'), 'the proxy is saved');
  ok((await p.$eval('#net-saved', e => e.textContent)).includes('***'), 'the password is not shown');
  ok(await p.$eval('#net-proxy', e => e.value) === '', 'the field is cleared after saving');
  await p.click('#net-detect'); await p.waitForTimeout(500);
  ok((await p.$eval('#net-result', e => e.textContent)).includes('127.0.0.1:10808'), 'a found proxy is offered');
  await p.click('#net-result button'); await p.waitForTimeout(500);
  ok(patches.some(x => x.net_proxy === 'socks5h://127.0.0.1:10808'), 'a found proxy can be used in one click');
  await p.click('#net-check'); await p.waitForTimeout(500);
  const res = await p.$eval('#net-result', e => e.textContent);
  ok(res.includes('❌ Telegram напрямую') && res.includes('✅ через прокси') && res.includes('✅ Погода напрямую'), 'the check shows direct and proxy results: ' + res);
  await p.click('#net-clear'); await p.waitForTimeout(500);
  ok(patches.some(x => x.net_proxy === ''), 'the proxy can be removed');
  // the switch «Запоминать с помощью ИИ» (memory card): off by default, says that the phrase goes to the AI, saves
  await p.evaluate(() => { setCat = 'off'; setSub['off'] = 'mem-card'; setView(); loadSettings(); }); await p.waitForTimeout(600);
  ok(!(await p.$eval('#mem-ai', e => e.checked)), 'memory by AI is off by default');
  ok((await p.$eval('#mem-ai', e => e.closest('.setrow').textContent)).includes('уходит ИИ'), 'it says the phrase goes to the AI');
  await p.evaluate(() => { document.querySelector('#mem-ai').click(); }); await p.waitForTimeout(500);
  ok(patches.some(x => x.mem_ai === true), 'the switch is saved');
  await b.close();
  console.log(fails.length ? 'FAILED:\n' + fails.join('\n') : 'NET CARD: all checks passed');
  process.exit(fails.length ? 1 : 0);
})();
