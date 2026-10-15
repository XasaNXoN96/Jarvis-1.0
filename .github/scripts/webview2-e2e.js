// The interface checked inside the REAL WebView2 window of the installed Jarvis (Edge engine), not in a test browser:
// connected through the debugging port that the CI opens for this one run (JARVIS_WEBVIEW_DEBUG_PORT).
// Usage: node webview2-e2e.js <port> <report-file>
const { chromium } = require('playwright-core');

(async () => {
  const [port, reportFile] = [process.argv[2] || '9333', process.argv[3] || 'webview2-e2e.txt'];
  const lines = [];
  let failed = 0;
  const say = s => { lines.push(s); console.log(s); };
  const check = (name, ok, info = '') => { if (!ok) failed++; say(`${ok ? 'PASS' : 'FAIL'} ${name}${info ? ' — ' + info : ''}`); };
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const ctx = browser.contexts()[0];
  const page = ctx.pages().find(p => p.url().includes('127.0.0.1:9714')) || ctx.pages()[0];
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('dialog', d => d.accept());
  const wait = ms => page.waitForTimeout(ms);
  say('engine: ' + await page.evaluate(() => navigator.userAgent));
  const vp = await page.evaluate(() => [innerWidth, innerHeight]);
  say('window: ' + vp.join('x'));

  // every page opens and is shown
  for (const name of ['home', 'system', 'devices', 'chat', 'tasks', 'messages', 'commands', 'builder', 'apps', 'files', 'diag', 'settings']) {
    await page.evaluate(n => show(n), name);
    await wait(350);
    const shown = await page.evaluate(n => { const el = document.getElementById('page-' + n); return !!el && el.offsetParent !== null; }, name);
    check('page ' + name + ' is shown', shown);
  }

  // the builder: drag and drop of the steps with the mouse
  await page.evaluate(() => show('builder'));
  await wait(500);
  await page.click('#b-new');
  await page.fill('#b-name', 'ci-dnd');
  await page.fill('#b-phrase', 'проверка мыши');
  for (const k of ['Ссылка', 'Громкость', 'Пауза']) { await page.click(`#b-kinds button:has-text("${k}")`); await wait(120); }
  const order = async () => JSON.stringify(await page.$$eval('#b-steps .b-step b', e => e.map(x => x.textContent)));
  const box = async i => (await (await page.$$('#b-steps .b-step'))[i]).boundingBox();
  const drag = async (f, x, y) => {
    await page.mouse.move(f.x + f.width / 2, f.y + f.height / 2);
    await page.mouse.down();
    await page.mouse.move(f.x + f.width / 2 + 20, f.y + 20, { steps: 4 });
    await page.mouse.move(x, y, { steps: 10 });
  };
  check('builder: three steps added by click', (await order()) === '["Ссылка","Громкость","Пауза"]', await order());
  let f = await box(0), l = await box(2);
  await drag(f, l.x + l.width - 6, l.y + l.height / 2);
  const marker = await page.$$eval('#b-steps .drop-after, #b-steps .drop-before', e => e.length);
  await page.mouse.up();
  await wait(250);
  check('builder: a step dragged to the end with the mouse', (await order()) === '["Громкость","Пауза","Ссылка"]', `${await order()} (drop marker shown: ${marker})`);
  f = await box(0); l = await box(2);
  await drag(f, l.x + l.width - 6, l.y + l.height / 2);
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await wait(250);
  check('builder: Esc cancels the drag', (await order()) === '["Громкость","Пауза","Ссылка"]', await order());
  // (clicking a kind scrolls the page down to its editor: the row of kinds and the chain must both be on the screen)
  await page.evaluate(() => { const el = document.querySelector('#page-builder'); if (el) el.scrollTop = 0; });
  await wait(200);
  const kb = await (await page.$('#b-kinds button:has-text("Текст")')).boundingBox();
  f = await box(0);
  await page.mouse.move(kb.x + kb.width / 2, kb.y + kb.height / 2);
  await page.mouse.down();
  await page.mouse.move(kb.x + 30, kb.y + 40, { steps: 4 });
  await page.mouse.move(f.x + 4, f.y + f.height / 2, { steps: 10 });
  await page.mouse.up();
  await wait(250);
  check('builder: a kind dragged into the chain', (await order()) === '["Текст","Громкость","Пауза","Ссылка"]', await order());
  f = await box(2);
  await page.mouse.click(f.x + f.width / 2, f.y + f.height / 2);
  await wait(200);
  await page.click('#e-up');
  await wait(200);
  check('builder: a click selects and the arrow still moves', (await order()) === '["Текст","Пауза","Громкость","Ссылка"]', await order());
  await page.evaluate(() => { const d = document.getElementById('b-delete'); if (d) d.click(); });
  await wait(300);

  // the dialogs: «delete» puts the dialog into the archive (Settings)
  await page.evaluate(async () => {
    await fetch('/chat', { method: 'POST', headers: { 'X-Jarvis': '1' }, body: JSON.stringify({ op: 'ask', text: 'сколько времени', rid: 'ci-webview-1' }) });
  });
  await wait(4000);
  await page.evaluate(() => show('chat'));
  await wait(700);
  const before = await page.$$eval('#conv-list .conv', e => e.length);
  check('dialogs: the typed question made a dialog', before >= 1, 'dialogs: ' + before);
  if (before >= 1) {
    await page.hover('#conv-list .conv');
    await page.click('#conv-list .conv .x');
    await wait(900);
    const after = await page.$$eval('#conv-list .conv', e => e.length);
    check('dialogs: delete removes it from the list', after === before - 1, `${before} -> ${after}`);
    await page.evaluate(() => show('settings'));
    await wait(900);
    // the archive is in the category «Безопасность и данные» of the settings
    await page.click('#set-cats button[data-cat=privacy]');
    await wait(300);
    const cnt = await page.textContent('#ar-count');
    check('archive: the deleted dialog is in the archive card', /В архиве диалогов: [1-9]/.test(cnt), cnt);
    await page.click('#ar-list .conv');
    await wait(400);
    const txt = await page.textContent('#ar-text');
    check('archive: the text of the dialog is shown', txt.includes('сколько времени'), JSON.stringify(txt.slice(0, 60)));
    await page.click('#ar-restore');
    await wait(900);
    check('archive: restore empties the archive again', (await page.textContent('#ar-count')).includes('Архив пуст'), await page.textContent('#ar-count'));
  }

  // Esc in the full screen mode leaves it (one action), the window stays open
  const closedBefore = page.isClosed();
  await page.evaluate(() => show('home'));
  await wait(300);
  await page.keyboard.press('Escape');
  await wait(300);
  // the state signs of the home page (voice, computer, AI, phone) at the right edge: they do not cover the orb, the input or the
  // clock; a click opens a panel at the right, the orb does not move; «all on the whole screen»; Esc closes; the full «Система» page
  // keeps its cards and controls
  await page.evaluate(() => show('home'));
  await wait(600);
  const sum = async () => page.evaluate(() => {
    const r = sel => { const e = document.querySelector(sel); if (!e) return null; const b = e.getBoundingClientRect(); return { t: b.top, b: b.bottom, l: b.left, r: b.right, w: b.width, h: b.height }; };
    const hit = (a, c) => a && c && a.w > 0 && c.w > 0 && !(a.b <= c.t || c.b <= a.t || a.r <= c.l || c.r <= a.l);
    const rail = r('#rail'), orb = r('#orb'), form = r('#h-form'), clock = r('.clockbox');
    const items = [...document.querySelectorAll('#rail .rb')];
    return { keys: items.map(b => b.dataset.k), titles: items.map(b => b.title), texts: items.map(b => b.querySelector('em').textContent.trim()),
      overOrb: hit(rail, orb), overForm: hit(rail, form), overClock: hit(rail, clock), inside: !!rail && rail.r <= innerWidth + 1 && rail.l >= 0, orb, orbH: orb ? orb.h : 0,
      hscroll: document.documentElement.scrollWidth > innerWidth + 1, mic: !!document.querySelector('#h-mic') && !!document.querySelector('#h-send') && !!document.querySelector('#h-input') };
  });
  const s1 = await sum();
  check('state signs: voice, computer, AI and phone, each with a text', JSON.stringify(s1.keys) === JSON.stringify(['voice', 'pc', 'ai', 'ph']) && s1.titles.every(t => t.length > 5) && s1.texts.every(t => t.length > 1 && t !== '…'), s1.titles.join(' | '));
  check('state signs: do not cover the orb, the input or the clock, no horizontal scroll', !s1.overOrb && !s1.overForm && !s1.overClock && s1.inside && s1.mic && !s1.hscroll && s1.orbH > 100, 'orb height ' + Math.round(s1.orbH));
  await page.click('#hs-ai'); await wait(500);
  const p1 = await page.evaluate(() => { const d = document.getElementById('drw'), b = d.getBoundingClientRect(); const o = document.getElementById('orb').getBoundingClientRect(); return { open: !d.hidden && d.classList.contains('on'), mode: d.dataset.mode, visible: [...document.querySelectorAll('#drw-body .hs-card')].filter(c => !c.hidden).map(c => c.id), right: Math.round(b.right), w: Math.round(b.width), orb: { t: o.top, l: o.left, w: o.width } }; });
  const s2 = await sum();
  check('state panel: a click on a sign opens one card at the right', p1.open && p1.mode === 'one' && p1.visible.length === 1 && p1.visible[0] === 'hsc-ai' && p1.right >= innerWidth - 1, JSON.stringify(p1));
  check('state panel: the orb stays where it was', Math.abs(s2.orb.t - s1.orb.t) < 1 && Math.abs(s2.orb.l - s1.orb.l) < 1 && Math.abs(s2.orb.w - s1.orb.w) < 1, JSON.stringify([s1.orb, s2.orb]));
  await page.click('#drw-full'); await wait(500);
  const p2 = await page.evaluate(() => { const d = document.getElementById('drw'), b = d.getBoundingClientRect(); return { mode: d.dataset.mode, visible: [...document.querySelectorAll('#drw-body .hs-card')].filter(c => !c.hidden).length, w: Math.round(b.width), h: Math.round(b.height), hscroll: document.documentElement.scrollWidth > innerWidth + 1 }; });
  check('state panel: all four cards on the whole screen', p2.mode === 'full' && p2.visible === 4 && p2.w >= innerWidth - 2 && p2.h >= innerHeight - 2 && !p2.hscroll, JSON.stringify(p2));
  await page.keyboard.press('Escape'); await wait(500);
  const p3 = await page.evaluate(() => { const d = document.getElementById('drw'); return { hidden: d.hidden, on: d.classList.contains('on'), moved: document.getElementById('page-home').classList.contains('drw-one') || document.getElementById('page-home').classList.contains('drw-full') }; });
  check('state panel: Esc closes it and not the window', p3.hidden && !p3.on && !p3.moved && !closedBefore && !page.isClosed(), JSON.stringify(p3));
  await page.click('#hs-voice'); await wait(500);
  const p4 = await page.evaluate(() => ({ visible: [...document.querySelectorAll('#drw-body .hs-card')].filter(c => !c.hidden).map(c => c.id), detail: document.getElementById('hsc-vd').textContent.length }));
  check('state panel: the card «Голос» says what is going on', p4.visible.length === 1 && p4.visible[0] === 'hsc-voice' && p4.detail > 5, JSON.stringify(p4));
  await page.evaluate(() => statePanel.close()); await wait(400);
  const sysCards = await page.evaluate(() => { show('system'); return ['card-health', 'card-ai', 'card-remote', 'rm-switch', 'ai-check', 'ai-settings', 'rm-renew', 'net-switch', 'ph-ring', 'ph-form'].filter(id => !document.getElementById(id)); });
  check('«Система» keeps all its cards and controls', sysCards.length === 0, sysCards.join(','));
  await page.evaluate(() => show('home'));
  await wait(300);
  check('Esc on the home page does not close the window', !closedBefore && !page.isClosed());

  // the object bar (Object Interaction Engine). Two different things are checked and named so:
  //  (a) the REAL program: the state the installed Jarvis reports is IDLE, the bar is hidden, and the «Стоп» request goes to the real
  //      backend and is answered honestly («nothing to stop», never a success);
  //  (b) the REAL WebView2 renders the bar for every state from the state table - the state is put into the page by objectBar(),
  //      it is NOT produced by the backend (the backend cannot be put into AI_TASK without an AI provider and a real document here).
  await page.evaluate(() => show('home'));
  await wait(800);
  const real = await page.evaluate(async () => { const r = await fetch('/state', { headers: { 'X-Jarvis': '1' } }).then(x => x.json()).catch(e => ({ err: String(e) })); return { state: r.object && r.object.state, hidden: document.getElementById('h-obj').hidden, err: r.err || '' }; });
  check('object bar (real backend): IDLE state, the bar is hidden', real.state === 'IDLE' && real.hidden === true, JSON.stringify(real));
  const stopAnswer = await page.evaluate(async () => { try { return await action('object_stop'); } catch (e) { return { err: String(e) }; } });
  check('object bar (real backend): «Стоп» with nothing to stop is answered honestly, not as a success', stopAnswer && stopAnswer.ok === false && typeof stopAnswer.message === 'string' && stopAnswer.message.length > 3, JSON.stringify(stopAnswer));
  const states = [
    ['DICTATION', { state: 'DICTATION', app: 'word', name: 'Отчёт.docx', operation: 'dictation' }, true],
    ['AI_TASK', { state: 'AI_TASK', app: 'vscode', name: 'calculator.py', operation: 'ai:напиши калькулятор на Python с обработкой деления на ноль и тестами и описанием' }, true],
    ['AWAITING_CONFIRMATION', { state: 'AWAITING_CONFIRMATION', app: 'word', name: 'Отчёт.docx', operation: 'confirm:replace_doc', pending: 'Заменить весь текст документа новым вариантом?' }, true],
    ['PAUSED', { state: 'PAUSED', app: 'word', name: 'Отчёт.docx' }, false],
    ['ERROR_RECOVERY', { state: 'ERROR_RECOVERY', app: 'word', name: 'Отчёт.docx', error: 'Документ «Отчёт.docx» закрыт.' }, false],
  ];
  for (const [name, obj, stopEnabled] of states) {
    const m = await page.evaluate(o => {
      objectBar(Object.assign({ pending: '', saved: false, last_result: '', error: '', operation: '' }, o));
      const r = sel => { const e = document.querySelector(sel); if (!e) return null; const b = e.getBoundingClientRect(); return { t: b.top, b: b.bottom, l: b.left, r: b.right, w: b.width, h: b.height }; };
      const hit = (a, c) => a && c && a.w > 0 && c.w > 0 && !(a.b <= c.t || c.b <= a.t || a.r <= c.l || c.r <= a.l);
      const bar = r('#h-obj'), orb = r('#orb'), form = r('#h-form'), top = r('.h-top'), chips = r('#rail');
      const res = { shown: !document.getElementById('h-obj').hidden, label: document.getElementById('ho-st').textContent, stopOff: document.getElementById('ho-stop').disabled,
        over: hit(bar, orb) || hit(bar, form) || hit(bar, top) || hit(bar, chips), inside: bar && bar.r <= innerWidth + 1 && bar.l >= 0, hscroll: document.documentElement.scrollWidth > innerWidth + 1 };
      objectBar({ state: 'IDLE' });
      return res;
    }, obj);
    check(`object bar (rendered in WebView2, state set by the page): ${name}`, m.shown && m.label.length > 2 && m.stopOff === !stopEnabled && !m.over && m.inside && !m.hscroll, JSON.stringify(m));
  }
  const hiddenAgain = await page.evaluate(() => document.getElementById('h-obj').hidden);
  check('object bar: hidden again after IDLE', hiddenAgain === true);
  // the helpers of the AI card (the beginning of a key suggests a provider, quick models, the host order of OpenRouter)
  await page.evaluate(() => show('settings'));
  await wait(400);
  const guesses = await page.evaluate(() => ['sk-or-v1-abc', 'sk-ant-api03-x', 'AIzaSyAbc', 'gsk_abc', 'sk-proj-abc', 'hello', ''].map(k => { const g = keyGuess(k); return g ? g.provider : null; }));
  check('AI card: a key suggests its provider', JSON.stringify(guesses) === JSON.stringify(['openrouter', 'anthropic', 'gemini', 'custom', 'openai', null, null]), JSON.stringify(guesses));
  const presets = await page.$$eval('#ai-presets button', b => b.length);
  check('AI card: quick model choices are shown', presets >= 3, String(presets));
  const route = await page.evaluate(() => { const sel = document.getElementById('ai-provider'); const before = document.getElementById('ai-route-row').style.display; sel.value = 'openrouter'; updateAiRows(); const on = document.getElementById('ai-route-row').style.display; sel.value = 'none'; updateAiRows(); return [before, on, document.getElementById('ai-route-row').style.display]; });
  check('AI card: the host order of OpenRouter is shown for OpenRouter only', route[0] === 'none' && route[1] === '' && route[2] === 'none', JSON.stringify(route));
  check('AI card: the screen check button exists', await page.$('#ai-screen-check') !== null);

  check('no JavaScript errors in the page', errors.length === 0, errors.slice(0, 3).join(' | '));
  say(`WEBVIEW2 E2E: ${failed === 0 ? 'PASS' : 'FAIL (' + failed + ')'}`);
  require('fs').writeFileSync(reportFile, lines.join('\n'));
  await browser.close();
  process.exit(failed === 0 ? 0 : 1);
})().catch(e => { console.log('WEBVIEW2 E2E: FAIL ' + e.message); process.exit(2); });
