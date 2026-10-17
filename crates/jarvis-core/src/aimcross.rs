//! Finding the crosshair of a game on the picture of the screen, so that the aim assist leads the mouse to THE CROSSHAIR and not to the middle of
//! the window (games from the third person keep it higher than the middle, and every game is a little different).
//!
//! What is looked for: the usual crosshair of four short thin ticks (up, down, left, right) at the same distance from one point, all of the
//! same kind (bright on dark or dark on bright). A point gets a mark by its WEAKEST tick, so a long line (a horizon, a window edge), a corner
//! or a texture do not pass: they have no four ticks at one distance. A dot or a circle crosshair is not found (the value from the settings
//! of the game is used then). The search is cheap (running sums, no per-pixel loops over the arms), and a found place is trusted only after
//! it is seen in several frames in a row (`CrossTracker`).

// the ticks start this far from the point (pixels) and are this long; the crosshairs of the games lie inside these numbers
const GAP_FROM: usize = 5;
const GAP_TO: usize = 30;
const ARM: usize = 7;
// the free pieces before and after a tick (the gap towards the centre and the end of the tick): a long line has no such pieces
const FREE: usize = 4;
// a tick may be this much longer than ARM before its free end is looked at
const SLACK: usize = 2;
// how far from a point the pictures are read (the ticks, their sides)
pub const MARGIN: usize = GAP_TO + ARM + SLACK + FREE + 3;
// the mark that a crosshair has at least (a real one on the pictures that were tried: 130; the best false place: 36)
pub const MIN_SCORE: f32 = 70.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    pub x: f32,
    pub y: f32,
    pub score: f32,
}

fn luma(bgra: &[u8], n: usize) -> Vec<f32> {
    (0..n).map(|i| bgra[i * 4 + 2] as f32 * 0.299 + bgra[i * 4 + 1] as f32 * 0.587 + bgra[i * 4] as f32 * 0.114).collect()
}

// the best crosshair inside the square of the half-side `reach` around `around` (pixels of the picture `side`×`side`, BGRA)
pub fn find(bgra: &[u8], side: usize, around: (f32, f32), reach: usize) -> Option<Found> {
    if side < 2 * MARGIN + 8 || bgra.len() != side * side * 4 {
        return None;
    }
    let g = luma(bgra, side * side);
    // contrast of a thin line: the pixel against the mean of the two pixels 3 steps to the sides (up and down for a horizontal tick, left and
    // right for a vertical one)
    let mut ch = vec![0f32; side * side];
    let mut cv = vec![0f32; side * side];
    for y in 0..side {
        for x in 0..side {
            let i = y * side + x;
            if y >= 3 && y + 3 < side {
                ch[i] = g[i] - (g[i - 3 * side] + g[i + 3 * side]) / 2.0;
            }
            if x >= 3 && x + 3 < side {
                cv[i] = g[i] - (g[i - 3] + g[i + 3]) / 2.0;
            }
        }
    }
    // running sums: along the rows for ch (horizontal ticks), down the columns for cv (vertical ticks); index +1 so that a sum of [a, b) is p[b] - p[a]
    let w1 = side + 1;
    let mut row = vec![0f32; side * w1];
    let mut col = vec![0f32; side * w1];
    for y in 0..side {
        for x in 0..side {
            row[y * w1 + x + 1] = row[y * w1 + x] + ch[y * side + x];
        }
    }
    for x in 0..side {
        for y in 0..side {
            col[x * w1 + y + 1] = col[x * w1 + y] + cv[y * side + x];
        }
    }
    let r = reach as i64;
    let (cx, cy) = (around.0.round() as i64, around.1.round() as i64);
    let lo = MARGIN as i64;
    let hi = side as i64 - MARGIN as i64;
    let (x0, x1) = ((cx - r).max(lo), (cx + r).min(hi - 1));
    let (y0, y1) = ((cy - r).max(lo), (cy + r).min(hi - 1));
    let kn = GAP_TO - GAP_FROM + 1;
    let mut best: Option<Found> = None;
    // per distance and arm: how bright (and how dark) the tick is, less the brightness (darkness) of its free pieces
    let mut arms = vec![[(0f32, 0f32); 4]; kn];
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (xu, yu) = (x as usize, y as usize);
            // the mean of a piece [from, to) steps away from the point along an arm: d = 0 right, 1 left, 2 down, 3 up
            let seg = |d: usize, from: usize, to: usize| -> f32 {
                let n = (to - from) as f32;
                match d {
                    0 => (row[yu * w1 + xu + to] - row[yu * w1 + xu + from]) / n,
                    1 => (row[yu * w1 + xu - from + 1] - row[yu * w1 + xu - to + 1]) / n,
                    2 => (col[xu * w1 + yu + to] - col[xu * w1 + yu + from]) / n,
                    _ => (col[xu * w1 + yu - from + 1] - col[xu * w1 + yu - to + 1]) / n,
                }
            };
            for (j, k) in (GAP_FROM..=GAP_TO).enumerate() {
                for d in 0..4 {
                    let tick = seg(d, k, k + ARM);
                    let (before, after) = (seg(d, k - FREE, k), seg(d, k + ARM + SLACK, k + ARM + SLACK + FREE));
                    arms[j][d] = (tick - before.max(after).max(0.0), -tick - (-before).max(-after).max(0.0));
                }
            }
            // the ticks may differ by one pixel in their distance: each arm takes the best of three neighbouring distances
            let mut score = 0f32;
            for j in 1..kn - 1 {
                let mut bright = f32::MAX;
                let mut dark = f32::MAX;
                for a in 0..4 {
                    bright = bright.min(arms[j - 1][a].0.max(arms[j][a].0).max(arms[j + 1][a].0));
                    dark = dark.min(arms[j - 1][a].1.max(arms[j][a].1).max(arms[j + 1][a].1));
                }
                score = score.max(bright.max(dark));
            }
            if score >= MIN_SCORE && best.map(|b| score > b.score).unwrap_or(true) {
                best = Some(Found { x: x as f32, y: y as f32, score });
            }
        }
    }
    best
}

// A found place is believed after three frames in a row within 3 pixels; then it is kept, and it is let go after a row of misses.
// Positions are the offsets from the middle of the window in pixels (they do not depend on where the square was cut).
#[derive(Default, Debug)]
pub struct CrossTracker {
    seen: Vec<(f32, f32)>,
    locked: Option<(f32, f32)>,
    misses: u32,
}

impl CrossTracker {
    // the answer is the place to use: Some while it is believed
    pub fn feed(&mut self, found: Option<(f32, f32)>) -> Option<(i32, i32)> {
        match (found, self.locked) {
            (Some(p), Some(l)) if (p.0 - l.0).abs() <= 6.0 && (p.1 - l.1).abs() <= 6.0 => {
                self.misses = 0;
                // a slow follow of a small drift
                self.locked = Some((l.0 * 0.8 + p.0 * 0.2, l.1 * 0.8 + p.1 * 0.2));
            }
            (Some(p), Some(_)) => {
                // found elsewhere: a new place counts when it repeats (the game was switched between first and third person)
                self.misses += 1;
                self.seen.push(p);
                if self.seen.len() > 2 {
                    self.seen.remove(0);
                }
                if self.seen.len() == 2 && self.seen.iter().all(|s| (s.0 - p.0).abs() <= 3.0 && (s.1 - p.1).abs() <= 3.0) {
                    self.locked = Some(p);
                    self.seen.clear();
                    self.misses = 0;
                } else if self.misses >= 4 {
                    self.locked = None;
                    self.seen.clear();
                    self.misses = 0;
                }
            }
            (None, Some(_)) => {
                self.misses += 1;
                if self.misses >= 4 {
                    self.locked = None;
                    self.seen.clear();
                    self.misses = 0;
                }
            }
            (Some(p), None) => {
                self.seen.push(p);
                if self.seen.len() > 3 {
                    self.seen.remove(0);
                }
                if self.seen.len() == 3 && self.seen.iter().all(|s| (s.0 - p.0).abs() <= 3.0 && (s.1 - p.1).abs() <= 3.0) {
                    let n = self.seen.len() as f32;
                    self.locked = Some((self.seen.iter().map(|s| s.0).sum::<f32>() / n, self.seen.iter().map(|s| s.1).sum::<f32>() / n));
                    self.seen.clear();
                    self.misses = 0;
                }
            }
            (None, None) => {
                // a break of the row: the count starts again
                self.seen.clear();
            }
        }
        self.locked.map(|l| (l.0.round() as i32, l.1.round() as i32))
    }

    pub fn reset(&mut self) {
        *self = CrossTracker::default();
    }

    // the lock is let go (a new press of the aim button): it is found again at once
    pub fn release(&mut self) {
        self.reset();
    }

    pub fn current(&self) -> Option<(i32, i32)> {
        self.locked.map(|l| (l.0.round() as i32, l.1.round() as i32))
    }

    pub fn is_locked(&self) -> bool {
        self.locked.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a textured picture (a game is never flat): small noise and big soft shapes
    fn scene(side: usize) -> Vec<u8> {
        scene_with(side, 90)
    }

    fn scene_with(side: usize, base: i32) -> Vec<u8> {
        let mut v = vec![255u8; side * side * 4];
        for y in 0..side {
            for x in 0..side {
                let n = ((x * 7 + y * 13 + (x * y) % 17) % 23) as i32;
                let soft = (((x as f32 / 29.0).sin() + (y as f32 / 41.0).cos()) * 25.0) as i32;
                let c = (base + n * 2 + soft).clamp(0, 255) as u8;
                let i = (y * side + x) * 4;
                v[i] = c;
                v[i + 1] = c.saturating_add(5);
                v[i + 2] = c.saturating_sub(5);
            }
        }
        v
    }

    fn put(v: &mut [u8], side: usize, x: usize, y: usize, c: u8) {
        let i = (y * side + x) * 4;
        v[i] = c;
        v[i + 1] = c;
        v[i + 2] = c;
    }

    // four ticks 2 pixels thick: `gap` from the centre, `len` long
    fn crosshair(v: &mut [u8], side: usize, cx: usize, cy: usize, gap: usize, len: usize, c: u8) {
        for t in 0..len {
            for w in 0..2 {
                put(v, side, cx + gap + t, cy + w, c);
                put(v, side, cx - gap - t, cy + w, c);
                put(v, side, cx + w, cy + gap + t, c);
                put(v, side, cx + w, cy - gap - t, c);
            }
        }
    }

    #[test]
    fn a_crosshair_of_four_ticks_is_found_where_it_is_bright_or_dark() {
        let side = 300;
        for (c, name, base) in [(255u8, "bright", 90), (10u8, "dark", 150)] {
            let mut v = scene_with(side, base);
            crosshair(&mut v, side, 140, 120, 14, 9, c);
            let f = find(&v, side, (150.0, 150.0), 80).unwrap_or_else(|| panic!("{} crosshair not found", name));
            assert!((f.x - 140.5).abs() <= 1.5 && (f.y - 120.5).abs() <= 1.5, "{}: {:?}", name, f);
            assert!(f.score >= MIN_SCORE);
        }
    }

    #[test]
    fn a_picture_without_a_crosshair_gives_nothing_also_with_lines_edges_and_corners() {
        let side = 300;
        let mut v = scene(side);
        assert_eq!(find(&v, side, (150.0, 150.0), 100), None, "texture only");
        // a long bright line (a horizon) and a long vertical one (a pole): no four ticks at one distance
        for x in 20..280 {
            put(&mut v, side, x, 140, 255);
            put(&mut v, side, x, 141, 255);
        }
        for y in 20..280 {
            put(&mut v, side, 90, y, 255);
            put(&mut v, side, 91, y, 255);
        }
        assert_eq!(find(&v, side, (150.0, 150.0), 100), None, "lines");
        // a cross made of two long lines is not four ticks either (the ticks have gaps)
        let mut w = scene(side);
        for t in 0..120 {
            for k in 0..2 {
                put(&mut w, side, 100 + t, 150 + k, 255);
                put(&mut w, side, 150 + k, 100 + t, 255);
            }
        }
        assert_eq!(find(&w, side, (150.0, 150.0), 40), None, "a plus of long lines");
    }

    #[test]
    fn it_looks_only_inside_the_reach_and_a_picture_that_is_too_small_gives_nothing() {
        let side = 300;
        let mut v = scene(side);
        crosshair(&mut v, side, 60, 60, 14, 9, 255);
        assert_eq!(find(&v, side, (200.0, 200.0), 40), None, "outside of the reach");
        assert!(find(&v, side, (60.0, 60.0), 40).is_some());
        assert_eq!(find(&vec![0u8; 60 * 60 * 4], 60, (30.0, 30.0), 10), None);
        assert_eq!(find(&v[..100], side, (150.0, 150.0), 40), None, "a wrong size");
    }

    #[test]
    fn a_place_is_believed_after_three_equal_frames_kept_and_let_go_after_misses() {
        let mut t = CrossTracker::default();
        assert_eq!(t.feed(Some((1.0, -92.0))), None);
        assert_eq!(t.feed(Some((1.5, -91.0))), None);
        assert_eq!(t.feed(Some((1.0, -92.0))), Some((1, -92)), "three in a row");
        // a single miss does not let it go
        assert_eq!(t.feed(None), Some((1, -92)));
        assert_eq!(t.feed(Some((1.0, -92.0))), Some((1, -92)));
        // four misses do
        for _ in 0..3 {
            assert!(t.feed(None).is_some());
        }
        assert_eq!(t.feed(None), None);
        assert!(!t.is_locked());
        // places that jump around are never believed
        let mut j = CrossTracker::default();
        for p in [(0.0, 0.0), (40.0, 0.0), (0.0, 40.0), (-40.0, 10.0), (20.0, 20.0)] {
            assert_eq!(j.feed(Some(p)), None);
        }
        // a break of the row starts the count again
        let mut b = CrossTracker::default();
        b.feed(Some((5.0, 5.0)));
        b.feed(Some((5.0, 5.0)));
        b.feed(None);
        assert_eq!(b.feed(Some((5.0, 5.0))), None);
    }

    // A check against a REAL screenshot (not run normally: the picture is not in the repository). AIM_REAL_BGRA, AIM_REAL_SIDE and, for a picture
    // that has a crosshair, AIM_REAL_CROSS="x,y" (where it is in that picture, to a pixel or two).
    #[test]
    fn a_real_screenshot_of_a_game_gives_its_crosshair() {
        let (Ok(raw), Ok(side)) = (std::env::var("AIM_REAL_BGRA"), std::env::var("AIM_REAL_SIDE")) else { return };
        let side: usize = side.parse().unwrap();
        let frame = std::fs::read(raw).unwrap();
        let found = find(&frame, side, (side as f32 / 2.0, side as f32 / 2.0), side / 2 - MARGIN);
        eprintln!("REAL CROSS {:?}", found);
        match std::env::var("AIM_REAL_CROSS") {
            Ok(want) => {
                let w: Vec<f32> = want.split(',').map(|v| v.parse().unwrap()).collect();
                let f = found.expect("the crosshair must be found");
                assert!((f.x - w[0]).abs() <= 2.0 && (f.y - w[1]).abs() <= 2.0, "{:?} vs {:?}", f, w);
            }
            Err(_) => assert!(found.is_none(), "a picture without a crosshair gave {:?}", found),
        }
    }

    #[test]
    fn a_lock_without_confirmation_expires_and_a_new_place_replaces_it() {
        let mut t = CrossTracker::default();
        for _ in 0..3 {
            t.feed(Some((100.0, 100.0)));
        }
        assert!(t.is_locked());
        // nothing is found any more: the lock expires
        for _ in 0..4 {
            t.feed(None);
        }
        assert!(!t.is_locked());
        // a lock, then the same new place twice: moved
        for _ in 0..3 {
            t.feed(Some((100.0, 100.0)));
        }
        t.feed(Some((150.0, 90.0)));
        let r = t.feed(Some((151.0, 90.0)));
        assert_eq!(r, Some((151, 90)));
    }
}
