//! The pictures of the large "Connected" notice (`crate::devices::Look`) as line drawings, drawn
//! by Direct2D once per notice and shown through silver (`device.rs`): the earbuds of three known
//! models and a neutral pair (a bud or the case), over-ear headphones and a speaker. The small
//! notices use the island's glyphs instead.
//!
//! The earbuds and the headphones are traced outlines (`traced.rs`). A bud's own lines are
//! thickened to the notice's line, so the buds and the case have one weight; the headphones stay
//! as drawn. One case serves every model.

mod traced;

use super::super::comp::color;
use crate::devices::Look;
use traced::*;
use windows::{
    Win32::Graphics::Direct2D::{
        Common::{
            D2D_RECT_F, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
            D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
        },
        D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE, D2D1_LINE_JOIN_ROUND,
        D2D1_PRIMITIVE_BLEND_COPY, D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_ROUNDED_RECT,
        D2D1_STROKE_STYLE_PROPERTIES, ID2D1DeviceContext, ID2D1Factory, ID2D1Geometry,
        ID2D1PathGeometry, ID2D1StrokeStyle,
    },
    core::{Interface, Result},
};
use windows_numerics::{Matrix3x2, Vector2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Left,
    Right,
    Case,
    /// A device without a case.
    Whole,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum S {
    M(f32, f32),
    L(f32, f32),
    /// A cubic curve: two control points and the end.
    C(f32, f32, f32, f32, f32, f32),
}

use S::*;

#[derive(Clone, Copy)]
enum Shape {
    /// `true`: the outline is closed; an open one still fills as if closed.
    Path(&'static [S], bool),
    Oval(f32, f32, f32, f32),
    Round(f32, f32, f32, f32, f32),
    /// SVG path data, see `traced.rs`.
    Traced(&'static str),
}

use Shape::*;

#[derive(Clone, Copy)]
enum Ink {
    /// The outline over a body that hides what was drawn behind it: draw back to front.
    Body,
    Line,
    /// A line so many times the line's width.
    Wide(f32),
    /// A filled outline whose own lines are this thick, in the drawing's units: widened to the
    /// line when thinner.
    Solid(f32),
    /// A filled outline kept at its own weight.
    Fill,
    Dot,
}

use Ink::*;

struct Art {
    items: &'static [(Ink, Shape)],
    /// From the drawing's own units to its box: scale and offset.
    k: (f32, f32),
    at: (f32, f32),
}

const BUD_BOX: (f32, f32) = (64.0, 104.0);
const CASE_BOX: (f32, f32) = (112.0, 104.0);
const SPEAKER_BOX: (f32, f32) = (80.0, 100.0);
const OVER_EAR_BOX: (f32, f32) = (140.0, 104.0);

const CASE_K: f32 = 96.0 / 192.68;

/// From the user's case drawing, 192.68 units wide.
const CASE: Art = Art {
    items: &[
        (
            Body,
            Path(
                &[
                    M(71.63, 33.32),
                    L(176.31, 33.32),
                    C(198.31, 33.32, 220.31, 55.32, 220.31, 77.32),
                    L(220.31, 131.43),
                    C(220.31, 153.43, 198.31, 175.43, 176.31, 175.43),
                    L(71.63, 175.43),
                    C(49.63, 175.43, 27.63, 153.43, 27.63, 131.43),
                    L(27.63, 77.32),
                    C(27.63, 55.32, 49.63, 33.32, 71.63, 33.32),
                ],
                true,
            ),
        ),
        (Line, Path(&[M(27.63, 79.76), L(220.31, 79.76)], false)),
        (Wide(1.8), Path(&[M(82.5, 79.3), L(164.8, 79.3)], false)),
        (Dot, Oval(124.0, 104.0, 8.0, 8.0)),
    ],
    k: (CASE_K, CASE_K),
    at: (56.0 - 123.97 * CASE_K, 56.0 - 104.375 * CASE_K),
};

/// In its box's units.
const SPEAKER: Art = Art {
    items: &[
        (Body, Round(14.0, 8.0, 52.0, 84.0, 12.0)),
        (Line, Oval(40.0, 60.0, 16.0, 16.0)),
        (Line, Oval(40.0, 60.0, 5.0, 5.0)),
        (Line, Oval(40.0, 26.0, 6.0, 6.0)),
    ],
    k: (1.0, 1.0),
    at: (0.0, 0.0),
};

/// The right bud, and the left one where it is drawn on its own rather than mirrored.
fn buds(look: Look) -> (&'static Art, Option<&'static Art>) {
    match look {
        Look::AirPodsPro => (&AIRPODS_BUD, Some(&AIRPODS_LEFT)),
        Look::GalaxyBuds => (&GALAXY_BUD, None),
        Look::GalaxyBuds3 => (&BUDS3_BUD, Some(&BUDS3_LEFT)),
        _ => (&PLAIN_BUD, Some(&PLAIN_LEFT)),
    }
}

/// The part's drawing, whether it is mirrored within its box, and the box.
fn piece(look: Look, part: Part) -> (&'static Art, bool, (f32, f32)) {
    match (part, buds(look)) {
        (Part::Right, (right, _)) => (right, false, BUD_BOX),
        (Part::Left, (_, Some(left))) => (left, false, BUD_BOX),
        (Part::Left, (right, None)) => (right, true, BUD_BOX),
        (Part::Case, _) => (&CASE, false, CASE_BOX),
        (Part::Whole, _) if look == Look::Speaker => (&SPEAKER, false, SPEAKER_BOX),
        (Part::Whole, _) => (&OVER_EAR, false, OVER_EAR_BOX),
    }
}

fn to_box(art: &Art, mirrored: bool, size: (f32, f32)) -> Matrix3x2 {
    let own = Matrix3x2 {
        M11: art.k.0,
        M12: 0.0,
        M21: 0.0,
        M22: art.k.1,
        M31: art.at.0,
        M32: art.at.1,
    };
    if !mirrored {
        return own;
    }
    own * Matrix3x2 {
        M11: -1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: size.0,
        M32: 0.0,
    }
}

pub(super) fn aspect(look: Look, part: Part) -> f32 {
    let (_, _, (w, h)) = piece(look, part);
    w / h
}

/// Draws the part `width` DIPs wide with lines `line` DIPs thick in `hex`. `base` is the
/// surface's transform.
pub(super) fn draw(
    dc: &ID2D1DeviceContext,
    base: &Matrix3x2,
    look: Look,
    part: Part,
    width: f32,
    line: f32,
    hex: &str,
) -> Result<()> {
    let (art, mirrored, size) = piece(look, part);
    let k = width / size.0;
    let m = to_box(art, mirrored, size);
    let to_surface = Matrix3x2 {
        M11: k,
        M12: 0.0,
        M21: 0.0,
        M22: k,
        M31: 0.0,
        M32: 0.0,
    } * *base;
    let pen = Pen::new(dc, color_f(hex, 1.0))?;
    let scale = (m.M11 * m.M11 + m.M12 * m.M12).sqrt() * k;
    unsafe { dc.SetTransform(&(m * to_surface)) };
    let drawn = pen.art(art, line / scale);
    unsafe {
        dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
        dc.SetTransform(base);
    }
    drawn
}

/// SVG path data of our own: `None` on anything but absolute M, L, C and Z with their numbers.
fn parse(d: &str) -> Option<Vec<S>> {
    let mut out = Vec::new();
    let mut rest = d.trim_start();
    while let Some(cmd) = rest.chars().next() {
        rest = &rest[cmd.len_utf8()..];
        let end = rest
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let n = rest[..end]
            .split_ascii_whitespace()
            .map(str::parse)
            .collect::<std::result::Result<Vec<f32>, _>>()
            .ok()?;
        rest = &rest[end..];
        out.push(match (cmd, n.as_slice()) {
            ('M', &[x, y]) => M(x, y),
            ('L', &[x, y]) => L(x, y),
            ('C', &[x1, y1, x2, y2, x, y]) => C(x1, y1, x2, y2, x, y),
            // Every figure is closed when the next one begins.
            ('Z', []) => continue,
            _ => return None,
        });
    }
    Some(out)
}

fn color_f(hex: &str, alpha: f32) -> D2D1_COLOR_F {
    let c = color(hex, alpha);
    D2D1_COLOR_F {
        r: f32::from(c.R) / 255.0,
        g: f32::from(c.G) / 255.0,
        b: f32::from(c.B) / 255.0,
        a: f32::from(c.A) / 255.0,
    }
}

fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

struct Pen<'a> {
    dc: &'a ID2D1DeviceContext,
    factory: ID2D1Factory,
    round: ID2D1StrokeStyle,
    ink: D2D1_COLOR_F,
}

impl<'a> Pen<'a> {
    fn new(dc: &'a ID2D1DeviceContext, ink: D2D1_COLOR_F) -> Result<Self> {
        unsafe {
            let factory = dc.GetFactory()?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            Ok(Self {
                dc,
                factory,
                round,
                ink,
            })
        }
    }

    /// `line` in the art's own units.
    fn art(&self, art: &Art, line: f32) -> Result<()> {
        let ink = unsafe { self.dc.CreateSolidColorBrush(&self.ink, None)? };
        let clear = unsafe {
            self.dc.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
                None,
            )?
        };
        for &(kind, shape) in art.items {
            let geometry = self.geometry(shape)?;
            unsafe {
                match kind {
                    Body => {
                        // Copied rather than blended, the transparent fill clears the lines
                        // behind the body; its own outline then covers the fill's edge.
                        self.dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
                        self.dc.FillGeometry(&geometry, &clear, None);
                        self.dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
                        self.dc.DrawGeometry(&geometry, &ink, line, &self.round);
                    }
                    Line => self.dc.DrawGeometry(&geometry, &ink, line, &self.round),
                    Wide(times) => self
                        .dc
                        .DrawGeometry(&geometry, &ink, line * times, &self.round),
                    Solid(own) => {
                        self.dc.FillGeometry(&geometry, &ink, None);
                        // A stroke over the outline widens each line by its width, half on
                        // either side.
                        if line > own {
                            self.dc
                                .DrawGeometry(&geometry, &ink, line - own, &self.round);
                        }
                    }
                    Fill | Dot => self.dc.FillGeometry(&geometry, &ink, None),
                }
            }
        }
        Ok(())
    }

    fn geometry(&self, shape: Shape) -> Result<ID2D1Geometry> {
        unsafe {
            match shape {
                Oval(cx, cy, rx, ry) => self
                    .factory
                    .CreateEllipseGeometry(&D2D1_ELLIPSE {
                        point: v(cx, cy),
                        radiusX: rx,
                        radiusY: ry,
                    })?
                    .cast(),
                Round(x, y, w, h, r) => self
                    .factory
                    .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: x,
                            top: y,
                            right: x + w,
                            bottom: y + h,
                        },
                        radiusX: r,
                        radiusY: r,
                    })?
                    .cast(),
                Path(segs, closed) => self.path(segs, closed)?.cast(),
                // The data is held by a test; broken data would only draw nothing.
                Traced(d) => self.path(&parse(d).unwrap_or_default(), true)?.cast(),
            }
        }
    }

    /// The figures fill even-odd (Direct2D's default), as the traced files expect.
    fn path(&self, segs: &[S], closed: bool) -> Result<ID2D1PathGeometry> {
        let end = if closed {
            D2D1_FIGURE_END_CLOSED
        } else {
            D2D1_FIGURE_END_OPEN
        };
        unsafe {
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            let mut open = false;
            for seg in segs {
                match *seg {
                    M(x, y) => {
                        if open {
                            sink.EndFigure(end);
                        }
                        // Filled even when open: a body hides what is behind it either way.
                        sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    L(x, y) => sink.AddLine(v(x, y)),
                    C(x1, y1, x2, y2, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: v(x1, y1),
                        point2: v(x2, y2),
                        point3: v(x, y),
                    }),
                }
            }
            if open {
                sink.EndFigure(end);
            }
            sink.Close()?;
            Ok(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traced_drawings_parse_and_fit_their_box() {
        let looks = [
            Look::AirPodsPro,
            Look::GalaxyBuds,
            Look::GalaxyBuds3,
            Look::Earbuds,
            Look::OverEar,
        ];
        for look in looks {
            for part in [Part::Left, Part::Right, Part::Whole] {
                let (art, mirrored, size) = piece(look, part);
                let m = to_box(art, mirrored, size);
                for (_, shape) in art.items {
                    let Traced(d) = shape else { continue };
                    let segs = parse(d).expect("our own path data");
                    assert!(matches!(segs.first(), Some(M(..))));
                    for seg in segs {
                        let (x, y) = match seg {
                            M(x, y) | L(x, y) | C(_, _, _, _, x, y) => (x, y),
                        };
                        let (bx, by) = (x * m.M11 + m.M31, y * m.M22 + m.M32);
                        assert!(
                            (-1.0..=size.0 + 1.0).contains(&bx)
                                && (-1.0..=size.1 + 1.0).contains(&by),
                            "{look:?} {part:?} at {bx}, {by}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn parse_refuses_what_it_does_not_know() {
        assert_eq!(parse("M1 2L3 4Z"), Some(vec![M(1.0, 2.0), L(3.0, 4.0)]));
        assert_eq!(parse("M1 2l3 4"), None);
        assert_eq!(parse("M1 2C3 4"), None);
    }
}
