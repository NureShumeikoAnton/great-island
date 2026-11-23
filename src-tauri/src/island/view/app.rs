//! The apps on the island: the row of apps pulled up from the island (`Mode::Apps`) and an
//! app's page (`Mode::App`), its `apps::ui` tree laid out and drawn in the island's look.
//!
//! The layout is pure ([`lay`]): every part has a fixed height (a text line by its style, a key,
//! a bar), a row and a grid share their width equally. A page whose tree keeps its shape is
//! updated in place: only its texts change, so a typed digit doesn't cross-fade the page. A
//! number too wide for its line takes a smaller font, never an ellipsis first.

use super::super::comp::{Motion, Text, TextStyle, color, scale, v2, v3};
use super::super::glyph::Glyph;
use super::{Button, Content, Hit, LATCH, MUTED, Model, Rect, Scene, TEXT, menu, style};
use crate::apps::{
    Icon,
    ui::{Align, Face, Node, Style, Tone},
};
use windows::{
    UI::Composition::{ContainerVisual, Visual},
    core::{Interface, Result},
};

const PAD: f32 = 14.0;
/// The ‹ and the app's title over the page.
const BAR: f32 = 24.0;
const BAR_GAP: f32 = 10.0;
const GAP: f32 = 8.0;
const KEY: f32 = 44.0;
const KEY_RADIUS: f32 = 14.0;
const PROGRESS: f32 = 4.0;
const DISPLAY: f32 = 40.0;
/// A number that doesn't fit shrinks to this before it loses its end.
const DISPLAY_MIN: f32 = 20.0;

const TILE: f32 = 44.0;
const TILE_RADIUS: f32 = 12.0;
const TILE_ICON: f32 = 22.0;
const TILE_COLUMN: f32 = 68.0;
const TILE_GAP: f32 = 6.0;
const TILE_LABEL_GAP: f32 = 6.0;
const TILE_LABEL: f32 = 14.0;

/// A sign on a key, about the height of a key's digit.
const SYMBOL: f32 = 22.0;

const KEY_PRESSED: f32 = 0.92;
/// A tapped area (a number) shrinks only a little: that is all the feedback a copy gets.
const TAP_PRESSED: f32 = 0.95;

pub struct AppTile {
    pub title: String,
    pub icon: Icon,
}

pub struct AppsModel<'a> {
    pub tiles: &'a [AppTile],
    pub page: Option<PageModel<'a>>,
}

pub struct PageModel<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub node: &'a Node,
}

/// The glyph and the color of an app's tile, as the settings window colors its sections.
fn tile_look(icon: Icon) -> (Glyph, &'static str) {
    match icon {
        Icon::Calculator => (Glyph::Calculator, "#FF9F0A"),
        Icon::Siren => (Glyph::Siren, "#FF453A"),
    }
}

fn line(style: Style) -> f32 {
    match style {
        Style::Title => 20.0,
        Style::Body | Style::Caption => 18.0,
        Style::Display => 52.0,
    }
}

fn text_style(style: Style) -> TextStyle {
    match style {
        Style::Title => super::style(15.0, true, TEXT),
        Style::Body => super::style(13.0, false, TEXT),
        Style::Caption => super::style(13.0, false, MUTED),
        Style::Display => TextStyle {
            tabular: true,
            ..super::style(DISPLAY, false, TEXT)
        },
    }
}

/// A part with its place in the page body; a container's parts inside it.
#[derive(Debug, PartialEq)]
pub(super) struct Laid<'a> {
    node: &'a Node,
    rect: Rect,
    parts: Vec<Laid<'a>>,
}

fn shares(x: f32, w: f32, n: usize) -> impl Iterator<Item = (f32, f32)> {
    let n = n.max(1);
    let cell = (w - GAP * (n - 1) as f32) / n as f32;
    (0..n).map(move |i| (x + i as f32 * (cell + GAP), cell))
}

pub(super) fn lay(node: &Node, x: f32, y: f32, w: f32) -> Laid<'_> {
    let (parts, h) = match node {
        Node::Column(children) => {
            let mut parts = Vec::with_capacity(children.len());
            let mut at = y;
            for child in children {
                let part = lay(child, x, at, w);
                at += part.rect.h + GAP;
                parts.push(part);
            }
            let h = (at - y - GAP).max(0.0);
            (parts, h)
        }
        Node::Row(children) => {
            let parts: Vec<Laid> = children
                .iter()
                .zip(shares(x, w, children.len()))
                .map(|(child, (cx, cw))| lay(child, cx, y, cw))
                .collect();
            let h = parts.iter().map(|p| p.rect.h).fold(0.0, f32::max);
            (parts, h)
        }
        Node::Grid { columns, cells } => {
            let columns = usize::from(*columns).max(1);
            let mut parts = Vec::with_capacity(cells.len());
            let mut at = y;
            for row in cells.chunks(columns) {
                let laid: Vec<Laid> = row
                    .iter()
                    .zip(shares(x, w, columns))
                    .map(|(cell, (cx, cw))| lay(cell, cx, at, cw))
                    .collect();
                at += laid.iter().map(|p| p.rect.h).fold(0.0, f32::max) + GAP;
                parts.extend(laid);
            }
            (parts, (at - y - GAP).max(0.0))
        }
        Node::Tap { child, .. } => {
            let part = lay(child, x, y, w);
            let h = part.rect.h;
            (vec![part], h)
        }
        Node::Text { style, .. } => (Vec::new(), line(*style)),
        Node::Button { .. } => (Vec::new(), KEY),
        Node::Progress(_) => (Vec::new(), PROGRESS),
        Node::Spacer => (Vec::new(), GAP),
    };
    Laid {
        node,
        rect: Rect { x, y, w, h },
        parts,
    }
}

const BODY_TOP: f32 = PAD + BAR + BAR_GAP;

pub(super) fn page_size(node: &Node) -> (f32, f32) {
    let body = lay(node, PAD, BODY_TOP, menu::WIDTH - 2.0 * PAD);
    (menu::WIDTH, BODY_TOP + body.rect.h + PAD)
}

pub(super) fn row_size() -> (f32, f32) {
    (menu::WIDTH, 2.0 * PAD + TILE + TILE_LABEL_GAP + TILE_LABEL)
}

/// The tree without its texts: the same shape is the same view, updated in place.
pub(super) fn shape(node: &Node) -> String {
    match node {
        Node::Column(children) | Node::Row(children) => {
            let kind = if matches!(node, Node::Column(_)) {
                "C"
            } else {
                "R"
            };
            let inner: Vec<String> = children.iter().map(shape).collect();
            format!("{kind}[{}]", inner.join(","))
        }
        Node::Grid { columns, cells } => {
            let inner: Vec<String> = cells.iter().map(shape).collect();
            format!("G{columns}[{}]", inner.join(","))
        }
        Node::Text { style, align, .. } => format!("T{style:?}{align:?}"),
        Node::Button { face, action, tone } => format!("B{face:?}{}{tone:?}", action.0),
        Node::Tap { action, child } => format!("P{}({})", action.0, shape(child)),
        Node::Progress(value) => format!("%{value}"),
        Node::Spacer => "S".into(),
    }
}

struct PageText {
    text: Text,
    rect: Rect,
    align: Align,
    style: Style,
    /// The font size it is drawn at now: a display number may be smaller than its style's.
    size: f32,
}

pub(super) struct Parts {
    texts: Vec<PageText>,
    /// What shrinks under a press, and how much.
    presses: Vec<(Hit, Visual, f32)>,
}

fn key_colors(tone: Tone) -> (windows::UI::Color, windows::UI::Color, f32) {
    match tone {
        Tone::Plain => (color("#FFFFFF", 0.16), color("#FFFFFF", 0.22), 19.0),
        Tone::Muted => (color("#FFFFFF", 0.30), color("#FFFFFF", 0.36), 17.0),
        Tone::Accent => (color("#0A84FF", 1.0), color("#409CFF", 1.0), 19.0),
    }
}

impl Scene {
    pub fn page_height(&self, node: &Node) -> f32 {
        page_size(node).1
    }

    pub(super) fn apps_size(&self, model: &Model) -> (f32, f32) {
        match model.apps.as_ref().and_then(|a| a.page.as_ref()) {
            Some(page) => page_size(page.node),
            None => row_size(),
        }
    }

    pub(super) fn build_apps_row(&self, model: &Model, content: &mut Content) -> Result<()> {
        (content.width, content.height) = row_size();
        let mut presses = Vec::new();
        let tiles = model.apps.as_ref().map_or(&[][..], |a| a.tiles);
        for (i, tile) in tiles.iter().enumerate() {
            let x = PAD + i as f32 * (TILE_COLUMN + TILE_GAP);
            let (glyph, hex) = tile_look(tile.icon);
            let rect = Rect {
                x: x + (TILE_COLUMN - TILE) / 2.0,
                y: PAD,
                w: TILE,
                h: TILE,
            };
            let base = color(hex, 1.0);
            let square = self.rounded_cell(&content.root, rect, TILE_RADIUS, base)?;
            let icon = self
                .comp
                .glyph(glyph, TILE_ICON, color("#FFFFFF", 1.0), self.scale)?;
            icon.SetOffset(v3((TILE - TILE_ICON) / 2.0, (TILE - TILE_ICON) / 2.0))?;
            square.0.Children()?.InsertAtTop(&icon)?;
            let st = style(11.0, false, MUTED);
            let label = self.comp.fit(&tile.title, st, TILE_COLUMN);
            let width = self.comp.measure(&label, st);
            self.label(
                &content.root,
                &label,
                st,
                x + (TILE_COLUMN - width) / 2.0,
                PAD + TILE + TILE_LABEL_GAP,
            )?;
            let hit = Hit::Launch(i);
            content.buttons.push(Button {
                fill: square.1,
                rect,
                hit: hit.clone(),
                base,
                hover: color(hex, 0.82),
            });
            presses.push((hit.clone(), square.0.cast()?, KEY_PRESSED));
            content.hits.push((
                Rect {
                    x,
                    y: PAD,
                    w: TILE_COLUMN,
                    h: TILE + TILE_LABEL_GAP + TILE_LABEL,
                },
                hit,
            ));
        }
        content.app = Some(Parts {
            texts: Vec::new(),
            presses,
        });
        Ok(())
    }

    pub(super) fn build_app_page(&self, model: &Model, content: &mut Content) -> Result<()> {
        let Some(page) = model.apps.as_ref().and_then(|a| a.page.as_ref()) else {
            return Ok(());
        };
        (content.width, content.height) = page_size(page.node);
        let root = content.root.clone();
        let mut parts = Parts {
            texts: Vec::new(),
            presses: Vec::new(),
        };

        let back = self.rounded_cell(
            &root,
            Rect {
                x: PAD,
                y: PAD,
                w: BAR,
                h: BAR,
            },
            BAR / 2.0,
            color("#FFFFFF", 0.12),
        )?;
        let arrow = self
            .comp
            .glyph(Glyph::Back, 14.0, color(TEXT, 1.0), self.scale)?;
        arrow.SetOffset(v3((BAR - 14.0) / 2.0, (BAR - 14.0) / 2.0))?;
        back.0.Children()?.InsertAtTop(&arrow)?;
        parts
            .presses
            .push((Hit::AppBack, back.0.cast()?, KEY_PRESSED));
        let st = style(13.0, false, MUTED);
        let title = self.comp.text(page.title, st, self.scale)?;
        self.place(
            &root,
            &title.visual,
            PAD + BAR + 8.0,
            PAD + self.comp.center_top(&title, BAR),
        )?;
        content.hits.push((
            Rect {
                x: PAD - 6.0,
                y: PAD - 6.0,
                w: BAR + 12.0,
                h: BAR + 12.0,
            },
            Hit::AppBack,
        ));

        let body = lay(page.node, PAD, BODY_TOP, menu::WIDTH - 2.0 * PAD);
        self.draw_laid(&body, &root, (0.0, 0.0), content, &mut parts)?;
        content.app = Some(parts);
        Ok(())
    }

    /// `origin` is where `parent` sits in the page: a tapped area is a container of its own, so
    /// that it shrinks as a whole.
    fn draw_laid(
        &self,
        laid: &Laid,
        parent: &ContainerVisual,
        origin: (f32, f32),
        content: &mut Content,
        parts: &mut Parts,
    ) -> Result<()> {
        let r = laid.rect;
        let local = (r.x - origin.0, r.y - origin.1);
        match laid.node {
            Node::Column(_) | Node::Row(_) | Node::Grid { .. } => {
                for part in &laid.parts {
                    self.draw_laid(part, parent, origin, content, parts)?;
                }
            }
            Node::Tap { action, .. } => {
                let area = self.comp.container()?;
                area.SetSize(v2(r.w, r.h))?;
                area.SetCenterPoint(v3(r.w / 2.0, r.h / 2.0))?;
                self.place(parent, &area, local.0, local.1)?;
                for part in &laid.parts {
                    self.draw_laid(part, &area, (r.x, r.y), content, parts)?;
                }
                let hit = Hit::App(*action);
                parts.presses.push((hit.clone(), area.cast()?, TAP_PRESSED));
                // The area's own parts (a button inside it) are found first.
                content.hits.push((r, hit));
            }
            Node::Text { text, style, align } => {
                let (shown, size) = self.fit_text(text, *style, r.w);
                let st = TextStyle {
                    size,
                    ..text_style(*style)
                };
                let visual = self.comp.text(&shown, st, self.scale)?;
                let x = aligned(local.0, r.w, visual.width, *align);
                self.place(
                    parent,
                    &visual.visual,
                    x,
                    local.1 + self.comp.center_top(&visual, r.h),
                )?;
                parts.texts.push(PageText {
                    text: visual,
                    rect: Rect {
                        x: local.0,
                        y: local.1,
                        ..r
                    },
                    align: *align,
                    style: *style,
                    size,
                });
            }
            Node::Button { face, action, tone } => {
                let (base, hover, size) = key_colors(*tone);
                let cell = self.rounded_cell(
                    parent,
                    Rect {
                        x: local.0,
                        y: local.1,
                        ..r
                    },
                    KEY_RADIUS,
                    base,
                )?;
                let (visual, w, h) = match face {
                    Face::Label(label) => {
                        let text = self
                            .comp
                            .text(label, style(size, false, TEXT), self.scale)?;
                        (text.visual, text.width, text.height)
                    }
                    Face::Symbol(symbol) => {
                        let glyph = self.comp.glyph(
                            super::live::glyph(*symbol),
                            SYMBOL,
                            color(TEXT, 1.0),
                            self.scale,
                        )?;
                        (glyph, SYMBOL, SYMBOL)
                    }
                };
                visual.SetOffset(v3(
                    super::snap((r.w - w) / 2.0, self.scale),
                    super::snap((r.h - h) / 2.0, self.scale),
                ))?;
                cell.0.Children()?.InsertAtTop(&visual)?;
                let hit = Hit::App(*action);
                content.buttons.push(Button {
                    fill: cell.1,
                    rect: r,
                    hit: hit.clone(),
                    base,
                    hover,
                });
                parts
                    .presses
                    .push((hit.clone(), cell.0.cast()?, KEY_PRESSED));
                content.hits.insert(0, (r, hit));
            }
            Node::Progress(value) => {
                let track = Rect {
                    x: local.0,
                    y: local.1,
                    ..r
                };
                self.rounded_cell(parent, track, r.h / 2.0, color("#FFFFFF", 0.2))?;
                let filled = Rect {
                    w: (r.w * value.clamp(0.0, 1.0)).max(r.h),
                    ..track
                };
                self.rounded_cell(parent, filled, r.h / 2.0, color(TEXT, 1.0))?;
            }
            Node::Spacer => {}
        }
        Ok(())
    }

    /// A rounded rectangle in a container that scales around its middle; the fill is returned
    /// for the hover.
    fn rounded_cell(
        &self,
        parent: &ContainerVisual,
        rect: Rect,
        radius: f32,
        fill: windows::UI::Color,
    ) -> Result<(
        ContainerVisual,
        windows::UI::Composition::CompositionColorBrush,
    )> {
        let c = &self.comp.compositor;
        let cell = c.CreateContainerVisual()?;
        cell.SetSize(v2(rect.w, rect.h))?;
        cell.SetCenterPoint(v3(rect.w / 2.0, rect.h / 2.0))?;
        let shape = c.CreateShapeVisual()?;
        shape.SetSize(v2(rect.w, rect.h))?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetSize(v2(rect.w, rect.h))?;
        geometry.SetCornerRadius(v2(radius, radius))?;
        let brush = self.comp.brush(fill)?;
        let sprite = c.CreateSpriteShapeWithGeometry(&geometry)?;
        sprite.SetFillBrush(&brush)?;
        shape.Shapes()?.Append(&sprite)?;
        cell.Children()?.InsertAtTop(&shape)?;
        self.place(parent, &cell, rect.x, rect.y)?;
        Ok((cell, brush))
    }

    /// A display number shrinks to fit its line; any text that still doesn't fit loses its end.
    fn fit_text(&self, text: &str, style: Style, width: f32) -> (String, f32) {
        let st = text_style(style);
        let mut size = st.size;
        if style == Style::Display {
            let natural = self.comp.measure(text, st);
            if natural > width {
                size = (st.size * width / natural).floor().max(DISPLAY_MIN);
            }
        }
        let st = TextStyle { size, ..st };
        (self.comp.fit(text, st, width), size)
    }

    pub(super) fn update_app_page(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(parts), Some(page)) = (
            &mut content.app,
            model.apps.as_ref().and_then(|a| a.page.as_ref()),
        ) else {
            return Ok(());
        };
        let mut texts = Vec::new();
        collect_texts(page.node, &mut texts);
        for (shown, &text) in parts.texts.iter_mut().zip(&texts) {
            let (fitted, size) = self.fit_text(text, shown.style, shown.rect.w);
            if size != shown.size {
                self.comp.set_text_size(&mut shown.text, size, self.scale)?;
                shown.size = size;
            }
            self.comp.set_text(&mut shown.text, &fitted, self.scale)?;
            let x = aligned(shown.rect.x, shown.rect.w, shown.text.width, shown.align);
            shown.text.visual.SetOffset(v3(
                super::snap(x, self.scale),
                super::snap(
                    shown.rect.y + self.comp.center_top(&shown.text, shown.rect.h),
                    self.scale,
                ),
            ))?;
        }
        Ok(())
    }

    pub(super) fn press_app(
        &self,
        parts: &Parts,
        was: Option<&Hit>,
        now: Option<&Hit>,
    ) -> Result<()> {
        for (hit, visual, k) in &parts.presses {
            let (before, after) = (was == Some(hit), now == Some(hit));
            if before == after {
                continue;
            }
            let k = if after { *k } else { 1.0 };
            self.comp
                .animate_vector3(visual, "Scale", scale(k, k), PRESS)?;
        }
        Ok(())
    }
}

const PRESS: Motion = LATCH;

fn aligned(x: f32, w: f32, width: f32, align: Align) -> f32 {
    match align {
        Align::Start => x,
        Align::Center => x + (w - width) / 2.0,
        Align::End => x + w - width,
    }
}

fn collect_texts<'a>(node: &'a Node, out: &mut Vec<&'a str>) {
    match node {
        Node::Column(children)
        | Node::Row(children)
        | Node::Grid {
            cells: children, ..
        } => children.iter().for_each(|c| collect_texts(c, out)),
        Node::Tap { child, .. } => collect_texts(child, out),
        Node::Text { text, .. } => out.push(text),
        Node::Button { .. } | Node::Progress(_) | Node::Spacer => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::ui::Action;

    fn key(n: u16) -> Node {
        Node::button(n.to_string(), Action(n), Tone::Plain)
    }

    #[test]
    fn a_grid_shares_its_width_and_stacks_its_rows() {
        let grid = Node::Grid {
            columns: 4,
            cells: (0..6).map(key).collect(),
        };
        let laid = lay(&grid, 14.0, 0.0, 290.0);
        assert_eq!(laid.parts.len(), 6);
        assert_eq!(
            laid.parts[0].rect,
            Rect {
                x: 14.0,
                y: 0.0,
                w: 66.5,
                h: KEY
            }
        );
        assert_eq!(laid.parts[3].rect.x, 14.0 + 3.0 * (66.5 + GAP));
        assert_eq!(laid.parts[4].rect.y, KEY + GAP, "the second row");
        assert_eq!(laid.rect.h, 2.0 * KEY + GAP);
    }

    #[test]
    fn a_column_stacks_its_parts_by_their_heights() {
        let column = Node::Column(vec![
            Node::text("1 +", Style::Caption, Align::End),
            Node::text("2", Style::Display, Align::End),
            Node::Progress(0.5),
        ]);
        let laid = lay(&column, 0.0, 10.0, 100.0);
        let ys: Vec<f32> = laid.parts.iter().map(|p| p.rect.y).collect();
        assert_eq!(
            ys,
            [10.0, 10.0 + 18.0 + GAP, 10.0 + 18.0 + 52.0 + 2.0 * GAP]
        );
        assert_eq!(laid.rect.h, 18.0 + 52.0 + PROGRESS + 2.0 * GAP);
    }

    #[test]
    fn the_calculator_fits_under_its_title() {
        crate::lang::with(crate::lang::Lang::Ru, || {
            use crate::apps::{Applet, Context, calculator::Calculator, prefs::Values};
            let values = Values::default();
            let page = Calculator::default().page(&Context::bare(&values));
            let (w, h) = page_size(&page);
            assert_eq!(w, menu::WIDTH);
            // The title, the two lines over the keys and five rows of keys.
            let body = 18.0 + 52.0 + GAP + 5.0 * KEY + 4.0 * GAP + GAP;
            assert_eq!(h, BODY_TOP + body + PAD);
        });
    }

    #[test]
    fn texts_are_not_the_shape_but_keys_are() {
        let page = |number: &str, key: &str| {
            Node::Column(vec![
                Node::text(number, Style::Display, Align::End),
                Node::button(key, Action(1), Tone::Plain),
            ])
        };
        assert_eq!(shape(&page("1", "=")), shape(&page("12", "=")));
        assert_ne!(shape(&page("1", "=")), shape(&page("1", "+")));
    }
}
