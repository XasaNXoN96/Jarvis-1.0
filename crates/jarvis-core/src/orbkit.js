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
const LEGACY_THEME = { graphite: 'jarvis', ocean: 'ice', amber: 'avengers', ruby: 'sunset', violet: 'cyberpunk', lime: 'matrix' };
const SPHERES = { dots: 'Частицы', core: 'Реактор', holo: 'Голограмма', neon: 'Неон', wave: 'Волна', orbit: 'Орбиты' };
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
const SPHERE_DRAW = { holo: drawHolo, neon: drawNeon, wave: drawWave, core: drawCore, orbit: drawOrbit };
