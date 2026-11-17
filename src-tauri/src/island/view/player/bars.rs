//! The player's four level bars: live from the player's loudness, frozen on a dimmed island, a pause
//! sign when paused, and a looping animation when the loudness can't be read.
//!
//! Windows gives one peak level per channel, not frequency bands, so the bars are not an equalizer:
//! each bar sways on its own slow rhythm and the music sets how far (`Waves`).

use super::super::super::comp::{Comp, Motion, color, v2};
use super::super::{DIM, MUTED};
use super::{SAMPLE_MS, STATUS};
use crate::media::Level;
use std::f64::consts::TAU;
use windows::{
    UI::{
        Color,
        Composition::{
            CompositionColorBrush, CompositionObject, CompositionPropertySet,
            CompositionRoundedRectangleGeometry, ShapeVisual,
        },
    },
    core::{HSTRING, Interface, Result},
};

#[derive(Clone, Copy)]
pub(super) enum PausedLook {
    Pair(f32, f32, f32),
    Dots,
}

#[derive(Clone, Copy)]
pub(super) struct BarSpec {
    bar: f32,
    gap: f32,
    pub(super) height: f32,
    min: f32,
    paused: PausedLook,
}

impl BarSpec {
    pub(super) fn width(&self) -> f32 {
        4.0 * self.bar + 3.0 * self.gap
    }

    fn live_x(&self, i: usize) -> f32 {
        i as f32 * (self.bar + self.gap)
    }

    fn height_at(&self, level: f32) -> f32 {
        self.min + (self.height - self.min) * level.clamp(0.0, 1.0)
    }
}

pub(super) const COLLAPSED_BARS: BarSpec = BarSpec {
    bar: 2.0,
    gap: 2.0,
    height: 12.0,
    min: 3.0,
    paused: PausedLook::Pair(2.5, 10.0, 3.0),
};
pub(super) const ANNOUNCE_BARS: BarSpec = BarSpec {
    bar: 3.0,
    gap: 2.0,
    height: 16.0,
    min: 3.0,
    paused: PausedLook::Pair(3.5, 13.0, 4.0),
};
pub(super) const EXPANDED_BARS: BarSpec = BarSpec {
    bar: 3.0,
    gap: 2.0,
    height: 22.0,
    min: 3.0,
    paused: PausedLook::Dots,
};

/// The frozen bars picture: fractions of the block height, as in the mockup (5, 11, 8, 4 of 12).
const FROZEN: [f32; 4] = [5.0 / 12.0, 11.0 / 12.0, 8.0 / 12.0, 4.0 / 12.0];
/// How far each bar moves: the second leads, the outer ones are calmer.
const PROFILE: [f32; 4] = [0.6, 1.0, 0.8, 0.45];
/// The looping animation: a calm sway in the lower part of the block, one period per bar (ms).
const DECORATIVE_PERIOD: [u32; 4] = [2300, 1900, 2100, 2700];
const DECORATIVE_REST: f32 = 0.12;
const DECORATIVE_SWAY: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BarsMode {
    Live,
    Frozen,
    Paused,
    /// Loudness unavailable: a looping animation run by the compositor.
    Decorative,
}

pub(super) struct Bars {
    pub(super) visual: ShapeVisual,
    geometries: Vec<CompositionRoundedRectangleGeometry>,
    props: CompositionPropertySet,
    brush: CompositionColorBrush,
    tint: Color,
    spec: BarSpec,
    waves: Waves,
    sent: [f32; 4],
    mode: Option<BarsMode>,
}

impl Bars {
    pub(super) fn new(comp: &Comp, spec: BarSpec, tint: Color) -> Result<Self> {
        let c = &comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(spec.width(), spec.height))?;
        let props = c.CreatePropertySet()?;
        let brush = comp.brush(tint)?;
        let p: CompositionObject = props.cast()?;
        let mut geometries = Vec::with_capacity(4);
        for i in 0..4 {
            props.InsertScalar(&HSTRING::from(format!("X{i}")), spec.live_x(i))?;
            let g = c.CreateRoundedRectangleGeometry()?;
            g.SetSize(v2(spec.bar, spec.min))?;
            g.SetCornerRadius(v2(spec.bar / 2.0, spec.bar / 2.0))?;
            let shape = c.CreateSpriteShapeWithGeometry(&g)?;
            shape.SetFillBrush(&brush)?;
            // A bar grows from the middle: its vertical position follows the height.
            let go: CompositionObject = g.cast()?;
            comp.bind(
                &shape,
                "Offset",
                &format!("Vector2(p.X{i}, (H - g.Size.Y) / 2)"),
                &[("p", &p), ("g", &go)],
                &[("H", spec.height)],
            )?;
            visual.Shapes()?.Append(&shape)?;
            geometries.push(g);
        }
        Ok(Self {
            visual,
            geometries,
            props,
            brush,
            tint,
            spec,
            waves: Waves::default(),
            sent: [0.0; 4],
            mode: None,
        })
    }

    pub(super) fn is_live(&self) -> bool {
        self.mode == Some(BarsMode::Live)
    }

    fn put(
        &self,
        comp: &Comp,
        i: usize,
        x: f32,
        size: (f32, f32),
        motion: Option<Motion>,
    ) -> Result<()> {
        let g = &self.geometries[i];
        let name = format!("X{i}");
        match motion {
            Some(m) => {
                comp.animate_scalar(&self.props, &name, x, m)?;
                comp.animate_vector2(g, "Size", v2(size.0, size.1), m)
            }
            None => {
                comp.stop(&self.props, &name)?;
                self.props.InsertScalar(&HSTRING::from(name), x)?;
                comp.stop(g, "Size")?;
                g.SetSize(v2(size.0, size.1))
            }
        }
    }

    pub(super) fn set_mode(&mut self, comp: &Comp, mode: BarsMode) -> Result<()> {
        if self.mode == Some(mode) {
            return Ok(());
        }
        // First time: immediately, with no animation: the view has just appeared.
        let animated = self.mode.is_some();
        let pick = |m: Motion| animated.then_some(m);
        self.mode = Some(mode);
        let spec = self.spec;
        let paint = if mode == BarsMode::Paused {
            color(MUTED, 1.0)
        } else {
            self.tint
        };
        if animated {
            comp.animate_color(&self.brush, paint, STATUS)?;
        } else {
            self.brush.SetColor(paint)?;
        }
        match mode {
            BarsMode::Paused => match spec.paused {
                PausedLook::Pair(w, h, gap) => {
                    let left = (spec.width() - (2.0 * w + gap)) / 2.0;
                    for i in 0..4 {
                        let x = if i < 2 { left } else { left + w + gap };
                        self.put(comp, i, x, (w, h), pick(STATUS))?;
                    }
                }
                PausedLook::Dots => {
                    for i in 0..4 {
                        self.put(comp, i, spec.live_x(i), (spec.bar, spec.bar), pick(STATUS))?;
                    }
                }
            },
            BarsMode::Frozen => {
                for (i, f) in FROZEN.iter().enumerate() {
                    let h = spec.height * f;
                    self.put(comp, i, spec.live_x(i), (spec.bar, h), pick(DIM))?;
                }
            }
            BarsMode::Live => {
                for i in 0..4 {
                    let h = spec.height_at(self.waves.level[i]);
                    self.sent[i] = h;
                    self.put(comp, i, spec.live_x(i), (spec.bar, h), pick(STATUS))?;
                }
            }
            BarsMode::Decorative => {
                for i in 0..4 {
                    let rest = v2(spec.bar, spec.height_at(DECORATIVE_REST));
                    let top = v2(
                        spec.bar,
                        spec.height_at(DECORATIVE_REST + DECORATIVE_SWAY * PROFILE[i]),
                    );
                    // Every other bar starts at the top, so they never rise together.
                    let (from, to) = if i % 2 == 0 { (rest, top) } else { (top, rest) };
                    self.put(comp, i, spec.live_x(i), (from.X, from.Y), None)?;
                    comp.loop_vector2(
                        &self.geometries[i],
                        "Size",
                        &[(0.0, from), (0.5, to), (1.0, from)],
                        DECORATIVE_PERIOD[i],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn feed(&mut self, comp: &Comp, level: Level, dt_ms: f32) -> Result<()> {
        if !self.is_live() {
            return Ok(());
        }
        let levels = self.waves.step(level, dt_ms);
        #[cfg(debug_assertions)]
        super::super::super::levels_debug::show_bar(
            self.waves.level[1],
            levels[1] - self.waves.level[1],
            self.waves.hit_at,
        );
        for (i, level) in levels.into_iter().enumerate() {
            let h = self.spec.height_at(level);
            if (h - self.sent[i]).abs() >= 0.25 {
                self.sent[i] = h;
                comp.animate_vector2(
                    &self.geometries[i],
                    "Size",
                    v2(self.spec.bar, h),
                    Motion::linear(SAMPLE_MS + 10),
                )?;
            }
        }
        Ok(())
    }
}

/// Two sines per bar, periods in ms: numbers with no common rhythm, so the bars never line up.
const SWAY: [[f64; 2]; 4] = [
    [1130.0, 1870.0],
    [970.0, 1590.0],
    [1210.0, 2030.0],
    [1370.0, 1730.0],
];
const PEAK_RISE_MS: f32 = 30.0;
const PEAK_FALL_MS: f32 = 180.0;
const BAR_RISE_MS: f32 = 45.0;
const BAR_FALL_MS: f32 = 200.0;
/// The scale, in dB of the peak: this and below is rest, 0 dB is the top. The island has already
/// moved the player's typical loudness to about −6 dB (`loudness`), which lands near half height.
const FLOOR_DB: f32 = -24.0;
/// How much of a bar's height is its own sway; the rest is the sound.
const SWAY_SHARE: f32 = 0.25;

/// A hit (a kick, a snare): the sound jumps at least this much (dB) above the quietest reading of
/// the last `HIT_WINDOW_MS`. Windows gives no frequencies, so any sharp start counts; hi-hats are too
/// quiet against the whole mix to show.
const HIT_DB: f32 = 5.0;
const HIT_WINDOW_MS: f64 = 150.0;
/// After a hit, new ones wait this long, so dense music doesn't keep the bars shaking.
const HIT_CALM_MS: f64 = 120.0;
/// A hit adds this share of each bar's own range, held about one reading, then fading: about 0.13 s
/// in all. Shorter would fall between readings (one every 33 ms).
const POP: f32 = 0.15;
const POP_HOLD_MS: f64 = 35.0;
const POP_FADE_MS: f64 = 30.0;

/// Loudness → bar heights (0 to 1).
#[derive(Default)]
struct Waves {
    peak: f32,
    clock_ms: f64,
    level: [f32; 4],
    /// Raw readings of the last `HIT_WINDOW_MS`: (clock, dB).
    recent: std::collections::VecDeque<(f64, f32)>,
    hit_at: Option<f64>,
}

impl Waves {
    fn step(&mut self, level: Level, dt_ms: f32) -> [f32; 4] {
        let raw = level.left.max(level.right);
        ease(&mut self.peak, raw, dt_ms, PEAK_RISE_MS, PEAK_FALL_MS);
        self.clock_ms += dt_ms as f64;
        self.detect_hit(raw);
        // Squared: the loud end of the scale, where music lives, gets most of the height.
        let db = 20.0 * self.peak.max(1e-6).log10();
        let loud = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0).powi(2);
        for (i, [a, b]) in SWAY.into_iter().enumerate() {
            let t = self.clock_ms;
            let phase = i as f64;
            let sway = (0.5
                + 0.25 * (TAU * t / a + phase).sin()
                + 0.25 * (TAU * t / b + 2.0 * phase).sin()) as f32;
            let target = PROFILE[i] * loud * (1.0 - SWAY_SHARE + SWAY_SHARE * sway);
            ease(&mut self.level[i], target, dt_ms, BAR_RISE_MS, BAR_FALL_MS);
        }
        // Added after the easing, which would otherwise swallow a pop this short.
        let pop = self.pop();
        let mut heights = self.level;
        for (h, profile) in heights.iter_mut().zip(PROFILE) {
            *h = (*h + POP * profile * pop).min(1.0);
        }
        heights
    }

    fn detect_hit(&mut self, raw: f32) {
        let db = 20.0 * raw.max(1e-6).log10();
        let quietest = self.recent.iter().map(|&(_, d)| d).fold(f32::MAX, f32::min);
        let calm = self
            .hit_at
            .is_none_or(|at| self.clock_ms - at >= HIT_CALM_MS);
        if calm && db > FLOOR_DB && db - quietest >= HIT_DB {
            self.hit_at = Some(self.clock_ms);
            // The jump is used up: one step up in loudness is one hit, not one every `HIT_CALM_MS`.
            self.recent.clear();
        }
        self.recent.push_back((self.clock_ms, db));
        while self
            .recent
            .front()
            .is_some_and(|&(t, _)| t <= self.clock_ms - HIT_WINDOW_MS)
        {
            self.recent.pop_front();
        }
    }

    /// 1 right after a hit, fading to 0.
    fn pop(&self) -> f32 {
        let Some(at) = self.hit_at else {
            return 0.0;
        };
        let age = self.clock_ms - at;
        if age < POP_HOLD_MS {
            1.0
        } else {
            (-(age - POP_HOLD_MS) / POP_FADE_MS).exp() as f32
        }
    }
}

fn ease(value: &mut f32, target: f32, dt_ms: f32, rise_ms: f32, fall_ms: f32) {
    let tau = if target > *value { rise_ms } else { fall_ms };
    *value += (target - *value) * (1.0 - (-dt_ms / tau).exp());
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = SAMPLE_MS as f32;

    fn level(peak: f32) -> Level {
        Level {
            left: peak,
            right: peak,
        }
    }

    fn run(w: &mut Waves, peak: f32, ms: f32) -> Vec<[f32; 4]> {
        (0..(ms / STEP) as usize)
            .map(|_| w.step(level(peak), STEP))
            .collect()
    }

    #[test]
    fn steady_music_keeps_every_bar_moving() {
        let mut w = Waves::default();
        run(&mut w, 0.8, 3000.0);
        let steps = run(&mut w, 0.8, 3000.0);
        for i in 0..4 {
            let low = steps.iter().map(|s| s[i]).fold(f32::MAX, f32::min);
            let high = steps.iter().map(|s| s[i]).fold(0.0, f32::max);
            assert!(high - low > 0.05, "bar {i} barely moves: {low}..{high}");
        }
    }

    #[test]
    fn the_second_bar_leads() {
        let mut w = Waves::default();
        run(&mut w, 0.8, 3000.0);
        let steps = run(&mut w, 0.8, 6000.0);
        for i in [0, 2, 3] {
            assert!(
                average(&steps, 1) > average(&steps, i),
                "bar {i} outruns the second"
            );
        }
    }

    fn average(steps: &[[f32; 4]], i: usize) -> f32 {
        steps.iter().map(|s| s[i]).sum::<f32>() / steps.len() as f32
    }

    #[test]
    fn a_beat_lifts_the_bars() {
        let mut w = Waves::default();
        let before = *run(&mut w, 0.4, 8000.0).last().unwrap();
        let after = *run(&mut w, 1.0, 100.0).last().unwrap();
        let total = |h: [f32; 4]| h.iter().sum::<f32>();
        assert!(
            total(after) - total(before) > 0.25,
            "{before:?} → {after:?}"
        );
    }

    #[test]
    fn louder_music_reaches_higher() {
        let quiet = run(&mut Waves::default(), 0.02, 4000.0);
        let loud = run(&mut Waves::default(), 0.9, 4000.0);
        assert!(average(&loud, 1) - average(&quiet, 1) > 0.25);
    }

    #[test]
    fn the_same_sound_keeps_the_same_height() {
        let mut w = Waves::default();
        run(&mut w, 0.6, 1000.0);
        let early = run(&mut w, 0.6, 4000.0);
        let late = run(&mut w, 0.6, 4000.0);
        assert!((average(&early, 1) - average(&late, 1)).abs() < 0.05);
    }

    fn beats(w: &mut Waves, ms: f32) -> Vec<[f32; 4]> {
        (0..(ms / 500.0) as usize)
            .flat_map(|_| {
                let mut s = run(w, 1.0, 100.0);
                s.extend(run(w, 0.3, 400.0));
                s
            })
            .collect()
    }

    #[test]
    fn beats_stay_as_high_through_the_track() {
        let mut w = Waves::default();
        let start = beats(&mut w, 4000.0);
        beats(&mut w, 20_000.0);
        let later = beats(&mut w, 4000.0);
        let top = |s: &[[f32; 4]]| s.iter().map(|h| h[1]).fold(0.0, f32::max);
        assert!(top(&later) > 0.75, "{}", top(&later));
        assert!((top(&start) - top(&later)).abs() < 0.08);
        assert!((average(&start, 1) - average(&later, 1)).abs() < 0.05);
    }

    #[test]
    fn a_sharp_jump_pops_briefly() {
        let mut w = Waves::default();
        run(&mut w, 0.3, 2000.0);
        assert_eq!(w.pop(), 0.0);
        w.step(level(0.6), STEP);
        assert_eq!(w.pop(), 1.0);
        run(&mut w, 0.6, 150.0);
        assert!(w.pop() < 0.1, "{}", w.pop());
    }

    #[test]
    fn a_slow_rise_does_not_pop() {
        let mut w = Waves::default();
        run(&mut w, 0.3, 2000.0);
        for k in 0..30 {
            w.step(level(0.3 + 0.01 * k as f32), STEP);
            assert_eq!(w.pop(), 0.0, "step {k}");
        }
    }

    #[test]
    fn dense_hits_pop_at_most_every_120_ms() {
        let mut w = Waves::default();
        let mut pops = 0;
        for k in 0..60 {
            w.step(level(if k % 2 == 0 { 0.1 } else { 0.8 }), STEP);
            if w.pop() == 1.0 && w.hit_at == Some(w.clock_ms) {
                pops += 1;
            }
        }
        assert!(
            pops <= (60.0 * STEP as f64 / HIT_CALM_MS) as usize + 1,
            "{pops}"
        );
        assert!(pops > 5);
    }

    #[test]
    fn the_second_bar_pops_most() {
        let mut w = Waves::default();
        run(&mut w, 0.2, 2000.0);
        let before = *run(&mut w, 0.2, STEP).last().unwrap();
        let after = w.step(level(0.4), STEP);
        let rise = |i: usize| after[i] - before[i];
        assert!([0, 2, 3].iter().all(|&i| rise(1) > rise(i)));
    }

    #[test]
    fn silence_lets_the_bars_rest() {
        let mut w = Waves::default();
        run(&mut w, 0.8, 2000.0);
        let rest = *run(&mut w, 0.0, 2000.0).last().unwrap();
        assert!(rest.iter().all(|&h| h < 0.02), "{rest:?}");
    }
}
