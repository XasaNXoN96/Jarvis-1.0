// orbkit: the themes and the spheres shared by the window and the phone page (the page that includes it defines ctx, W, H, level, glow,
// palette(), orbKind(), ORB_KINDS, reducedMotion(), PERF_LOW)
const THEMES = {
  jarvis: [176, 45, 25, 'dots', 'Jarvis'],
  cyberpunk: [55, 100, 320, 'neon', 'Cyberpunk'],
  avengers: [44, 100, 44, 'holo', 'Avengers: Эра Альтрона'],
  matrix: [130, 90, 130, 'wave', 'Matrix'],
  ice: [200, 80, 200, 'core', 'Ice'],
  sunset: [18, 90, 340, 'orbit', 'Sunset']
};
const LEGACY_THEME = { day: 'jarvis', graphite: 'jarvis', ocean: 'ice', amber: 'avengers', ruby: 'sunset', violet: 'cyberpunk', lime: 'matrix' };
const SPHERES = { dots: 'Частицы', core: 'Реактор', holo: 'Голограмма', neon: 'Неон', wave: 'Волна', orbit: 'Орбиты', phoenix: 'Феникс' };
const SPEEDS = { slow: 0.45, normal: 1, fast: 1.8, hyper: 3 }, GLOWS = { soft: 0.6, normal: 1, strong: 1.5 };
// ---- the spheres. Every one draws from the same few things: the colour of the theme and of the state (palette), `level` (how loud/active),
// `t` (the animation clock that already has the chosen speed in it) and the glow multiplier of the setting ----
const TAU = Math.PI * 2;
const glowK = () => GLOWS[glow];
function haze(cx, cy, r, hue, sat, a) {
  const g = ctx.createRadialGradient(cx, cy, r * 0.04, cx, cy, r);
  g.addColorStop(0, `hsla(${hue},${sat}%,55%,${Math.min(0.9, a * glowK())})`); g.addColorStop(1, `hsla(${hue},${sat}%,30%,0)`);
  ctx.fillStyle = g; ctx.fillRect(cx - r, cy - r, r * 2, r * 2);
}
// a point of a globe: [x, y, depth 0..1]
function globe(lon, lat, R, rot, tilt) {
  const x0 = Math.cos(lat) * Math.cos(lon + rot), z0 = Math.cos(lat) * Math.sin(lon + rot), y0 = Math.sin(lat);
  const y = y0 * Math.cos(tilt) - z0 * Math.sin(tilt), z = y0 * Math.sin(tilt) + z0 * Math.cos(tilt);
  return [W / 2 + x0 * R, H / 2 + y * R, (z + 1) / 2];
}
// «Голограмма» (Avengers, Age of Ultron): a gold wire globe with thin rings, a scanning beam and a light flicker, like Jarvis's hologram
function drawHolo(t) {
  const [hue, sat] = palette(), cx = W / 2, cy = H / 2, R = W * 0.3, speaking = orbKind() === 'speaking';
  const flick = 0.9 + 0.1 * Math.sin(t / 47) * Math.sin(t / 131);
  haze(cx, cy, R * 1.6, hue, sat, 0.1 + level * 0.18);
  ctx.save(); ctx.globalAlpha = flick; ctx.lineCap = 'round';
  const rot = t / 2600, tilt = 0.42, k = 1 + (speaking ? 0.035 * level * Math.sin(t / 120) : Math.sin(t / 1100) * 0.008);
  const wire = (f, n, w) => { let prev = null; for (let i = 0; i <= n; i++) { const q = f(i / n); if (prev) { const d = (q[2] + prev[2]) / 2; ctx.strokeStyle = `hsla(${hue},${sat}%,${58 + d * 30}%,${(0.1 + d * 0.62) * Math.min(1.4, glowK())})`; ctx.lineWidth = w; ctx.beginPath(); ctx.moveTo(prev[0], prev[1]); ctx.lineTo(q[0], q[1]); ctx.stroke(); } prev = q; } };
  const M = PERF_LOW ? 8 : 12, P = PERF_LOW ? 6 : 9;
  for (let m = 0; m < M; m++) wire(u => globe(m / M * TAU, (u - 0.5) * Math.PI, R * k, rot, tilt), 40, 2);
  for (let p = 1; p < P; p++) wire(u => globe(u * TAU, (p / P - 0.5) * Math.PI, R * k, rot, tilt), 56, p === (P >> 1) ? 3 : 1.6);
  // two thin orbit rings with ticks
  for (let i = 0; i < 2; i++) {
    const rr = R * (1.22 + i * 0.16), a = t / (3400 + i * 1500) * (i ? -1 : 1), tl = 1.05 + i * 0.35;
    ctx.strokeStyle = `hsla(${hue},${sat}%,72%,${0.5 * glowK()})`; ctx.lineWidth = 2; ctx.beginPath();
    for (let j = 0; j <= 90; j++) { const th = j / 90 * TAU, x = Math.cos(th) * rr, y = Math.sin(th) * rr * Math.cos(tl); const xr = x * Math.cos(a) - y * Math.sin(a), yr = x * Math.sin(a) + y * Math.cos(a); j ? ctx.lineTo(cx + xr, cy + yr) : ctx.moveTo(cx + xr, cy + yr); }
    ctx.stroke();
    for (let j = 0; j < 24; j++) { const th = j / 24 * TAU + a * 2, x = Math.cos(th) * rr, y = Math.sin(th) * rr * Math.cos(tl); ctx.fillStyle = `hsla(${hue},${sat}%,80%,.55)`; ctx.fillRect(cx + x - 2, cy + y - 2, 4, 4); }
  }
  // the scanning beam: a bright band that runs over the globe from top to bottom
  const by = cy - R + ((t / 1700) % 1) * R * 2, half = Math.sqrt(Math.max(0, R * R - (by - cy) ** 2));
  const bg = ctx.createLinearGradient(0, by - 26, 0, by + 26); bg.addColorStop(0, `hsla(${hue},${sat}%,70%,0)`); bg.addColorStop(0.5, `hsla(${hue},${sat}%,75%,${0.32 * glowK()})`); bg.addColorStop(1, `hsla(${hue},${sat}%,70%,0)`);
  ctx.fillStyle = bg; ctx.fillRect(cx - half, by - 26, half * 2, 52);
  // a few bright specks on the surface
  for (let i = 0; i < (PERF_LOW ? 14 : 30); i++) { const q = globe(((i * 2.399) % TAU), Math.asin(((i * 0.618) % 1) * 2 - 1), R * k, rot, tilt); ctx.fillStyle = `hsla(${hue},${sat}%,85%,${0.15 + q[2] * 0.6})`; ctx.fillRect(q[0] - 2, q[1] - 2, 4, 4); }
  ctx.restore();
}
// «Неон» (Cyberpunk): a segmented neon ring, two turning hexagons, a radar sweep and short glitches with a colour split
function drawNeon(t) {
  const [hue, sat] = palette(), cx = W / 2, cy = H / 2, R = W * 0.31, glitch = (Math.sin(t / 233) * Math.sin(t / 71) > 0.93) && !reducedMotion();
  const cyan = ORB_KINDS[orbKind()][1] ? hue : 185, mag = ORB_KINDS[orbKind()][1] ? hue : 322;
  haze(cx, cy, R * 1.5, hue, sat, 0.1 + level * 0.2);
  const ring = (dx, h, alpha) => {
    ctx.strokeStyle = `hsla(${h},100%,${h === hue ? 58 : 62}%,${alpha * Math.min(1.3, glowK())})`; ctx.lineWidth = 7; ctx.lineCap = 'butt';
    const n = 36, rot = t / 1800;
    for (let i = 0; i < n; i++) { if (i % 6 === 5) continue; const a0 = rot + i / n * TAU, len = (i % 3 === 0 ? 0.13 : 0.075) + level * 0.02; ctx.beginPath(); ctx.arc(cx + dx, cy, R, a0, a0 + len); ctx.stroke(); }
  };
  if (glitch) { ring(-10, cyan, 0.55); ring(10, mag, 0.55); }
  ring(0, hue, 0.95);
  const hex = (r, rot, w, alpha) => { ctx.strokeStyle = `hsla(${hue},${sat}%,66%,${alpha * glowK()})`; ctx.lineWidth = w; ctx.beginPath(); for (let i = 0; i <= 6; i++) { const a = rot + i / 6 * TAU; const x = cx + Math.cos(a) * r, y = cy + Math.sin(a) * r; i ? ctx.lineTo(x, y) : ctx.moveTo(x, y); } ctx.stroke(); };
  hex(R * 0.78, t / 3000, 4, 0.85); hex(R * 0.58, -t / 2100, 3, 0.7); hex(R * 0.38 * (1 + level * 0.08), t / 1500, 3, 0.9);
  // the radar sweep
  const sw = t / 1100;
  const g = ctx.createConicGradient ? ctx.createConicGradient(sw, cx, cy) : null;
  if (g) { g.addColorStop(0, `hsla(${hue},${sat}%,60%,${0.4 * glowK()})`); g.addColorStop(0.18, `hsla(${hue},${sat}%,60%,0)`); g.addColorStop(1, `hsla(${hue},${sat}%,60%,0)`); ctx.fillStyle = g; ctx.beginPath(); ctx.arc(cx, cy, R * 0.74, 0, TAU); ctx.fill(); }
  // data ticks around
  ctx.fillStyle = `hsla(${hue},${sat}%,70%,.7)`;
  for (let i = 0; i < 72; i++) { const a = i / 72 * TAU, r0 = R * 1.14, l = i % 6 === 0 ? 16 : 7; ctx.save(); ctx.translate(cx + Math.cos(a) * r0, cy + Math.sin(a) * r0); ctx.rotate(a); ctx.fillRect(0, -1, l, 2); ctx.restore(); }
  ctx.fillStyle = `hsla(${hue},${sat}%,75%,${0.6 + level * 0.4})`; ctx.beginPath(); ctx.arc(cx, cy, 10 + level * 14 + Math.sin(t / 220) * 2, 0, TAU); ctx.fill();
}
// «Волна»: a ring of bars that follow how loud it is
function drawWave(t) {
  const [hue, sat] = palette(), cx = W / 2, cy = H / 2, R0 = W * 0.25, N = PERF_LOW ? 64 : 112;
  haze(cx, cy, R0 * 2.1, hue, sat, 0.08 + level * 0.2);
  ctx.lineCap = 'round'; ctx.lineWidth = PERF_LOW ? 6 : 4.5;
  for (let i = 0; i < N; i++) {
    const a = i / N * TAU - Math.PI / 2, n1 = Math.sin(a * 3 + t / 420) * 0.5 + Math.sin(a * 7 - t / 290) * 0.35 + Math.sin(a * 13 + t / 170) * 0.15;
    const len = 10 + (0.35 + Math.abs(n1)) * (18 + level * 150);
    ctx.strokeStyle = `hsla(${hue},${sat}%,${55 + Math.abs(n1) * 25}%,${(0.45 + Math.abs(n1) * 0.5) * Math.min(1.3, glowK())})`;
    ctx.beginPath(); ctx.moveTo(cx + Math.cos(a) * R0, cy + Math.sin(a) * R0); ctx.lineTo(cx + Math.cos(a) * (R0 + len), cy + Math.sin(a) * (R0 + len)); ctx.stroke();
  }
  ctx.strokeStyle = `hsla(${hue},${sat}%,70%,.55)`; ctx.lineWidth = 2; ctx.beginPath(); ctx.arc(cx, cy, R0 - 14, 0, TAU); ctx.stroke();
  ctx.fillStyle = `hsla(${hue},${sat}%,70%,${0.18 + level * 0.3})`; ctx.beginPath(); ctx.arc(cx, cy, R0 - 24 + level * 10, 0, TAU); ctx.fill();
}
// «Реактор»: concentric rings with ticks that turn in turns, and a bright core
function drawCore(t) {
  const [hue, sat] = palette(), cx = W / 2, cy = H / 2, S = W * 0.45;
  haze(cx, cy, S * 1.05, hue, sat, 0.12 + level * 0.22);
  const rings = [[0.46, 2, 120, 1], [0.38, 6, 24, -1.4], [0.3, 3, 60, 0.8], [0.22, 10, 12, -0.6]];
  rings.forEach(([f, w, n, dir], k) => {
    const r = S * f, rot = t / 2400 * dir;
    ctx.lineWidth = w; ctx.strokeStyle = `hsla(${hue},${sat}%,${60 + k * 6}%,${(0.55 + k * 0.07) * Math.min(1.3, glowK())})`;
    ctx.setLineDash(w > 5 ? [r * TAU / n * 0.62, r * TAU / n * 0.38] : [r * TAU / n * 0.5, r * TAU / n * 0.5]);
    ctx.lineDashOffset = -rot * r; ctx.beginPath(); ctx.arc(cx, cy, r, rot, rot + TAU); ctx.stroke();
  });
  ctx.setLineDash([]);
  // triangular coils of the reactor
  for (let i = 0; i < 10; i++) { const a = i / 10 * TAU + t / 5000; ctx.fillStyle = `hsla(${hue},${sat}%,66%,${0.35 + level * 0.3})`; ctx.beginPath(); ctx.moveTo(cx + Math.cos(a - 0.12) * S * 0.14, cy + Math.sin(a - 0.12) * S * 0.14); ctx.lineTo(cx + Math.cos(a + 0.12) * S * 0.14, cy + Math.sin(a + 0.12) * S * 0.14); ctx.lineTo(cx + Math.cos(a) * S * 0.2, cy + Math.sin(a) * S * 0.2); ctx.closePath(); ctx.fill(); }
  const pulse = 1 + Math.sin(t / 300) * 0.06 + level * 0.25;
  const g = ctx.createRadialGradient(cx, cy, 2, cx, cy, S * 0.12 * pulse); g.addColorStop(0, '#fff'); g.addColorStop(0.4, `hsla(${hue},${sat}%,75%,.95)`); g.addColorStop(1, `hsla(${hue},${sat}%,50%,0)`);
  ctx.fillStyle = g; ctx.beginPath(); ctx.arc(cx, cy, S * 0.12 * pulse, 0, TAU); ctx.fill();
}
// «Орбиты»: a nucleus and three tilted orbits with electrons that leave a trail
function drawOrbit(t) {
  const [hue, sat] = palette(), cx = W / 2, cy = H / 2, R = W * 0.33;
  haze(cx, cy, R * 1.2, hue, sat, 0.12 + level * 0.2);
  for (let k = 0; k < 3; k++) {
    const tilt = k * Math.PI / 3 + t / 9000, ry = R * (0.34 + 0.04 * k), c = Math.cos(tilt), s = Math.sin(tilt);
    const pt = th => [cx + Math.cos(th) * R * c - Math.sin(th) * ry * s, cy + Math.cos(th) * R * s + Math.sin(th) * ry * c];
    ctx.strokeStyle = `hsla(${hue},${sat}%,66%,${0.45 * Math.min(1.3, glowK())})`; ctx.lineWidth = 2; ctx.beginPath();
    for (let j = 0; j <= 72; j++) { const q = pt(j / 72 * TAU); j ? ctx.lineTo(q[0], q[1]) : ctx.moveTo(q[0], q[1]); }
    ctx.stroke();
    const th0 = t / (700 + k * 260) * (k % 2 ? -1 : 1);
    for (let j = 0; j < (PERF_LOW ? 6 : 14); j++) { const q = pt(th0 - j * 0.07 * (k % 2 ? -1 : 1)); ctx.fillStyle = `hsla(${hue},${sat}%,${80 - j * 3}%,${(1 - j / 15) * 0.9})`; ctx.beginPath(); ctx.arc(q[0], q[1], 9 - j * 0.5, 0, TAU); ctx.fill(); }
  }
  const g = ctx.createRadialGradient(cx, cy, 2, cx, cy, 46 + level * 30); g.addColorStop(0, '#fff'); g.addColorStop(0.35, `hsla(${hue},${sat}%,72%,.95)`); g.addColorStop(1, `hsla(${hue},${sat}%,50%,0)`);
  ctx.fillStyle = g; ctx.beginPath(); ctx.arc(cx, cy, 46 + level * 30, 0, TAU); ctx.fill();
}
// «Феникс»: a living face. A round fire-bird with a crest of flames, eyes, brows and a beak. It has MOODS (calm, listening, thinking,
// happy, laughing, surprised, angry, sad, confused, shy, asleep) and moves smoothly from one to another. The mood comes from the state of the
// program (orbKind or oState) and from short reactions that the page sets with setMood(name, ms) (a joke, an insult, a thanks, a failure).
const MOODS = {
  calm:      { label: 'спокоен',    hue: 28,  flame: 0.85, eye: 1,    shape: 0,   brow: 0,    asym: 0, beak: 0.08, blush: 0,    droop: 0,   shake: 0,    bounce: 0.3, tilt: 0,     gx: 0,    gy: 0,    tear: 0 },
  listening: { label: 'слушает',    hue: 34,  flame: 1.05, eye: 1.18, shape: 0,  brow: -0.15, asym: 0, beak: 0.06, blush: 0,    droop: 0,   shake: 0,    bounce: 0.2, tilt: 0.06,  gx: 0,    gy: 0,    tear: 0 },
  thinking:  { label: 'думает',     hue: 30,  flame: 0.7,  eye: 0.8,  shape: 0,  brow: 0.12, asym: 0, beak: 0,    blush: 0,    droop: 0,   shake: 0,    bounce: 0.1, tilt: -0.07, gx: 0.7,  gy: -0.7, tear: 0 },
  speaking:  { label: 'говорит',    hue: 28,  flame: 1,    eye: 1,    shape: 0,  brow: 0,    asym: 0, beak: 0.1,  blush: 0,    droop: 0,   shake: 0,    bounce: 0.5, tilt: 0,     gx: 0,    gy: 0,    tear: 0 },
  happy:     { label: 'радуется',   hue: 42,  flame: 1.25, eye: 1,    shape: 1,  brow: 0,    asym: 0, beak: 0.35, blush: 0.45, droop: 0,   shake: 0,    bounce: 1,   tilt: 0.05,  gx: 0,    gy: 0,    tear: 0 },
  laugh:     { label: 'смеётся',    hue: 44,  flame: 1.35, eye: 1,    shape: 1,  brow: 0,    asym: 0, beak: 0.9,  blush: 0.5,  droop: 0,   shake: 0.5,  bounce: 1.3, tilt: 0,     gx: 0,    gy: 0,    tear: 0 },
  surprised: { label: 'удивлён',    hue: 36,  flame: 1.5,  eye: 1.45, shape: 0,  brow: -0.35, asym: 0, beak: 0.5,  blush: 0,    droop: 0,   shake: 0,    bounce: 0.2, tilt: 0,     gx: 0,    gy: 0,    tear: 0 },
  angry:     { label: 'злится',     hue: 4,   flame: 1.75, eye: 0.62, shape: 0,  brow: 0.75, asym: 0, beak: 0.2,  blush: 0,    droop: 0,   shake: 0.35, bounce: 0.2, tilt: 0,     gx: 0,    gy: 0,    tear: 0 },
  sad:       { label: 'грустит',    hue: 212, flame: 0.3,  eye: 0.85, shape: 0,  brow: -0.6, asym: 0, beak: 0,    blush: 0,    droop: 1,   shake: 0,    bounce: 0,   tilt: 0,     gx: 0,    gy: 0.4,  tear: 1 },
  confused:  { label: 'не понял',   hue: 28,  flame: 0.8,  eye: 1,    shape: 0,  brow: 0.35, asym: 1, beak: 0.05, blush: 0,    droop: 0,   shake: 0,    bounce: 0.1, tilt: 0.14,  gx: 0.4,  gy: -0.3, tear: 0 },
  shy:       { label: 'смущён',     hue: 8,   flame: 0.9,  eye: 0.9,  shape: 0.5, brow: -0.2, asym: 0, beak: 0.05, blush: 1,    droop: 0.2, shake: 0,    bounce: 0.2, tilt: -0.06, gx: -0.6, gy: 0.4,  tear: 0 },
  sleep:     { label: 'спит',       hue: 220, flame: 0.12, eye: 0.05, shape: -1, brow: 0,    asym: 0, beak: 0,    blush: 0,    droop: 0.6, shake: 0,    bounce: 0,   tilt: 0.05,  gx: 0,    gy: 0,    tear: 0 }
};
const MOOD_TAGS = { 'злость': 'angry', 'радость': 'happy', 'смех': 'laugh', 'грусть': 'sad', 'удивление': 'surprised', 'смущение': 'shy', 'непонимание': 'confused', 'спокойствие': 'calm' };
const PX = { cur: Object.assign({}, MOODS.calm), name: 'calm', set: '', until: 0 };
// a short reaction; `weak` ones (a command went well) never push away a stronger one that is still on
function setMood(name, ms, weak) {
  if (!MOODS[name]) return;
  const now = performance.now();
  if (weak && PX.set && now < PX.until) return;
  PX.set = name; PX.until = now + (ms || 2500);
}
function clearMood() { PX.set = ''; PX.until = 0; }
// [злость] at the start or the end of an answer is a mood and is not shown or read: returns [clean text, mood or '']
function moodSplit(text) {
  let mood = '';
  const clean = String(text == null ? '' : text).replace(/\s*\[([а-яё]+)\]\s*/gi, (all, w) => { const m = MOOD_TAGS[w.toLowerCase()]; if (m) { mood = m; return ' '; } return all; }).trim();
  return [clean, mood];
}
// what an answer sounds like (when it has no tag): [mood, how long]
function moodOfAnswer(text) {
  const s = String(text || '').toLowerCase();
  if (/ха-?ха|хах|хех|смешн|рассмеш/.test(s)) return ['laugh', 3200];
  if (/не понял|не поня|не расслыш|повтори|не удалось|не получилось|не нашёл|не нашел|не знаю/.test(s)) return ['confused', 3500];
  if (/извин|прост(и|ите)\b|к сожалению|жаль/.test(s)) return ['sad', 3500];
  if (/ого\b|вау|неужели|ничего себе/.test(s)) return ['surprised', 2800];
  if (/отличн|ура\b|с радостью|поздравля|рад вас|рад тебя|пожалуйста|привет|здравствуй/.test(s)) return ['happy', 3000];
  return ['', 0];
}
// what the owner said: an insult makes it angry, a thanks makes it glad (only when the setting of the reactions is on)
function moodOfWords(text) {
  const s = String(text || '').toLowerCase();
  if (/дурак|тупо[йе]|тупиц|идиот|дебил|чебурек|тормоз|бесишь|ненавижу|заткнись|урод|глуп|ужасн|отстой|бесполезн/.test(s)) return ['angry', 7000];
  if (/молодец|умниц|красавч|спасибо|супер|класс|круто|лучший|люблю тебя|обожаю/.test(s)) return ['happy', 4000];
  return ['', 0];
}
function moodBase() {
  const k = typeof orbKind === 'function' ? orbKind() : (typeof oState !== 'undefined' ? oState : 'idle');
  if (k === 'error') return 'angry';
  if (k === 'off' || k === 'muted' || k === 'locked') return 'sleep';
  if (k === 'listening') return 'listening';
  if (k === 'speaking') return 'speaking';
  if (k === 'thinking' || k === 'recognizing' || k === 'searching' || k === 'executing' || k === 'busy') return 'thinking';
  return 'calm';
}
function moodName() { return PX.set && performance.now() < PX.until ? PX.set : moodBase(); }
function drawPhoenix(t) {
  const name = moodName(), tgt = MOODS[name] || MOODS.calm, c = PX.cur;
  for (const k in tgt) if (k !== 'label') c[k] += (tgt[k] - c[k]) * 0.1;
  PX.name = name;
  const calm = typeof reducedMotion === 'function' && reducedMotion(), m = calm ? 0.2 : 1;
  const kind = typeof orbKind === 'function' ? orbKind() : (typeof oState !== 'undefined' ? oState : 'idle'), speaking = kind === 'speaking';
  const hue = c.hue, s = W / 900, cx = W / 2, cy = H / 2;
  haze(cx, cy, W * 0.5, hue, 95, 0.12 + level * 0.18 + c.flame * 0.04);
  ctx.save(); ctx.globalCompositeOperation = 'source-over';
  ctx.translate(cx + Math.sin(t / 23) * c.shake * 5 * s * m, cy + (c.droop * 16 + Math.sin(t / 520) * 7 * c.bounce * m) * s);
  ctx.rotate(c.tilt + Math.sin(t / 1400) * 0.012 * m); ctx.scale(s, s);
  const col = (dh, l, a) => `hsla(${hue + dh},96%,${l}%,${a == null ? 1 : a})`;
  const tongue = (x, y, w, h, sw, fill) => { ctx.fillStyle = fill; ctx.beginPath(); ctx.moveTo(x - w, y); ctx.bezierCurveTo(x - w, y - h * 0.45, x + sw * 0.4 - w * 0.2, y - h * 0.7, x + sw, y - h); ctx.bezierCurveTo(x + w * 0.15 + sw * 0.5, y - h * 0.62, x + w, y - h * 0.4, x + w, y); ctx.closePath(); ctx.fill(); };
  const layers = [[-6, 52, 1], [16, 60, 0.7], [34, 72, 0.42]];
  const fl = c.flame, sway = (i, a) => Math.sin(t / (250 + i * 37) + i * 1.7) * a * (0.5 + fl * 0.5) * m;
  // wings of fire at the sides and a tail below, then the crest
  for (const side of [-1, 1]) for (const [dh, l, sc] of layers) {
    ctx.save(); ctx.translate(side * 168, 10); ctx.rotate(side * (1.05 + Math.sin(t / 380 + side) * 0.08 * m - c.droop * 0.5)); tongue(0, 0, 44 * sc, 120 * fl * sc, sway(side + 3, 16), col(dh, l)); ctx.restore();
  }
  for (let i = -1; i <= 1; i++) for (const [dh, l, sc] of layers) { ctx.save(); ctx.translate(i * 52, 150); ctx.rotate(Math.PI + i * 0.25); tongue(0, 0, 30 * sc, (70 + (i === 0 ? 30 : 0)) * fl * sc, sway(i + 9, 12), col(dh, l)); ctx.restore(); }
  const hs = [0.5, 0.82, 1, 0.82, 0.5];
  for (const [dh, l, sc] of layers) for (let i = 0; i < 5; i++) { const x = (i - 2) * 62; tongue(x, -135 + Math.abs(x) * 0.18, 40 * sc + 6, 165 * hs[i] * fl * sc, sway(i, 24), col(dh, l)); }
  // the body: a warm light face with a bright outline
  const body = ctx.createRadialGradient(0, -30, 20, 0, 0, 200); body.addColorStop(0, `hsl(${hue},100%,98%)`); body.addColorStop(1, `hsl(${hue},85%,86%)`);
  ctx.fillStyle = body; ctx.strokeStyle = col(0, 52); ctx.lineWidth = 15;
  if (!PERF_LOW) { ctx.shadowColor = col(0, 55, 0.9 * Math.min(1.4, glowK())); ctx.shadowBlur = 26 * glowK() * s; }
  ctx.beginPath(); ctx.ellipse(0, 0, 190, 165, 0, 0, TAU); ctx.fill(); ctx.stroke(); ctx.shadowBlur = 0;
  // cheeks
  if (c.blush > 0.03) for (const side of [-1, 1]) { ctx.fillStyle = `hsla(350,90%,62%,${c.blush * 0.5})`; ctx.beginPath(); ctx.ellipse(side * 118, 44, 34, 20, 0, 0, TAU); ctx.fill(); }
  // eyes: round ones, glad arcs and sleeping lines are mixed by `shape`
  const ink = '#2a1812', blink = !calm && (t % 3900) < 130 ? 0.1 : 1;
  for (const side of [-1, 1]) {
    const ex = side * 68, ey = -18, ry = 34 * c.eye * (c.shape > 0.5 ? 1 : blink);
    const round = 1 - Math.abs(c.shape);
    if (round > 0.02) {
      ctx.globalAlpha = round; ctx.fillStyle = ink; ctx.beginPath(); ctx.ellipse(ex + c.gx * 9, ey + c.gy * 7, 24, Math.max(3, ry), 0, 0, TAU); ctx.fill();
      ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(ex + c.gx * 9 - 7, ey + c.gy * 7 - Math.min(12, ry * 0.35), 6, 0, TAU); ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.strokeStyle = ink; ctx.lineWidth = 12; ctx.lineCap = 'round';
    if (c.shape > 0.02) { ctx.globalAlpha = c.shape; ctx.beginPath(); ctx.arc(ex, ey + 12, 25, Math.PI * 1.12, Math.PI * 1.88); ctx.stroke(); ctx.globalAlpha = 1; }
    if (c.shape < -0.02) { ctx.globalAlpha = -c.shape; ctx.beginPath(); ctx.arc(ex, ey - 8, 24, Math.PI * 0.12, Math.PI * 0.88); ctx.stroke(); ctx.globalAlpha = 1; }
    // brows: slanted down inside = anger, up inside = worry; `asym` turns the right one the other way
    const a = c.brow * (side > 0 ? 1 - 2 * c.asym : 1);
    if (Math.abs(a) > 0.06) {
      const inner = -side, y0 = ey - 50 * Math.max(0.6, c.eye) - 6;
      ctx.globalAlpha = Math.min(1, Math.abs(a) * 2.5); ctx.beginPath();
      ctx.moveTo(ex - inner * 38, y0 - a * 8); ctx.lineTo(ex + inner * 38, y0 + a * 26); ctx.stroke(); ctx.globalAlpha = 1;
    }
  }
  // a tear
  if (c.tear > 0.1) { const ph = (t / 14) % 70; ctx.fillStyle = `hsla(205,90%,70%,${c.tear * (1 - ph / 80)})`; ctx.beginPath(); ctx.ellipse(-92, 26 + ph, 7, 11, 0, 0, TAU); ctx.fill(); }
  // the beak: opens by the mood and moves with the voice while it speaks
  const talk = speaking ? level * 1.3 * (0.5 + 0.5 * Math.sin(t / 85)) : 0, o = Math.max(0, Math.min(1, c.beak + talk)), my = 46;
  if (o > 0.06) { ctx.fillStyle = '#5a1a10'; ctx.beginPath(); ctx.ellipse(0, my + 10 + o * 14, 24 + o * 8, 4 + o * 30, 0, 0, TAU); ctx.fill(); }
  ctx.fillStyle = col(14, 47); ctx.beginPath(); ctx.moveTo(-22, my + 4 + o * 30); ctx.lineTo(22, my + 4 + o * 30); ctx.lineTo(0, my + 24 + o * 46); ctx.closePath(); ctx.fill();
  ctx.fillStyle = col(8, 56); ctx.beginPath(); ctx.moveTo(-28, my - 8); ctx.quadraticCurveTo(0, my - 22, 28, my - 8); ctx.lineTo(0, my + 26); ctx.closePath(); ctx.fill();
  ctx.restore();
  // sparks that rise from the crest; the stronger the flame the more of them
  if (!calm) {
    const n = Math.round((PERF_LOW ? 5 : 11) * Math.min(1.5, c.flame)), top = cy + (-150 * s);
    for (let i = 0; i < n; i++) {
      const ph = (t / (1100 + i * 90) + i * 0.37) % 1, x = cx + Math.sin(i * 12.9 + t / 900) * 150 * s, y = top - ph * 230 * s * Math.min(1.3, c.flame);
      ctx.fillStyle = col(30, 68, (1 - ph) * 0.9); const r = (2 + (i % 3)) * s * 1.3; ctx.fillRect(x - r, y - r, r * 2, r * 2);
    }
  }
}
const SPHERE_DRAW = { holo: drawHolo, neon: drawNeon, wave: drawWave, core: drawCore, orbit: drawOrbit, phoenix: drawPhoenix };
