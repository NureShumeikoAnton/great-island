//! Viewer panel: on the left Show in folder, Copy and Edit; on the right a reddish Delete; the file name in
//! the middle (a click copies the path). Without the strip (a setting) there is no name and the capsules
//! lie right on the shot.

use super::super::comp::{Comp, Motion, Text, TextStyle, color};
use super::super::glyph::Glyph;
use super::super::log;
use super::capsule::{Capsule, Face, Panel};
use super::{CHECK, MUTED, Metrics, RED_ICON, TEXT};
use crate::lang::tr;
use std::borrow::Cow;
use windows::{
    UI::Composition::Visual,
    core::{Interface, Result},
};

const FADE: Motion = Motion::curve(200, 0.25, 0.1, 0.25, 1.0);
const HOVER: Motion = Motion::curve(150, 0.25, 0.1, 0.25, 1.0);
const NAME_HOVER: f32 = 0.7;
const NAME_PAD: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Folder,
    Copy,
    Edit,
    Delete,
    Name,
}

const BUTTONS: [(Item, Glyph); 4] = [
    (Item::Folder, Glyph::Folder),
    (Item::Copy, Glyph::Copy),
    (Item::Edit, Glyph::Pencil),
    (Item::Delete, Glyph::Trash),
];

pub struct Bar {
    pub panel: Panel<Item>,
    copied: Visual,
    title: Text,
    /// File name; `None` = no file (the shot is only in the clipboard) or it is still being written.
    file: Option<String>,
    /// The file is still being written: no name yet, but no "no file" either.
    saving: bool,
    path_copied: bool,
    name_rect: (f32, f32, f32, f32),
    name_hovered: bool,
    name_pressed: bool,
    m: Metrics,
    k: f32,
    width: f32,
}

impl Bar {
    pub fn new(
        comp: &Comp,
        k: f32,
        m: Metrics,
        file: Option<String>,
        saving: bool,
    ) -> Result<Self> {
        let mut panel = Panel::new(comp)?;
        let mut copied = None;
        for (item, glyph) in BUTTONS {
            let (look, ink) = if item == Item::Delete {
                (m.danger(), color(RED_ICON, 1.0))
            } else {
                (m.plain(), color(TEXT, 1.0))
            };
            let capsule = Capsule::new(comp, k, m.button, Face::Glyph(glyph, m.icon, ink), look)?;
            if item == Item::Copy {
                let check = comp.glyph(Glyph::Check, m.icon, color(CHECK, 1.0), k)?;
                check.SetOffset(capsule.face.Offset()?)?;
                check.SetOpacity(0.0)?;
                capsule.body.Children()?.InsertAtTop(&check)?;
                copied = Some(check.cast()?);
            }
            panel.add(item, capsule)?;
        }
        let title = comp.text("", name_style(m, TEXT), k)?;
        panel.root.Children()?.InsertAtTop(&title.visual)?;
        let mut bar = Self {
            panel,
            copied: copied.ok_or_else(windows::core::Error::empty)?,
            title,
            file,
            saving,
            path_copied: false,
            name_rect: (0.0, 0.0, 0.0, 0.0),
            name_hovered: false,
            name_pressed: false,
            m,
            k,
            width: 0.0,
        };
        bar.sync_buttons();
        Ok(bar)
    }

    pub fn layout(&mut self, comp: &Comp, width: f32) -> Result<()> {
        self.width = width;
        let m = self.m;
        let (w, _) = m.button;
        let y = m.button_top();
        let mut left = m.margin;
        for (item, capsule) in self.panel.items_mut() {
            if item == Item::Delete {
                capsule.place(width - m.margin - w, y);
            } else {
                capsule.place(left, y);
                left += w + m.gap;
            }
        }
        self.draw_title(comp)
    }

    /// The video's Edit is pale and does nothing until the video editor exists.
    pub fn set_editable(&mut self, on: bool) {
        if let Some(edit) = self.panel.get_mut(Item::Edit) {
            edit.set_enabled(on);
        }
    }

    pub fn file(&self) -> Option<&String> {
        self.file.as_ref()
    }

    pub fn set_file(&mut self, comp: &Comp, file: Option<String>) -> Result<()> {
        self.saving = false;
        self.file = file;
        self.path_copied = false;
        self.sync_buttons();
        self.draw_title(comp)
    }

    fn sync_buttons(&mut self) {
        let has_file = self.file.is_some() || self.saving;
        if let Some(folder) = self.panel.get_mut(Item::Folder) {
            folder.set_enabled(has_file);
        }
    }

    pub fn show_copied(&self, comp: &Comp, on: bool) {
        let Some(copy) = self.panel.get(Item::Copy) else {
            return;
        };
        let (icon, check) = if on { (0.0, 1.0) } else { (1.0, 0.0) };
        log(comp.animate_scalar(&copy.face, "Opacity", icon, FADE));
        log(comp.animate_scalar(&self.copied, "Opacity", check, FADE));
    }

    pub fn show_path_copied(&mut self, comp: &Comp, on: bool) -> Result<()> {
        self.path_copied = on;
        self.draw_title(comp)
    }

    fn draw_title(&mut self, comp: &Comp) -> Result<()> {
        let (m, k) = (self.m, self.k);
        let left_end = m.margin + 3.0 * (m.button.0 + m.gap);
        let right_start = self.width - m.margin - m.button.0 - m.gap;
        let center = self.width / 2.0;
        let room = 2.0 * ((center - left_end).min(right_start - center) - m.gap);
        let (content, tint): (Cow<str>, _) = match (&self.file, self.path_copied) {
            // Without the strip there is no name: it is remembered but not shown.
            _ if m.floating() => ("".into(), TEXT),
            (_, true) => (tr!("viewer.path_copied"), CHECK),
            (Some(name), false) => (name.as_str().into(), TEXT),
            (None, false) if self.saving => ("".into(), TEXT),
            (None, false) => (tr!("viewer.no_file"), MUTED),
        };
        let style = name_style(m, tint);
        let fitted = comp.fit(&content, style, room.max(0.0));
        comp.set_text_color(&mut self.title, style.color, k)?;
        comp.set_text(&mut self.title, &fitted, k)?;
        let w = self.title.width;
        let x = center - w / 2.0;
        let y = m.strip / 2.0 - comp.cap_middle(style);
        self.title.visual.SetOffset(super::super::comp::v3(
            (x * k).round() / k,
            (y * k).round() / k,
        ))?;
        self.name_rect = if self.file.is_some() && !m.floating() {
            (x - NAME_PAD, 0.0, w + 2.0 * NAME_PAD, m.strip)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        Ok(())
    }

    fn on_name(&self, (x, y): (f32, f32)) -> bool {
        let (left, top, w, h) = self.name_rect;
        x >= left && x <= left + w && y >= top && y <= top + h
    }

    pub fn at(&self, p: (f32, f32)) -> Option<Item> {
        self.panel
            .at(p)
            .or_else(|| self.on_name(p).then_some(Item::Name))
    }

    pub fn enabled_at(&self, p: (f32, f32)) -> Option<Item> {
        self.panel
            .enabled_at(p)
            .or_else(|| self.on_name(p).then_some(Item::Name))
    }

    pub fn hover(&mut self, comp: &Comp, p: Option<(f32, f32)>) {
        self.panel.hover(comp, p);
        let on = p.is_some_and(|p| self.on_name(p));
        if on != self.name_hovered {
            self.name_hovered = on;
            let opacity = if on { NAME_HOVER } else { 1.0 };
            log(comp.animate_scalar(&self.title.visual, "Opacity", opacity, HOVER));
        }
    }

    pub fn press(&mut self, comp: &Comp, p: (f32, f32)) -> bool {
        if self.panel.press(comp, p) {
            return true;
        }
        self.name_pressed = self.on_name(p);
        self.name_pressed
    }

    pub fn release(&mut self, comp: &Comp, p: (f32, f32)) -> Option<Item> {
        if std::mem::take(&mut self.name_pressed) {
            return self.on_name(p).then_some(Item::Name);
        }
        self.panel.release(comp, p)
    }

    pub fn cancel(&mut self, comp: &Comp) {
        self.name_pressed = false;
        self.panel.cancel(comp);
    }
}

fn name_style(m: Metrics, tint: &str) -> TextStyle {
    TextStyle {
        size: m.text,
        bold: false,
        color: color(tint, 1.0),
        tabular: false,
    }
}
