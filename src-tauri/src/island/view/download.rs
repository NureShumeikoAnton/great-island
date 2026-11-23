//! Downloads on the island (`crate::download`), in the island's standard sizes:
//! - **coming** (`Mode::Downloading`), small: a spinner, the file's name or how many files,
//!   and how much has come so far, which changes in place;
//! - **arrived** (`Mode::Download`), small: the arrow dropping into the tray and the file's
//!   name and size; several files show their pictures in a stack, how many and the total;
//! - **the card**, under the cursor: one file's picture (a click opens it) or a pile of the
//!   last three, the name or the count, the size and the site; a click on the text shows the
//!   files in Explorer.
//!
//! A file without a picture of its own gets the placeholder (`file_icon.rs`).

use super::super::comp::{Motion, Text, TextStyle, color, v2, v3};
use super::super::glyph::Glyph;
use super::system::{EASE_OUT, LINEAR};
use super::{Content, Hit, IconButton, MUTED, Model, Rect, Scene, TEXT, format_size, snap, style};
use crate::{
    download::Progress,
    lang::{self, Count, tr},
    media::Icon,
};
use windows::{
    UI::Composition::{ContainerVisual, Visual},
    core::{Interface, Result},
};

pub struct DownloadModel<'a> {
    pub coming: Option<&'a Progress>,
    /// Newest last.
    pub files: Vec<FileModel<'a>>,
    pub decor: bool,
}

pub struct FileModel<'a> {
    pub name: &'a str,
    pub bytes: u64,
    pub source: Option<&'a str>,
    pub picture: Option<&'a Icon>,
}

const BLUE: &str = "#0A84FF";
const TILE: &str = "#1C1C1F";

const SMALL_LEFT: f32 = 12.0;
const SMALL_GAP: f32 = 8.0;
const SMALL_RIGHT: f32 = 14.0;
const GLYPH: f32 = 18.0;
const SPINNER: f32 = 14.0;
const SPIN_MS: u32 = 900;
const NAME_MAX: f32 = 190.0;
const MINI: f32 = 18.0;
/// Each next picture of the stack covers this much of the one before.
const MINI_OVERLAP: f32 = 6.0;
const STACK: usize = 3;
/// The black ring that parts the pictures of a stack or a pile.
const RING: f32 = 1.5;

const HEIGHT: f32 = 64.0;
const PAD: f32 = 12.0;
const THUMB_H: f32 = 40.0;
const TILE_W: f32 = 64.0;
const RADIUS: f32 = 6.0;
const PLACEHOLDER: f32 = 30.0;
const TEXT_GAP: f32 = 12.0;
const RIGHT: f32 = 18.0;
const MIN_WIDTH: f32 = 240.0;
const MAX_WIDTH: f32 = 380.0;
/// How the lower sheets of the pile lie: turned and shifted, from the top one down.
const PILE: [(f32, f32); 2] = [(-7.0, -5.0), (7.0, 5.0)];

pub(super) struct Parts {
    size: Text,
    size_x: f32,
}

fn files_label(n: usize) -> String {
    match lang::count(n as u64) {
        Count::One => tr!("download.files_one", n = n),
        Count::Few => tr!("download.files_few", n = n),
        Count::Many => tr!("download.files_many", n = n),
    }
    .into_owned()
}

fn total(files: &[FileModel]) -> u64 {
    files.iter().map(|f| f.bytes).sum()
}

fn dim(text: &str) -> String {
    format!(" {text}")
}

fn picture_width(picture: Option<&Icon>) -> f32 {
    match picture {
        Some(p) => {
            let aspect = p.width as f32 / p.height.max(1) as f32;
            (THUMB_H * aspect)
                .clamp(THUMB_H * 0.6, THUMB_H * 2.4)
                .round()
        }
        None => TILE_W,
    }
}

impl Scene {
    /// The middle of a long name goes, its extension stays.
    fn fit_name(&self, name: &str, st: TextStyle, max: f32) -> String {
        if self.comp.measure(name, st) <= max {
            return name.to_string();
        }
        let chars: Vec<char> = name.chars().collect();
        let dot = chars.iter().rposition(|&c| c == '.').unwrap_or(chars.len());
        let tail: String = chars[dot.saturating_sub(5).min(chars.len())..]
            .iter()
            .collect();
        for keep in (1..dot.saturating_sub(5)).rev() {
            let head: String = chars[..keep].iter().collect();
            let candidate = format!("{}…{tail}", head.trim_end());
            if self.comp.measure(&candidate, st) <= max {
                return candidate;
            }
        }
        self.comp.fit(name, st, max)
    }

    fn coming_label(&self, coming: &Progress) -> String {
        match (&coming.name, coming.files) {
            (Some(name), 1) => self.fit_name(name, style(13.0, true, TEXT), NAME_MAX),
            (_, n) => files_label(n),
        }
    }

    fn arrived_label(&self, files: &[FileModel]) -> (String, u64) {
        match files {
            [one] => (
                self.fit_name(one.name, style(13.0, true, TEXT), NAME_MAX),
                one.bytes,
            ),
            _ => (files_label(files.len()), total(files)),
        }
    }

    fn small_icon_width(files: &[FileModel]) -> f32 {
        match files.len() {
            0 | 1 => GLYPH,
            n => {
                let shown = n.min(STACK) as f32;
                MINI + (shown - 1.0) * (MINI - MINI_OVERLAP)
            }
        }
    }

    pub(super) fn downloading_size(&self, model: &Model) -> (f32, f32) {
        let Some(coming) = model.download.as_ref().and_then(|d| d.coming) else {
            return (30.0, 30.0);
        };
        let label = self
            .comp
            .measure(&self.coming_label(coming), style(13.0, true, TEXT));
        let size = self
            .comp
            .measure(&dim(&format_size(coming.bytes)), digits(13.0, false, MUTED));
        let width = SMALL_LEFT + SPINNER + SMALL_GAP + label + size + SMALL_RIGHT;
        (width.round(), 30.0)
    }

    pub(super) fn download_size(&self, model: &Model, open: bool) -> (f32, f32) {
        let Some(download) = model.download.as_ref().filter(|d| !d.files.is_empty()) else {
            return (30.0, 30.0);
        };
        if open {
            return (self.download_card_width(&download.files), HEIGHT);
        }
        let (label, bytes) = self.arrived_label(&download.files);
        let width = SMALL_LEFT
            + Self::small_icon_width(&download.files)
            + SMALL_GAP
            + self.comp.measure(&label, style(13.0, true, TEXT))
            + self
                .comp
                .measure(&dim(&format_size(bytes)), digits(13.0, false, MUTED))
            + SMALL_RIGHT;
        (width.round(), 30.0)
    }

    fn card_texts(files: &[FileModel]) -> (String, String) {
        match files {
            [one] => (
                one.name.to_string(),
                match one.source {
                    Some(site) => format!("{} · {site}", format_size(one.bytes)),
                    None => format_size(one.bytes),
                },
            ),
            _ => (files_label(files.len()), format_size(total(files))),
        }
    }

    fn card_thumb_width(files: &[FileModel]) -> f32 {
        match files {
            [one] => picture_width(one.picture),
            _ => TILE_W,
        }
    }

    fn download_card_width(&self, files: &[FileModel]) -> f32 {
        let (title, sub) = Self::card_texts(files);
        let text = self
            .comp
            .measure(&title, style(14.0, true, TEXT))
            .max(self.comp.measure(&sub, style(12.0, false, MUTED)));
        let width = PAD + Self::card_thumb_width(files) + TEXT_GAP + text + RIGHT;
        width.clamp(MIN_WIDTH, MAX_WIDTH).round()
    }

    pub(super) fn build_downloading(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (width, height) = self.downloading_size(model);
        content.width = width;
        content.height = height;
        let Some(coming) = model.download.as_ref().and_then(|d| d.coming) else {
            return Ok(());
        };
        // The size grows to the right in place: the view keeps to the capsule's left edge.
        content.anchor_left = true;
        let root = content.root.clone();
        // A quarter of a ring turning on the compositor: nothing is drawn per frame by us.
        let ring = self.ring(BLUE, SPINNER, 2.0)?;
        ring.arc.SetTrimEnd(0.27)?;
        self.place(&root, &ring.visual, SMALL_LEFT, (30.0 - SPINNER) / 2.0)?;
        self.comp.loop_scalar(
            &ring.visual,
            "RotationAngleInDegrees",
            &[
                (0.0, 0.0, Motion::linear(SPIN_MS)),
                (1.0, 360.0, Motion::linear(SPIN_MS)),
            ],
            SPIN_MS,
        )?;
        let x = SMALL_LEFT + SPINNER + SMALL_GAP;
        let label = self.comp.text(
            &self.coming_label(coming),
            style(13.0, true, TEXT),
            self.scale,
        )?;
        self.place(&root, &label.visual, x, self.comp.center_top(&label, 30.0))?;
        let size_x = x + label.width;
        let size = self.comp.text(
            &dim(&format_size(coming.bytes)),
            digits(13.0, false, MUTED),
            self.scale,
        )?;
        self.place(
            &root,
            &size.visual,
            size_x,
            self.comp.center_top(&size, 30.0),
        )?;
        content.download = Some(Parts { size, size_x });
        Ok(())
    }

    pub(super) fn update_downloading(&self, model: &Model, content: &mut Content) -> Result<()> {
        let (Some(coming), Some(parts)) = (
            model.download.as_ref().and_then(|d| d.coming),
            &mut content.download,
        ) else {
            return Ok(());
        };
        self.comp.set_text(
            &mut parts.size,
            &dim(&format_size(coming.bytes)),
            self.scale,
        )?;
        content.width = (parts.size_x + parts.size.width + SMALL_RIGHT).round();
        Ok(())
    }

    pub(super) fn build_download(
        &self,
        model: &Model,
        open: bool,
        content: &mut Content,
    ) -> Result<()> {
        let (width, height) = self.download_size(model, open);
        content.width = width;
        content.height = height;
        let Some(download) = model.download.as_ref().filter(|d| !d.files.is_empty()) else {
            return Ok(());
        };
        if open {
            content.anchor_left = true;
            self.build_download_card(download, content)
        } else {
            self.build_download_small(download, content)
        }
    }

    fn build_download_small(&self, download: &DownloadModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let files = &download.files;
        if let [_] = files.as_slice() {
            self.download_glyph(&root, download.decor)?;
        } else {
            let shown = &files[files.len().saturating_sub(STACK)..];
            let mut x = SMALL_LEFT;
            for file in shown {
                let mini = self.file_tile(file, (MINI, MINI), 4.0, true)?;
                self.place(&root, &mini, x - RING, (30.0 - MINI) / 2.0 - RING)?;
                x += MINI - MINI_OVERLAP;
            }
        }
        let (label, bytes) = self.arrived_label(files);
        let x = SMALL_LEFT + Self::small_icon_width(files) + SMALL_GAP;
        let label = self
            .comp
            .text(&label, style(13.0, true, TEXT), self.scale)?;
        self.place(&root, &label.visual, x, self.comp.center_top(&label, 30.0))?;
        let size = self.comp.text(
            &dim(&format_size(bytes)),
            digits(13.0, false, MUTED),
            self.scale,
        )?;
        self.place(
            &root,
            &size.visual,
            x + label.width,
            self.comp.center_top(&size, 30.0),
        )
    }

    fn download_glyph(&self, root: &ContainerVisual, decor: bool) -> Result<()> {
        let y = (30.0 - GLYPH) / 2.0;
        let tray = self
            .comp
            .glyph(Glyph::DownloadTray, GLYPH, color(BLUE, 1.0), self.scale)?;
        self.place(root, &tray, SMALL_LEFT, y)?;
        let arrow = self
            .comp
            .glyph(Glyph::DownloadArrow, GLYPH, color(BLUE, 1.0), self.scale)?;
        self.place(root, &arrow, SMALL_LEFT, y)?;
        if decor {
            let y = snap(y, self.scale);
            let drop = [(0.0, y - 7.0, LINEAR), (1.0, y, EASE_OUT)];
            self.comp
                .scalar_keys(&arrow, "Offset.Y", &drop, 620, 160, Some(1))?;
            let fade = [(0.0, 0.0, LINEAR), (0.6, 1.0, LINEAR), (1.0, 1.0, LINEAR)];
            self.comp
                .scalar_keys(&arrow, "Opacity", &fade, 620, 160, Some(1))?;
        }
        Ok(())
    }

    /// A file's picture or its placeholder on a tile, `size` big. `ringed`: on a black ring
    /// that parts it from the one it covers.
    fn file_tile(
        &self,
        file: &FileModel,
        size: (f32, f32),
        radius: f32,
        ringed: bool,
    ) -> Result<ContainerVisual> {
        let holder = self.comp.container()?;
        let inset = if ringed { RING } else { 0.0 };
        holder.SetSize(v2(size.0 + 2.0 * inset, size.1 + 2.0 * inset))?;
        if ringed {
            let ring = self.rounded_rect(
                size.0 + 2.0 * RING,
                size.1 + 2.0 * RING,
                radius + RING,
                color("#000000", 1.0),
            )?;
            holder.Children()?.InsertAtTop(&ring)?;
        }
        match file.picture {
            Some(picture) => {
                let image = self.file_picture(picture, size, radius)?;
                self.place(&holder, &image, inset, inset)?;
            }
            None => {
                let tile = self.rounded_rect(size.0, size.1, radius, color(TILE, 1.0))?;
                self.place(&holder, &tile, inset, inset)?;
                let side = (size.1 * PLACEHOLDER / THUMB_H).round();
                let icon = self.file_icon(file.name, side)?;
                self.place(
                    &holder,
                    &icon,
                    inset + (size.0 - side) / 2.0,
                    inset + (size.1 - side) / 2.0,
                )?;
            }
        }
        Ok(holder)
    }

    fn build_download_card(&self, download: &DownloadModel, content: &mut Content) -> Result<()> {
        let root = content.root.clone();
        let files = &download.files;
        let thumb = Rect {
            x: PAD,
            y: (HEIGHT - THUMB_H) / 2.0,
            w: Self::card_thumb_width(files),
            h: THUMB_H,
        };
        let (picture, hit): (Visual, Hit) = match files.as_slice() {
            [one] => {
                let tile = self.file_tile(one, (thumb.w, thumb.h), RADIUS, false)?;
                (tile.cast()?, Hit::DownloadOpen)
            }
            _ => (self.pile(files, thumb)?.cast()?, Hit::DownloadFolder),
        };
        picture.SetCenterPoint(v3(thumb.w / 2.0, thumb.h / 2.0))?;
        self.place(&root, &picture, thumb.x, thumb.y)?;
        content.icon_buttons.push(IconButton {
            visual: picture,
            hit: hit.clone(),
        });
        content.hits.push((thumb, hit));

        let (title, sub) = Self::card_texts(files);
        let x = thumb.x + thumb.w + TEXT_GAP;
        let room = content.width - x - RIGHT;
        let title_style = style(14.0, true, TEXT);
        let sub_style = style(12.0, false, MUTED);
        let title = match files.as_slice() {
            [_] => self.fit_name(&title, title_style, room),
            _ => title,
        };
        let title = self.comp.text(&title, title_style, self.scale)?;
        let sub = self
            .comp
            .text(&self.comp.fit(&sub, sub_style, room), sub_style, self.scale)?;
        let text_rect = Rect {
            x,
            y: 0.0,
            w: content.width - x - RIGHT / 2.0,
            h: HEIGHT,
        };
        let label = self.comp.container()?;
        label.SetSize(v2(text_rect.w, text_rect.h))?;
        label.SetCenterPoint(v3(text_rect.w / 2.0, text_rect.h / 2.0))?;
        let top = snap(self.comp.stack_top(&title, &sub, 2.0, HEIGHT), self.scale);
        self.place(&label, &title.visual, 0.0, top)?;
        self.place(&label, &sub.visual, 0.0, top + title.height + 2.0)?;
        self.place(&root, &label, text_rect.x, text_rect.y)?;
        content.icon_buttons.push(IconButton {
            visual: label.cast()?,
            hit: Hit::DownloadFolder,
        });
        content.hits.push((text_rect, Hit::DownloadFolder));
        Ok(())
    }

    /// The last three files as a pile of sheets, the newest on top.
    fn pile(&self, files: &[FileModel], thumb: Rect) -> Result<ContainerVisual> {
        let pile = self.comp.container()?;
        pile.SetSize(v2(thumb.w, thumb.h))?;
        let shown = &files[files.len().saturating_sub(STACK)..];
        for (depth, file) in shown.iter().rev().enumerate().rev() {
            let tile = self.file_tile(file, (thumb.w, thumb.h), RADIUS, true)?;
            tile.SetCenterPoint(v3(thumb.w / 2.0 + RING, thumb.h / 2.0 + RING))?;
            let shift = match depth.checked_sub(1).and_then(|i| PILE.get(i)) {
                Some(&(angle, x)) => {
                    tile.SetRotationAngleInDegrees(angle)?;
                    x
                }
                None => 0.0,
            };
            self.place(&pile, &tile, shift - RING, -RING)?;
        }
        Ok(pile)
    }
}

fn digits(size: f32, bold: bool, hex: &str) -> TextStyle {
    TextStyle {
        tabular: true,
        ..style(size, bold, hex)
    }
}
