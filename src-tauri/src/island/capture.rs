//! What the screenshot and the recording share on the island: what is captured, keeping the
//! island itself out of it, and the card both end in.
//!
//! - **The source.** `capture_source` is the whole monitor with the island. An area picked in
//!   the selection (`select.rs`) is a smaller rectangle on it: the screenshot cuts it out of
//!   its capture, the recording records it.
//! - **Hiding.** The island stays on the monitor but is absent from captures while a shot is
//!   being taken or waits on its card, and for the whole recording.
//! - **The card.** One view (`view/card.rs`) with two owners: a taken screenshot and a finished
//!   recording. Both can exist at once; the notice queue decides which one is on screen, and
//!   that one gets the click, Ctrl+C and Ctrl+X.
//! - **The windows.** A screenshot opens in the screenshot viewer, a recording in the video window
//!   (`watch.rs`); both can be open. The island's status (`Mode::Viewing`) belongs to the one
//!   opened last: its thumbnail, its window raised by a click, sent back by the arrow.
//! - **The flights** between the card and a window: `start_glide`, `card_place`.

use super::{
    App,
    flight::{Flight, GLIDE_FLIGHT_MS, Place},
    timers,
    view::{CardPhase, Mode, Video},
};
use crate::{
    capture::{Image, Source, Status, adapters as system},
    journal,
    lang::tr,
    platform,
    settings::Fullscreen,
    stage::NoticeKind,
};
use std::sync::Arc;
use windows::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};

/// Thumbnail copy width: with headroom for 200% scale.
pub(super) const THUMB_WIDTH: u32 = 320;
/// Width of the flight copy: full-screen it is visible only for a moment under the flash,
/// and it takes a few MB only during the flight.
pub(super) const PREVIEW_WIDTH: u32 = 1280;
/// The tick stays this long after a copy, then the card closes.
pub(super) const QUICK_CLOSE_MS: u32 = 1000;
pub(super) const FLIGHT_TAIL_MS: u32 = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Front {
    Shot,
    Video,
}

pub(super) struct Card {
    pub(super) thumb: Arc<Image>,
    pub(super) status: Status,
    pub(super) phase: CardPhase,
    /// A recording's length and size; `None` = a screenshot.
    pub(super) video: Option<Video>,
}

impl App {
    pub(super) fn capture_source(&self) -> Source {
        system::whole_monitor(self.monitor)
    }

    pub(super) fn sync_capture_hiding(&self) {
        let hidden = self.recording.live.is_some()
            || self.shots.taken.is_some()
            || self.stage.has(NoticeKind::Shutter)
            || self.stage.has(NoticeKind::Scan);
        system::hide_from_capture(self.island, hidden);
    }

    pub(super) fn has_card(&self) -> bool {
        self.shots.taken.is_some() || self.recording.clip.is_some()
    }

    pub(super) fn card(&self) -> Option<Card> {
        match (&self.shots.taken, &self.recording.clip) {
            (_, Some(clip)) if self.showing_clip() => Some(Card {
                thumb: clip.thumb.clone(),
                status: clip.take.status(),
                phase: CardPhase::Open,
                video: Some(Video {
                    duration_ms: clip.take.duration_ms(),
                    bytes: clip.take.bytes(),
                }),
            }),
            (Some(taken), _) => Some(Card {
                thumb: taken.thumb.clone(),
                status: taken.status(),
                phase: taken.phase.unwrap_or(CardPhase::Catching),
                video: None,
            }),
            _ => None,
        }
    }

    /// Ctrl+C / Ctrl+X or a click on the card's text (`only` = clipboard only, no file).
    /// The card on screen decides; when it is hidden under a reminder, the screenshot goes first.
    pub(super) fn copy_card(&mut self, only: bool) {
        if self.showing_clip() || self.shots.taken.is_none() {
            self.copy_clip(only);
        } else {
            self.copy_shot(only);
        }
    }

    pub(super) fn open_card(&mut self) {
        if self.showing_clip() {
            self.open_clip();
        } else {
            self.open_shot();
        }
    }

    pub(super) fn start_glide(
        &mut self,
        preview: &Image,
        path: (Place, Place),
        radius: (f32, f32),
    ) -> bool {
        let monitor = platform::monitor_rect(self.monitor);
        match Flight::glide(&self.scene.comp, monitor, self.scale, preview, path, radius) {
            Ok(flight) => {
                self.shots.flight = Some(flight);
                self.set_timer(timers::FLIGHT, GLIDE_FLIGHT_MS + FLIGHT_TAIL_MS);
                true
            }
            Err(e) => {
                journal::warn(tr!("journal.shot_transfer_failed", error = e));
                false
            }
        }
    }

    pub(super) fn card_place(&self) -> Option<Place> {
        let input = self.view_input(Mode::Card);
        let (x, y, w, h) = self.scene.card_slot(&input.model())?;
        let monitor = platform::monitor_rect(self.monitor);
        let mut window = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.island, &mut window);
        }
        let k = self.scale;
        Some((
            (window.left - monitor.left) as f32 / k + x,
            (window.top - monitor.top) as f32 / k + y,
            w,
            h,
        ))
    }

    /// A card will be visible: the island isn't hidden from the tray or by a fullscreen app,
    /// and isn't occupied by a reminder.
    pub(super) fn island_takes_cards(&self) -> bool {
        let fullscreen_hide = self.fullscreen && self.settings.fullscreen == Fullscreen::Hide;
        self.hidden.is_none() && !fullscreen_hide && !self.scheduler.has_alert()
    }

    /// The viewer window the island's status belongs to: the one opened last.
    pub(super) fn front(&self) -> Option<Front> {
        let shot = self.shots.viewing.as_ref().map(|v| v.opened_at);
        let video = self.watch.watching.as_ref().map(|w| w.opened_at);
        match (shot, video) {
            (Some(shot), Some(video)) if video >= shot => Some(Front::Video),
            (Some(_), _) => Some(Front::Shot),
            (None, Some(_)) => Some(Front::Video),
            (None, None) => None,
        }
    }

    /// The status's thumbnail: the front window's, or a screenshot flying home from its window.
    pub(super) fn viewing_thumb(&self) -> Option<Arc<Image>> {
        match self.front() {
            Some(Front::Shot) => self.shots.viewing.as_ref().map(|v| v.thumb.clone()),
            Some(Front::Video) => self.watch.watching.as_ref().map(|w| w.clip.thumb.clone()),
            None => self.shots.taken.as_ref().map(|t| t.thumb.clone()),
        }
    }

    /// `Some(running)` when the status belongs to the video window: it shows ▶/❚❚.
    pub(super) fn viewing_play(&self) -> Option<bool> {
        match self.front() {
            Some(Front::Video) => self.watched_running(),
            _ => None,
        }
    }

    pub(super) fn raise_front(&self) {
        match self.front() {
            Some(Front::Shot) => self.raise_viewer(),
            Some(Front::Video) => self.raise_watched(),
            None => {}
        }
    }

    pub(super) fn return_front(&mut self) {
        match self.front() {
            Some(Front::Shot) => self.return_viewed(),
            Some(Front::Video) => self.return_watched(),
            None => {}
        }
    }

    /// The status ends when no viewer window is left. True = the notice queue changed.
    pub(super) fn end_viewing(&mut self) -> bool {
        if self.front().is_some() {
            return false;
        }
        self.stage.end(NoticeKind::Viewing, self.now_ms())
    }
}
