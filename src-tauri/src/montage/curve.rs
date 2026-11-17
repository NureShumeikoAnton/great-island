//! The zoom and transition curves: where the zoom frame is at a moment of its fragment, and how
//! the two pictures of a transition are laid over each other. Watching an edit samples them
//! into compositor animations, saving computes them per frame: the same numbers either way.

/// A square of the picture in shares of its sides, so the zoomed picture keeps its proportions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub side: f64,
}

/// The tightest zoom: a frame of this share of the picture's side.
pub const MIN_SIDE: f64 = 0.15;
/// A smooth zoom comes in and goes out this long, less in a short fragment.
const RAMP: f64 = 0.6;
const PUSH_OUT: f64 = 1.3;
const PUSH_IN: f64 = 0.85;

impl Frame {
    pub const WHOLE: Self = Self {
        x: 0.0,
        y: 0.0,
        side: 1.0,
    };

    /// A frame around `center`, kept inside the picture.
    pub fn around(center: (f64, f64), side: f64) -> Self {
        let side = side.clamp(MIN_SIDE, 1.0);
        let place = |c: f64| (c - side / 2.0).clamp(0.0, 1.0 - side);
        Self {
            x: place(center.0),
            y: place(center.1),
            side,
        }
    }

    fn lerp(self, to: Self, t: f64) -> Self {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Self {
            x: mix(self.x, to.x),
            y: mix(self.y, to.y),
            side: mix(self.side, to.side),
        }
    }
}

/// `smooth`: in at the fragment's start, out at its end; otherwise the frame holds throughout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub frame: Frame,
    pub smooth: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Dissolve,
    Black,
    Slide,
    PushIn,
}

/// How one picture of a transition is drawn: `shift` in shares of the width (+ to the right),
/// `scale` around the middle. The incoming picture lies on top, both over black.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub opacity: f64,
    pub shift: f64,
    pub scale: f64,
}

pub const PLAIN: Layer = Layer {
    opacity: 1.0,
    shift: 0.0,
    scale: 1.0,
};

pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
    }
}

/// The zoom frame at `local` result seconds into a fragment `length` long.
pub fn zoom_frame(zoom: Option<Zoom>, local: f64, length: f64) -> Frame {
    let Some(zoom) = zoom else {
        return Frame::WHOLE;
    };
    if !zoom.smooth {
        return zoom.frame;
    }
    let ramp = RAMP.min(length / 3.0);
    if local < ramp {
        Frame::WHOLE.lerp(zoom.frame, ease(local / ramp))
    } else if local > length - ramp {
        zoom.frame
            .lerp(Frame::WHOLE, ease((local - (length - ramp)) / ramp))
    } else {
        zoom.frame
    }
}

/// The outgoing and the incoming picture at `progress` 0..1 of a transition.
pub fn layers(transition: Transition, progress: f64) -> (Layer, Layer) {
    let e = ease(progress);
    match transition {
        Transition::Dissolve => (
            PLAIN,
            Layer {
                opacity: e,
                ..PLAIN
            },
        ),
        Transition::Black => (
            Layer {
                opacity: 1.0 - ease(progress * 2.0),
                ..PLAIN
            },
            Layer {
                opacity: ease(progress * 2.0 - 1.0),
                ..PLAIN
            },
        ),
        Transition::Slide => (
            Layer { shift: -e, ..PLAIN },
            Layer {
                shift: 1.0 - e,
                ..PLAIN
            },
        ),
        Transition::PushIn => (
            Layer {
                opacity: 1.0 - e,
                scale: 1.0 + (PUSH_OUT - 1.0) * e,
                ..PLAIN
            },
            Layer {
                opacity: e,
                scale: PUSH_IN + (1.0 - PUSH_IN) * e,
                ..PLAIN
            },
        ),
    }
}

/// Between samples the compositor runs straight lines: this close they read as the curve.
const SAMPLE: f64 = 1.0 / 20.0;

/// The zoom frame from `local` `from` to `to` of a fragment `length` long, as keys of
/// (seconds after `from`, frame) for a compositor animation. A frame that holds is one key.
pub fn zoom_keys(zoom: Option<Zoom>, length: f64, from: f64, to: f64) -> Vec<(f64, Frame)> {
    let at = |t: f64| (t - from, zoom_frame(zoom, t, length));
    let moving = zoom.is_some_and(|z| z.smooth) && to > from;
    if !moving {
        return vec![at(from)];
    }
    let ramp = RAMP.min(length / 3.0);
    let mut keys = vec![at(from)];
    let mut t = from;
    while t < to {
        let in_ramp = t < ramp || t > length - ramp;
        // Where the frame holds, one jump to the next ramp (or the end) is enough.
        let next = if in_ramp {
            t + SAMPLE
        } else {
            (length - ramp).max(t + SAMPLE)
        };
        t = next.min(to);
        keys.push(at(t));
    }
    keys
}

/// The outgoing or the incoming layer of a transition `overlap` seconds long, from `progress`
/// `from` to `to`, as keys of (seconds after `from`, layer).
pub fn layer_keys(
    transition: Transition,
    overlap: f64,
    (from, to): (f64, f64),
    incoming: bool,
) -> Vec<(f64, Layer)> {
    let pick = |p: f64| {
        let (out, inc) = layers(transition, p);
        ((p - from) * overlap, if incoming { inc } else { out })
    };
    if to <= from || overlap <= 0.0 {
        return vec![pick(from)];
    }
    let steps = (((to - from) * overlap / SAMPLE).ceil() as usize).max(1);
    (0..=steps)
        .map(|i| pick(from + (to - from) * i as f64 / steps as f64))
        .collect()
}

/// The two sounds of a transition cross at equal power: no dip in the middle.
pub fn sound_gains(progress: f64) -> (f64, f64) {
    let angle = progress.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_frame_stays_inside_the_picture() {
        let f = Frame::around((0.95, 0.05), 0.4);
        assert!(near(f.x, 0.6) && near(f.y, 0.0) && near(f.side, 0.4));
        assert!(near(Frame::around((0.5, 0.5), 0.01).side, MIN_SIDE));
    }

    #[test]
    fn a_smooth_zoom_comes_in_holds_and_goes_out() {
        let frame = Frame::around((0.3, 0.3), 0.5);
        let zoom = Some(Zoom {
            frame,
            smooth: true,
        });
        assert_eq!(zoom_frame(zoom, 0.0, 6.0), Frame::WHOLE);
        assert_eq!(zoom_frame(zoom, RAMP, 6.0), frame);
        assert_eq!(zoom_frame(zoom, 3.0, 6.0), frame);
        assert_eq!(zoom_frame(zoom, 6.0, 6.0), Frame::WHOLE);
        let midway = zoom_frame(zoom, RAMP / 2.0, 6.0);
        assert!(midway.side < 1.0 && midway.side > 0.5);
    }

    #[test]
    fn a_zoom_at_once_holds_and_no_zoom_is_the_whole_picture() {
        let frame = Frame::around((0.5, 0.5), 0.5);
        let zoom = Some(Zoom {
            frame,
            smooth: false,
        });
        assert_eq!(zoom_frame(zoom, 0.0, 6.0), frame);
        assert_eq!(zoom_frame(None, 1.0, 6.0), Frame::WHOLE);
    }

    #[test]
    fn a_short_fragment_shortens_the_ramp() {
        let frame = Frame::around((0.5, 0.5), 0.5);
        let zoom = Some(Zoom {
            frame,
            smooth: true,
        });
        assert_eq!(zoom_frame(zoom, 0.3, 0.9), frame);
    }

    #[test]
    fn transitions_start_on_the_outgoing_picture_and_end_on_the_incoming() {
        for t in [
            Transition::Dissolve,
            Transition::Black,
            Transition::Slide,
            Transition::PushIn,
        ] {
            let (out, inc) = layers(t, 0.0);
            assert!(near(out.opacity, 1.0) && near(out.shift, 0.0) && near(out.scale, 1.0));
            assert!(inc.opacity == 0.0 || inc.shift == 1.0, "{t:?}");
            let (_, inc) = layers(t, 1.0);
            assert!(near(inc.opacity, 1.0) && near(inc.shift, 0.0) && near(inc.scale, 1.0));
        }
    }

    #[test]
    fn through_black_is_black_in_the_middle() {
        let (out, inc) = layers(Transition::Black, 0.5);
        assert!(near(out.opacity, 0.0) && near(inc.opacity, 0.0));
    }

    #[test]
    fn a_holding_zoom_is_one_key_and_a_smooth_one_follows_its_ramps() {
        let frame = Frame::around((0.5, 0.5), 0.5);
        let held = Some(Zoom {
            frame,
            smooth: false,
        });
        assert_eq!(zoom_keys(held, 6.0, 1.0, 5.0), [(0.0, frame)]);
        let smooth = Some(Zoom {
            frame,
            smooth: true,
        });
        let keys = zoom_keys(smooth, 6.0, 0.0, 6.0);
        assert_eq!(keys[0], (0.0, Frame::WHOLE));
        assert_eq!(*keys.last().unwrap(), (6.0, Frame::WHOLE));
        assert!(
            keys.len() < 2 * (RAMP / SAMPLE) as usize + 6,
            "{}",
            keys.len()
        );
        assert!(keys.windows(2).all(|w| w[1].0 > w[0].0));
        let later = zoom_keys(smooth, 6.0, 2.0, 3.0);
        assert_eq!(later.first().unwrap().1, frame);
        assert!(near(later.last().unwrap().0, 1.0));
    }

    #[test]
    fn layer_keys_run_from_the_given_progress_to_the_end() {
        let keys = layer_keys(Transition::Dissolve, 0.5, (0.5, 1.0), true);
        assert!(near(keys[0].0, 0.0) && near(keys[0].1.opacity, 0.5));
        let last = keys.last().unwrap();
        assert!(near(last.0, 0.25) && near(last.1.opacity, 1.0));
        assert_eq!(
            layer_keys(Transition::Slide, 0.5, (0.3, 0.3), false).len(),
            1
        );
    }

    #[test]
    fn sounds_cross_at_equal_power() {
        let (a, b) = sound_gains(0.5);
        assert!(near(a * a + b * b, 1.0));
        assert_eq!(sound_gains(0.0), (1.0, 0.0));
    }
}
