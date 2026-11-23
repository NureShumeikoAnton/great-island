//! The break timers on the island: the collapsed row of rings with the nearest clock, the list
//! on hover, and the reminder itself. A ring counts down in the compositor, not here.

use super::{
    Clock, Content, Hit, LATCH, Liveness, MUTED, Mode, Model, Rect, Ring, Scene, TEXT, clock_style,
    dots_space, format_clock, snap, style,
};
use crate::{
    island::comp::{Motion, color, scale, v2, v3},
    lang::tr,
    scheduler::{Snapshot, TimerView},
};
use windows::{UI::Composition::ShapeVisual, core::Result};

const ROW_CLOCK: &str = "#D1D1D6";
const MAX_NAME: f32 = 200.0;
const NEAREST_CLOCK: Liveness = Liveness::Freezable;

impl Scene {
    /// Restarts the rings: each one decays smoothly on its own to the end of its countdown.
    /// Called on changes, not every second.
    pub fn sync_rings(&self, model: &Model) -> Result<()> {
        match &self.content {
            Some(content) => self.sync_content_rings(content, model),
            None => Ok(()),
        }
    }

    pub(super) fn sync_content_rings(&self, content: &Content, model: &Model) -> Result<()> {
        let snapshot = model.snapshot;
        for (id, ring) in &content.rings {
            match id {
                Some(id) => {
                    let Some(timer) = snapshot.timers.iter().find(|t| &t.id == id) else {
                        continue;
                    };
                    let fraction = timer.left_ms as f32 / timer.interval_ms.max(1) as f32;
                    let running = !snapshot.paused
                        && !timer.held
                        && model.skipping != Some(id.as_str())
                        && !in_alert(snapshot, id);
                    self.set_ring(ring, fraction, running.then_some(timer.left_ms))?;
                }
                None => {
                    let Some(alert) = &snapshot.alert else {
                        continue;
                    };
                    let fraction = alert.left_ms as f32 / alert.duration_ms.max(1) as f32;
                    self.set_ring(ring, fraction, Some(alert.left_ms))?;
                }
            }
        }
        Ok(())
    }

    pub fn latch_ring(&self, id: &str) -> Result<()> {
        let Some(content) = &self.content else {
            return Ok(());
        };
        for (_, ring) in content
            .rings
            .iter()
            .filter(|(ring_id, _)| ring_id.as_deref() == Some(id))
        {
            self.comp
                .animate_vector3(&ring.visual, "Scale", scale(1.18, 1.18), LATCH)?;
        }
        Ok(())
    }

    fn set_ring(&self, ring: &Ring, fraction: f32, running_ms: Option<i64>) -> Result<()> {
        let fraction = fraction.clamp(0.0, 1.0);
        self.comp.stop(&ring.arc, "TrimEnd")?;
        match running_ms {
            Some(ms) if ms > 0 && fraction > 0.0 => self.comp.animate_scalar_from(
                &ring.arc,
                "TrimEnd",
                fraction,
                0.0,
                Motion::linear(ms as u32),
            ),
            _ => ring.arc.SetTrimEnd(fraction),
        }
    }

    pub(super) fn collapsed_width(&self, model: &Model) -> f32 {
        self.badges_width(model.modes) + self.collapsed_content_width(model)
    }

    fn collapsed_content_width(&self, model: &Model) -> f32 {
        let snapshot = model.snapshot;
        if snapshot.paused {
            return 13.0
                + 10.0
                + 9.0
                + self
                    .comp
                    .measure(&tr!("island.paused"), style(12.0, false, MUTED))
                + 13.0;
        }
        if snapshot.timers.is_empty() {
            return 13.0
                + self
                    .comp
                    .measure(&tr!("island.no_reminders"), style(12.0, false, MUTED))
                + 13.0;
        }
        let n = snapshot.timers.len() as f32;
        let (label, _) = clock_label(model, None);
        13.0 + n * 11.0
            + (n - 1.0) * 5.0
            + 9.0
            + self.comp.measure(&label, clock_style(13.0, TEXT))
            + 13.0
    }

    pub(super) fn build_collapsed(&self, model: &Model, content: &mut Content) -> Result<()> {
        let snapshot = model.snapshot;
        let root = content.root.clone();
        content.height = 30.0;
        self.badges(&root, 13.0, model.modes)?;
        let start = 13.0 + self.badges_width(model.modes);

        if snapshot.paused || snapshot.timers.is_empty() {
            let mut x = start;
            let message = if snapshot.paused {
                tr!("island.paused")
            } else {
                tr!("island.no_reminders")
            };
            if snapshot.paused {
                self.place(&root, &self.pause_icon()?, x, 10.0)?;
                x += 10.0 + 9.0;
            }
            let st = style(12.0, false, MUTED);
            let text = self.label(&root, &message, st, x, 0.0)?;
            text.visual.SetOffset(v3(
                snap(x, self.scale),
                snap(15.0 - self.comp.x_middle(st), self.scale),
            ))?;
            content.width = x + text.width + 13.0;
            return Ok(());
        }

        let mut x = start;
        for timer in &snapshot.timers {
            let ring = self.ring(&timer.color, 11.0, 2.2)?;
            self.place(&root, &ring.visual, x, 9.5)?;
            content.hits.push((
                Rect {
                    x: x - 3.0,
                    y: 0.0,
                    w: 17.0,
                    h: 30.0,
                },
                Hit::Timer(timer.id.clone()),
            ));
            content.rings.push((Some(timer.id.clone()), ring));
            x += 11.0 + 5.0;
        }
        content.clock_left = x - 5.0 + 9.0;
        let text = self.comp.text("", clock_style(13.0, TEXT), self.scale)?;
        root.Children()?.InsertAtTop(&text.visual)?;
        content.clocks.push(Clock {
            text,
            timer: None,
            right: None,
            y: 0.0,
        });
        Ok(())
    }

    fn pause_icon(&self) -> Result<ShapeVisual> {
        let c = &self.comp.compositor;
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(10.0, 10.0))?;
        let brush = self.comp.brush(color(MUTED, 1.0))?;
        for x in [1.5, 6.1] {
            let geometry = c.CreateRoundedRectangleGeometry()?;
            geometry.SetSize(v2(2.4, 8.0))?;
            geometry.SetCornerRadius(v2(1.0, 1.0))?;
            let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
            shape.SetOffset(v2(x, 1.0))?;
            shape.SetFillBrush(&brush)?;
            visual.Shapes()?.Append(&shape)?;
        }
        Ok(visual)
    }

    pub(super) fn expanded_size(&self, model: &Model) -> (f32, f32) {
        let timers = &model.snapshot.timers;
        let name_w = timers
            .iter()
            .map(|t| {
                self.comp
                    .measure(&t.name, style(13.0, false, TEXT))
                    .min(MAX_NAME)
            })
            .fold(0.0, f32::max);
        let clock_w = timers
            .iter()
            .map(|t| {
                self.comp.measure(
                    &clock_label(model, Some(&t.id)).0,
                    clock_style(13.0, ROW_CLOCK),
                )
            })
            .fold(0.0, f32::max);
        let mut width = (14.0 + 13.0 + 9.0 + name_w + 16.0 + clock_w + 14.0).max(220.0);
        if timers.is_empty() {
            let hint = self
                .comp
                .measure(&tr!("island.enable_reminders"), style(12.0, false, MUTED));
            width = width.max(14.0 + hint + 14.0);
        }
        let rows = timers.len().max(1) as f32;
        let top = 12.0 + dots_space(model);
        (width, top + rows * 18.0 + rows * 8.0 + 4.0 + 28.0 + 12.0)
    }

    pub(super) fn build_expanded(&self, model: &Model, content: &mut Content) -> Result<()> {
        let snapshot = model.snapshot;
        let root = content.root.clone();
        let (width, height) = self.expanded_size(model);
        content.width = width;
        content.height = height;
        self.dots(&root, width, model)?;

        let mut y = 12.0 + dots_space(model);
        if snapshot.timers.is_empty() {
            let text = self.label(
                &root,
                &tr!("island.enable_reminders"),
                style(12.0, false, MUTED),
                14.0,
                y,
            )?;
            let _ = text;
            y += 18.0 + 8.0;
        }
        for timer in &snapshot.timers {
            let ring = self.ring(&timer.color, 13.0, 2.4)?;
            self.place(&root, &ring.visual, 14.0, y + 2.5)?;
            content.rings.push((Some(timer.id.clone()), ring));

            let name_style = style(13.0, false, TEXT);
            let name = self.comp.fit(&timer.name, name_style, MAX_NAME);
            let text = self.comp.text(&name, name_style, self.scale)?;
            self.place(
                &root,
                &text.visual,
                14.0 + 13.0 + 9.0,
                y + self.comp.center_top(&text, 18.0),
            )?;

            let clock = self
                .comp
                .text("", clock_style(13.0, ROW_CLOCK), self.scale)?;
            root.Children()?.InsertAtTop(&clock.visual)?;
            let clock_y = y + (18.0 - clock.height.max(17.0)) / 2.0;
            content.clocks.push(Clock {
                text: clock,
                timer: Some(timer.id.clone()),
                right: Some(width - 14.0),
                y: clock_y,
            });

            content.hits.push((
                Rect {
                    x: 8.0,
                    y: y - 4.0,
                    w: width - 16.0,
                    h: 26.0,
                },
                Hit::Timer(timer.id.clone()),
            ));
            y += 18.0 + 8.0;
        }

        y += 4.0;
        let button_w = (220.0 - 28.0 - 6.0) / 2.0;
        let pause_label = if snapshot.paused {
            tr!("island.resume")
        } else {
            tr!("island.pause")
        };
        let pause = self.button(
            &root,
            &pause_label,
            Rect {
                x: (width - button_w) / 2.0,
                y,
                w: button_w,
                h: 28.0,
            },
            Hit::Pause,
            TEXT,
            color("#FFFFFF", 0.12),
            color("#FFFFFF", 0.2),
        )?;
        content.hits.push((pause.rect, pause.hit.clone()));
        content.buttons.push(pause);
        Ok(())
    }

    fn alert_timers<'a>(&self, snapshot: &'a Snapshot) -> Vec<&'a TimerView> {
        let Some(alert) = &snapshot.alert else {
            return Vec::new();
        };
        alert
            .ids
            .iter()
            .filter_map(|id| snapshot.timers.iter().find(|t| &t.id == id))
            .collect()
    }

    fn alert_texts(&self, snapshot: &Snapshot) -> (String, String) {
        let timers = self.alert_timers(snapshot);
        let title = timers
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let title = self.comp.fit(&title, style(15.0, true, TEXT), MAX_NAME);
        let minutes = timers.first().map(|t| t.interval_ms / 60_000).unwrap_or(0);
        (title, tr!("island.next_in", minutes = minutes).into_owned())
    }

    pub(super) fn alert_width(&self, model: &Model) -> f32 {
        let (title, subtitle) = self.alert_texts(model.snapshot);
        let text_w = self
            .comp
            .measure(&title, style(15.0, true, TEXT))
            .max(self.comp.measure(&subtitle, style(12.0, false, MUTED)));
        let button_w = self
            .comp
            .measure(&tr!("island.done"), style(12.0, true, TEXT))
            + 32.0;
        (14.0 + 42.0 + 12.0 + text_w + 12.0 + button_w + 12.0).max(300.0)
    }

    pub(super) fn build_alert(&self, model: &Model, content: &mut Content) -> Result<()> {
        let snapshot = model.snapshot;
        let root = content.root.clone();
        let Some(main) = self.alert_timers(snapshot).first().copied() else {
            content.width = 300.0;
            content.height = 66.0;
            return Ok(());
        };
        content.width = self.alert_width(model);
        content.height = 66.0;

        let ring = self.ring(&main.color, 42.0, 3.5)?;
        self.place(&root, &ring.visual, 14.0, 12.0)?;
        content.rings.push((None, ring));
        let number = self.comp.text("", clock_style(13.0, TEXT), self.scale)?;
        root.Children()?.InsertAtTop(&number.visual)?;
        content.clocks.push(Clock {
            text: number,
            timer: None,
            right: None,
            y: 0.0,
        });

        let (title, subtitle) = self.alert_texts(snapshot);
        let title = self
            .comp
            .text(&title, style(15.0, true, TEXT), self.scale)?;
        let subtitle = self
            .comp
            .text(&subtitle, style(12.0, false, MUTED), self.scale)?;
        let top = self.comp.stack_top(&title, &subtitle, 2.0, 66.0);
        self.place(&root, &title.visual, 68.0, top)?;
        self.place(&root, &subtitle.visual, 68.0, top + title.height + 2.0)?;

        let done_label = tr!("island.done");
        let label_w = self.comp.measure(&done_label, style(12.0, true, TEXT));
        let rect = Rect {
            x: content.width - 12.0 - (label_w + 32.0),
            y: 17.0,
            w: label_w + 32.0,
            h: 32.0,
        };
        let done = self.button(
            &root,
            &done_label,
            rect,
            Hit::Done,
            &main.color,
            color(&main.color, 0.22),
            color(&main.color, 0.34),
        )?;
        content.hits.push((done.rect, Hit::Done));
        content.buttons.push(done);
        Ok(())
    }

    pub(super) fn update_timers(&self, mode: Mode, content: &mut Content) -> Result<()> {
        let s = self.scale;
        match mode {
            Mode::Collapsed => {
                if let Some(clock) = content.clocks.first() {
                    clock.text.visual.SetOffset(v3(
                        snap(content.clock_left, s),
                        snap(self.comp.center_top(&clock.text, 30.0), s),
                    ))?;
                    content.width = content.clock_left + clock.text.width + 13.0;
                }
            }
            Mode::Expanded => {
                for clock in &content.clocks {
                    if let Some(right) = clock.right {
                        clock
                            .text
                            .visual
                            .SetOffset(v3(snap(right - clock.text.width, s), snap(clock.y, s)))?;
                    }
                }
            }
            Mode::Alert => {
                if let Some(clock) = content.clocks.first() {
                    clock.text.visual.SetOffset(v3(
                        snap(14.0 + (42.0 - clock.text.width) / 2.0, s),
                        snap(12.0 + self.comp.center_top(&clock.text, 42.0), s),
                    ))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn in_alert(snapshot: &Snapshot, id: &str) -> bool {
    snapshot
        .alert
        .as_ref()
        .is_some_and(|a| a.ids.iter().any(|x| x == id))
}

/// Clock text and, if needed, its color. `timer` is a row of the expanded view; `None` = the collapsed
/// view's clock (to the nearest) or the reminder's seconds.
pub(super) fn clock_label(model: &Model, timer: Option<&str>) -> (String, Option<String>) {
    let snapshot = model.snapshot;
    match (model.mode, timer) {
        (Mode::Alert, _) => {
            let left = snapshot.alert.as_ref().map(|a| a.left_ms).unwrap_or(0);
            (((left + 999) / 1000).to_string(), None)
        }
        (_, Some(id)) => {
            let Some(t) = snapshot.timers.iter().find(|t| t.id == id) else {
                return (String::new(), None);
            };
            if model.skipping == Some(id) {
                (
                    format!("» {}", format_clock(t.left_ms)),
                    Some(t.color.clone()),
                )
            } else if t.held {
                (tr!("island.waiting").into_owned(), Some(ROW_CLOCK.into()))
            } else {
                (format_clock(t.left_ms), Some(ROW_CLOCK.into()))
            }
        }
        (_, None) => {
            let Some(nearest) = snapshot.timers.iter().min_by_key(|t| t.left_ms) else {
                return (String::new(), None);
            };
            let label = if !NEAREST_CLOCK.moving(model.frozen) && nearest.left_ms > 60_000 {
                format_minutes(nearest.left_ms)
            } else {
                format_clock(nearest.left_ms)
            };
            (label, Some(nearest.color.clone()))
        }
    }
}

fn format_minutes(ms: i64) -> String {
    tr!("time.minutes", m = (ms.max(0) + 59_999) / 60_000).into_owned()
}
