//! The editor's picture: the two engines' videos fitted into the picture area, each with the zoom
//! and the transition layer the program gives it (`montage::watch::Look`), animated by the
//! compositor until the next change. Over them, while a fragment's zoom is being set, the frame
//! with its corners on the whole picture ([`Framer`]). Until the engines show the first frame a
//! small spinner turns in the picture's middle; then the pictures fade in.

use super::super::super::comp::{Comp, Motion, color, scale, v2, v3};
use super::super::super::log;
use super::super::lens::{Lens, Room};
use super::super::signs::{SPINNER, Spinner};
use crate::montage::curve::{Frame, Layer, MIN_SIDE};
use crate::montage::watch::Look;
use windows::{
    UI::Composition::{
        CompositionRoundedRectangleGeometry, CompositionSpriteShape, ContainerVisual, SpriteVisual,
    },
    Win32::Foundation::HANDLE,
    core::Result,
};
use windows_numerics::Vector3;

struct Deck {
    /// The transition's layer: opacity, shift, scale around the middle; clips the zoom.
    layer: ContainerVisual,
    zoom: ContainerVisual,
    video: Option<SpriteVisual>,
}

pub struct Screen {
    pub root: ContainerVisual,
    /// The decks' pictures: hidden until the first frame is there.
    stage: ContainerVisual,
    spinner: Spinner,
    decks: [Deck; 2],
    full: (u32, u32),
    /// The fitted picture inside the area, in the area's coordinates.
    picture: Room,
    area: Room,
}

impl Screen {
    pub fn new(comp: &Comp, full: (u32, u32)) -> Result<Self> {
        let root = comp.container()?;
        root.SetClip(&comp.compositor.CreateInsetClip()?)?;
        let stage = comp.container()?;
        stage.SetOpacity(0.0)?;
        root.Children()?.InsertAtTop(&stage)?;
        let spinner = Spinner::new(comp)?;
        spinner.show(comp, 0);
        root.Children()?.InsertAtTop(&spinner.root)?;
        let deck = || -> Result<Deck> {
            let layer = comp.container()?;
            layer.SetClip(&comp.compositor.CreateInsetClip()?)?;
            layer.SetOpacity(0.0)?;
            let zoom = comp.container()?;
            layer.Children()?.InsertAtTop(&zoom)?;
            stage.Children()?.InsertAtTop(&layer)?;
            Ok(Deck {
                layer,
                zoom,
                video: None,
            })
        };
        let decks = [deck()?, deck()?];
        Ok(Self {
            root,
            stage,
            spinner,
            decks,
            full,
            picture: (0.0, 0.0, 0.0, 0.0),
            area: (0.0, 0.0, 0.0, 0.0),
        })
    }

    /// The deck's engine has loaded: its swap chain becomes the deck's picture.
    pub fn attach(&mut self, comp: &Comp, deck: usize, handle: HANDLE) -> Result<()> {
        let (_, _, w, h) = self.picture;
        let video = comp.swap_chain(handle, (w.max(1.0), h.max(1.0)))?;
        let d = &mut self.decks[deck];
        d.zoom.Children()?.InsertAtTop(&video)?;
        d.video = Some(video);
        Ok(())
    }

    pub fn layout(&mut self, comp: &Comp, area: Room) -> Result<()> {
        self.area = area;
        comp.stop(&self.root, "Offset")?;
        comp.stop(&self.root, "Scale")?;
        self.root.SetScale(scale(1.0, 1.0))?;
        self.root.SetOffset(v3(area.0, area.1))?;
        self.root.SetSize(v2(area.2, area.3))?;
        let content = (self.full.0 as f32, self.full.1 as f32);
        let mut lens = Lens::new();
        lens.fit((0.0, 0.0, area.2, area.3), content);
        let (w, h) = (content.0 * lens.zoom, content.1 * lens.zoom);
        self.picture = (lens.offset.0, lens.offset.1, w, h);
        self.spinner.root.SetOffset(v3(
            lens.offset.0 + (w - SPINNER) / 2.0,
            lens.offset.1 + (h - SPINNER) / 2.0,
        ))?;
        for d in &self.decks {
            d.layer.SetSize(v2(w, h))?;
            d.layer.SetCenterPoint(v3(w / 2.0, h / 2.0))?;
            if let Some(video) = &d.video {
                video.SetSize(v2(w, h))?;
            }
        }
        Ok(())
    }

    /// The picture in window coordinates.
    pub fn picture(&self) -> Room {
        let (x, y, w, h) = self.picture;
        (self.area.0 + x, self.area.1 + y, w, h)
    }

    /// The picture was at `from` (window coordinates): it moves and grows from there to where
    /// it is now.
    pub fn glide_from(&self, comp: &Comp, from: Room, motion: Motion) -> Result<()> {
        let (to_scale, to_offset) = self.mapping(from);
        comp.stop(&self.root, "Offset")?;
        comp.stop(&self.root, "Scale")?;
        self.root.SetScale(to_scale)?;
        self.root.SetOffset(to_offset)?;
        comp.animate_vector3(&self.root, "Scale", scale(1.0, 1.0), motion)?;
        comp.animate_vector3(&self.root, "Offset", v3(self.area.0, self.area.1), motion)
    }

    /// The picture moves and grows to `to` (window coordinates) and stays there until the next
    /// layout.
    pub fn glide_to(&self, comp: &Comp, to: Room, motion: Motion) -> Result<()> {
        let (to_scale, to_offset) = self.mapping(to);
        comp.animate_vector3(&self.root, "Scale", to_scale, motion)?;
        comp.animate_vector3(&self.root, "Offset", to_offset, motion)
    }

    /// The root's scale and offset that put the picture on `rect`.
    fn mapping(&self, rect: Room) -> (Vector3, Vector3) {
        let (px, py, w, _) = self.picture;
        let s = if w > 0.0 { rect.2 / w } else { 1.0 };
        (scale(s, s), v3(rect.0 - s * px, rect.1 - s * py))
    }

    /// Where the picture would lie in `area`, window coordinates.
    pub fn fitted(&self, area: Room) -> Room {
        let content = (self.full.0 as f32, self.full.1 as f32);
        let mut lens = Lens::new();
        lens.fit(area, content);
        (
            lens.offset.0,
            lens.offset.1,
            content.0 * lens.zoom,
            content.1 * lens.zoom,
        )
    }

    /// The first frame is there: the pictures fade in over the viewer's, the spinner goes.
    pub fn reveal(&self, comp: &Comp) {
        log(comp.animate_scalar(&self.stage, "Opacity", 1.0, REVEAL));
        self.spinner.hide(comp);
    }

    pub fn set_shown(&self, comp: &Comp, shown: bool, motion: Motion) {
        log(comp.animate_scalar(&self.root, "Opacity", if shown { 1.0 } else { 0.0 }, motion));
    }

    /// `whole`: the zoom is being set, the pictures show the whole frame.
    pub fn show(&self, comp: &Comp, looks: &[Option<Look>; 2], whole: bool) {
        for (d, look) in self.decks.iter().zip(looks) {
            log(self.show_deck(comp, d, look.as_ref(), whole));
        }
    }

    fn show_deck(&self, comp: &Comp, d: &Deck, look: Option<&Look>, whole: bool) -> Result<()> {
        let Some(look) = look else {
            comp.stop(&d.layer, "Opacity")?;
            return d.layer.SetOpacity(0.0);
        };
        if look.top {
            self.stage.Children()?.Remove(&d.layer)?;
            self.stage.Children()?.InsertAtTop(&d.layer)?;
        }
        let (px, py, w, h) = self.picture;
        let frame = |f: &Frame| {
            let s = (1.0 / f.side.max(MIN_SIDE)) as f32;
            (
                scale(s, s),
                v3(-(f.x as f32) * w * s, -(f.y as f32) * h * s),
            )
        };
        let frames: Vec<(f64, Frame)> = if whole {
            vec![(0.0, Frame::WHOLE)]
        } else {
            look.frame.clone()
        };
        let sizes: Vec<_> = frames.iter().map(|(t, f)| (*t, frame(f).0)).collect();
        let places: Vec<_> = frames.iter().map(|(t, f)| (*t, frame(f).1)).collect();
        run(comp, &d.zoom, "Scale", &sizes)?;
        run(comp, &d.zoom, "Offset", &places)?;
        let layer = |l: &Layer| {
            (
                l.opacity as f32,
                v3(px + l.shift as f32 * w, py),
                l.scale as f32,
            )
        };
        let opacity: Vec<_> = look.layer.iter().map(|(t, l)| (*t, layer(l).0)).collect();
        let offsets: Vec<_> = look.layer.iter().map(|(t, l)| (*t, layer(l).1)).collect();
        let scales: Vec<_> = look
            .layer
            .iter()
            .map(|(t, l)| (*t, scale(layer(l).2, layer(l).2)))
            .collect();
        run(comp, &d.layer, "Offset", &offsets)?;
        run(comp, &d.layer, "Scale", &scales)?;
        run_scalar(comp, &d.layer, "Opacity", &opacity)
    }
}

/// One key holds the value; more run straight between them over the last key's time.
fn run(
    comp: &Comp,
    target: &ContainerVisual,
    property: &str,
    keys: &[(f64, Vector3)],
) -> Result<()> {
    let total = keys.last().map_or(0.0, |k| k.0);
    if keys.len() < 2 || total <= 0.0 {
        comp.stop(target, property)?;
        let value = keys.first().map_or(v3(0.0, 0.0), |k| k.1);
        return match property {
            "Scale" => target.SetScale(value),
            _ => target.SetOffset(value),
        };
    }
    let path: Vec<_> = keys
        .iter()
        .map(|(t, v)| ((t / total) as f32, *v, Motion::linear(0)))
        .collect();
    comp.path_vector3(target, property, &path, (total * 1000.0).round() as u32)
}

fn run_scalar(
    comp: &Comp,
    target: &ContainerVisual,
    property: &str,
    keys: &[(f64, f32)],
) -> Result<()> {
    let total = keys.last().map_or(0.0, |k| k.0);
    if keys.len() < 2 || total <= 0.0 {
        comp.stop(target, property)?;
        return target.SetOpacity(keys.first().map_or(1.0, |k| k.1));
    }
    let path: Vec<_> = keys.iter().map(|(t, v)| ((t / total) as f32, *v)).collect();
    comp.keyframes(target, property, &path, (total * 1000.0).round() as u32)
}

pub const REVEAL: Motion = Motion::curve(180, 0.25, 0.1, 0.25, 1.0);

const DIM: (&str, f32) = ("#000000", 0.5);
const FRAME_LINE: f32 = 1.5;
const DOT: f32 = 9.0;
/// A corner takes the pointer this far around its dot.
const REACH: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Move,
    /// 0 top left, 1 top right, 2 bottom left, 3 bottom right.
    Corner(usize),
}

pub struct Framer {
    pub root: ContainerVisual,
    dims: [SpriteVisual; 4],
    outline: CompositionRoundedRectangleGeometry,
    _stroke: CompositionSpriteShape,
    dots: [SpriteVisual; 4],
    /// The picture in window coordinates and the frame on it.
    picture: Room,
    frame: Frame,
    shown: bool,
}

impl Framer {
    pub fn new(comp: &Comp, k: f32) -> Result<Self> {
        let c = &comp.compositor;
        let root = comp.container()?;
        root.SetOpacity(0.0)?;
        let dim = || -> Result<SpriteVisual> {
            let s = c.CreateSpriteVisual()?;
            s.SetBrush(&comp.brush(color(DIM.0, DIM.1))?)?;
            root.Children()?.InsertAtTop(&s)?;
            Ok(s)
        };
        let dims = [dim()?, dim()?, dim()?, dim()?];
        let shapes = c.CreateShapeVisual()?;
        shapes.SetSize(v2(10_000.0, 10_000.0))?;
        let outline = c.CreateRoundedRectangleGeometry()?;
        outline.SetCornerRadius(v2(3.0, 3.0))?;
        let stroke = c.CreateSpriteShapeWithGeometry(&outline)?;
        stroke.SetStrokeBrush(&comp.brush(color("#F5F5F7", 1.0))?)?;
        stroke.SetStrokeThickness(FRAME_LINE)?;
        shapes.Shapes()?.Append(&stroke)?;
        root.Children()?.InsertAtTop(&shapes)?;
        let dot = || -> Result<SpriteVisual> {
            let d = comp.picture(DOT, DOT, k, |dc, _| unsafe {
                let brush = dc.CreateSolidColorBrush(
                    &windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                        r: 0.96,
                        g: 0.96,
                        b: 0.97,
                        a: 1.0,
                    },
                    None,
                )?;
                dc.FillEllipse(
                    &windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 {
                            X: DOT / 2.0,
                            Y: DOT / 2.0,
                        },
                        radiusX: DOT / 2.0,
                        radiusY: DOT / 2.0,
                    },
                    &brush,
                );
                Ok(())
            })?;
            root.Children()?.InsertAtTop(&d)?;
            Ok(d)
        };
        let dots = [dot()?, dot()?, dot()?, dot()?];
        Ok(Self {
            root,
            dims,
            outline,
            _stroke: stroke,
            dots,
            picture: (0.0, 0.0, 0.0, 0.0),
            frame: Frame::WHOLE,
            shown: false,
        })
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn show(&mut self, picture: Room, frame: Frame) {
        self.picture = picture;
        self.frame = frame;
        self.shown = true;
        log(self.draw());
        log(self.root.SetOpacity(1.0));
    }

    pub fn hide(&mut self) {
        self.shown = false;
        log(self.root.SetOpacity(0.0));
    }

    /// The frame in window coordinates.
    fn rect(&self) -> Room {
        let (x, y, w, h) = self.picture;
        let f = self.frame;
        (
            x + f.x as f32 * w,
            y + f.y as f32 * h,
            f.side as f32 * w,
            f.side as f32 * h,
        )
    }

    fn draw(&self) -> Result<()> {
        let (px, py, pw, ph) = self.picture;
        let (x, y, w, h) = self.rect();
        let place = |s: &SpriteVisual, l: f32, t: f32, w: f32, h: f32| -> Result<()> {
            s.SetOffset(v3(l, t))?;
            s.SetSize(v2(w.max(0.0), h.max(0.0)))
        };
        let [top, bottom, left, right] = &self.dims;
        place(top, px, py, pw, y - py)?;
        place(bottom, px, y + h, pw, py + ph - y - h)?;
        place(left, px, y, x - px, h)?;
        place(right, x + w, y, px + pw - x - w, h)?;
        self.outline.SetOffset(v2(x, y))?;
        self.outline.SetSize(v2(w, h))?;
        for (i, dot) in self.dots.iter().enumerate() {
            let cx = if i % 2 == 0 { x } else { x + w };
            let cy = if i < 2 { y } else { y + h };
            dot.SetOffset(v3(cx - DOT / 2.0, cy - DOT / 2.0))?;
        }
        Ok(())
    }

    pub fn grip_at(&self, p: (f32, f32)) -> Option<Grip> {
        if !self.shown {
            return None;
        }
        let (x, y, w, h) = self.rect();
        for i in 0..4 {
            let cx = if i % 2 == 0 { x } else { x + w };
            let cy = if i < 2 { y } else { y + h };
            if (p.0 - cx).abs() <= REACH && (p.1 - cy).abs() <= REACH {
                return Some(Grip::Corner(i));
            }
        }
        (p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h).then_some(Grip::Move)
    }

    pub fn on_picture(&self, p: (f32, f32)) -> bool {
        let (x, y, w, h) = self.picture;
        self.shown && p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h
    }

    /// The frame after the pointer moved by `delta` (window pixels) from where `grip` was taken
    /// on the frame `from`.
    pub fn dragged(&mut self, grip: Grip, from: Frame, delta: (f32, f32)) {
        let (_, _, pw, ph) = self.picture;
        let (dx, dy) = (
            f64::from(delta.0 / pw.max(1.0)),
            f64::from(delta.1 / ph.max(1.0)),
        );
        self.frame = match grip {
            Grip::Move => Frame {
                x: (from.x + dx).clamp(0.0, 1.0 - from.side),
                y: (from.y + dy).clamp(0.0, 1.0 - from.side),
                side: from.side,
            },
            Grip::Corner(i) => {
                // The opposite corner stays; the side follows the larger of the two moves.
                let (sx, sy) = (
                    if i % 2 == 0 { -1.0 } else { 1.0 },
                    if i < 2 { -1.0 } else { 1.0 },
                );
                let anchor = (
                    if i % 2 == 0 {
                        from.x + from.side
                    } else {
                        from.x
                    },
                    if i < 2 { from.y + from.side } else { from.y },
                );
                let grow = (dx * sx).max(dy * sy);
                let room_x = if sx > 0.0 { 1.0 - anchor.0 } else { anchor.0 };
                let room_y = if sy > 0.0 { 1.0 - anchor.1 } else { anchor.1 };
                let side = (from.side + grow)
                    .max(MIN_SIDE)
                    .min(room_x.min(room_y))
                    .max(MIN_SIDE.min(room_x.min(room_y)));
                Frame {
                    x: if sx > 0.0 { anchor.0 } else { anchor.0 - side },
                    y: if sy > 0.0 { anchor.1 } else { anchor.1 - side },
                    side,
                }
            }
        };
        log(self.draw());
    }
}
