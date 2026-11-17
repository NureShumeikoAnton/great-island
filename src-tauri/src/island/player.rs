//! The media player on the island: follows the system's media sessions, drops a player that
//! is silent, keeps the loudness meter connected, feeds the level bars and sends the buttons' commands.
//!
//! Which session is the current one is decided by `media::Tracker`.

use super::{
    App, log, timers,
    view::{Mode, PlayerForm, PlayerHit, SAMPLE_MS},
    window::WM_MEDIA,
};
use crate::{
    capture::adapters::SystemClipboard,
    fault::{Cause, Failed, Fault, What},
    journal,
    lang::tr,
    loudness::Leveler,
    media::{self, LevelSource, Media, Meter, PlayerEvent, Session, Tracker},
    persist,
    stage::{Activity, NoticeKind, Presence},
};
use std::time::Instant;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

/// A peak level below this is silence (−60 dB).
const SILENCE_LEVEL: f32 = 0.001;
/// Pauses before each reconnect while the loudness is lost (ms). After the last one only an event
/// reconnects (a player, play state, track or output change), and the user is told.
const RETRY_MS: [u64; 5] = [1000, 2000, 4000, 8000, 16_000];
const COPIED_MS: u32 = 1500;

pub(super) struct Player {
    /// The thread that listens to the system; `None` = the player is turned off in the settings.
    pub(super) media: Option<Media>,
    pub(super) tracker: Tracker,
    pub(super) meter: Meter,
    meter_for: Option<Listened>,
    /// Why the loudness is lost; already in the journal. Cleared by the next successful read.
    meter_failure: Option<Failed>,
    /// The notice about the lost loudness is shown; cleared by the next successful read.
    meter_reported: bool,
    retries: usize,
    retry_at: u64,
    last_sound: u64,
    /// The level bars are being fed.
    sampling: bool,
    last_sample: Instant,
    leveler: Leveler,
    pub(super) headphones: bool,
    card_shown: bool,
    /// The card says the track was copied, until `timers::COPIED`.
    pub(super) copied: bool,
}

impl Player {
    pub(super) fn new() -> Self {
        Self {
            media: None,
            tracker: Tracker::new(),
            meter: Meter::new(),
            meter_for: None,
            meter_failure: None,
            meter_reported: false,
            retries: 0,
            retry_at: 0,
            last_sound: 0,
            sampling: false,
            last_sample: Instant::now(),
            leveler: Leveler::default(),
            headphones: false,
            card_shown: false,
            copied: false,
        }
    }
}

/// What the meter was connected for. The player's audio may move to another process or stream on a
/// player change, when playback starts and on a track change (a new browser tab).
#[derive(PartialEq)]
struct Listened {
    key: String,
    playing: bool,
    title: String,
    artist: String,
}

impl Listened {
    fn of(session: &Session) -> Self {
        Self {
            key: session.key.clone(),
            playing: session.is_playing(),
            title: session.title.clone(),
            artist: session.artist.clone(),
        }
    }
}

impl App {
    pub(super) fn sync_media(&mut self) {
        if self.settings.player && self.player.media.is_none() {
            // A HWND can't be passed to another thread, a number can.
            let service = self.service.0 as isize;
            self.player.media = Some(Media::start(move || unsafe {
                let target = HWND(service as *mut _);
                let _ = PostMessageW(Some(target), WM_MEDIA, WPARAM(0), LPARAM(0));
            }));
        } else if !self.settings.player && self.player.media.is_some() {
            self.player.media = None;
            self.player.tracker = Tracker::new();
        }
        let changed = self.apply_player(&[]);
        self.after_stage(changed);
    }

    pub(super) fn on_media(&mut self) {
        let Some(media) = &self.player.media else {
            return;
        };
        let now = self.now_ms();
        let mut events = Vec::new();
        for update in media.drain() {
            events.extend(self.player.tracker.apply(update, now));
        }
        let changed = self.apply_player(&events);
        self.after_stage(changed);
        if self.visible && self.stage.shown().activity == Activity::Player {
            self.render();
        }
    }

    pub(super) fn apply_player(&mut self, events: &[PlayerEvent]) -> bool {
        let now = self.now_ms();
        let presence = if self.player.media.is_some() {
            self.player.tracker.presence(now)
        } else {
            Presence::Absent
        };
        let mut changed = self.stage.set_presence(Activity::Player, presence, now);
        for event in events {
            changed |= match event {
                PlayerEvent::Announce if self.settings.announce_tracks => {
                    self.stage.post(NoticeKind::Track, now)
                }
                PlayerEvent::Announce => false,
                PlayerEvent::Paused => self.stage.post(NoticeKind::Paused, now),
            };
        }
        changed
    }

    /// Once a second: reconnects the meter when needed, and removes a player that is "playing" but silent
    /// for longer than the setting until its next event (a video without sound in a background tab, etc.).
    pub(super) fn check_silence(&mut self, now: u64) {
        let Some(session) = self.player.tracker.current() else {
            self.player.meter.detach();
            self.player.meter_for = None;
            return;
        };
        let target = Listened::of(session);
        let playing = target.playing;
        let lost = playing && self.player.meter.source() == LevelSource::None;
        let event =
            self.player.meter_for.as_ref() != Some(&target) || self.player.meter.output_changed();
        if event {
            self.player.retries = 0;
        }
        // While playing without a level, retry a few times: the device may be back.
        let retry =
            lost && !event && self.player.retries < RETRY_MS.len() && now >= self.player.retry_at;
        if event || retry {
            if retry {
                self.player.retries += 1;
            }
            self.player.retry_at = now + RETRY_MS.get(self.player.retries).copied().unwrap_or(0);
            let app_id = session.app_id.clone();
            self.player.meter_for = Some(target);
            self.player.last_sound = now;
            let attached = self.player.meter.attach(&app_id);
            // Only a miss is worth a line: a player found by its own audio session is the normal case.
            if self.player.meter.source() != LevelSource::Player {
                journal::info(tr!(
                    "journal.meter_attached",
                    app = app_id,
                    source = format!("{:?}", self.player.meter.source()),
                    found = self.player.meter.describe()
                ));
            }
            if let Err(e) = attached {
                self.meter_lost(e);
            }
        }
        if lost
            && !event
            && self.player.retries == RETRY_MS.len()
            && self.player.meter.source() == LevelSource::None
            && !self.player.meter_reported
        {
            self.player.meter_reported = true;
            let failed = self
                .player
                .meter_failure
                .clone()
                .unwrap_or_else(|| Failed::new(Cause::Unknown));
            self.report(Fault::new(What::PlayerLevel, failed));
        }
        let limit = self.settings.silent_drop_min as u64 * 60_000;
        // Silence is measured only by the player's own audio: the overall system sound says nothing about it.
        if !playing || limit == 0 || self.player.meter.source() != LevelSource::Player {
            self.player.last_sound = now;
            return;
        }
        let loud = match self.player.meter.sample() {
            Ok(Some(level)) => {
                self.meter_back();
                level.left.max(level.right) > SILENCE_LEVEL
            }
            Ok(None) => false,
            Err(e) => {
                self.meter_lost(e);
                return;
            }
        };
        if loud {
            self.player.last_sound = now;
        } else if now.saturating_sub(self.player.last_sound) >= limit {
            self.player.tracker.mark_silent();
            self.player.meter_for = None;
        }
    }

    /// The player's card; without a track position it has no progress row.
    pub(super) fn player_card(&self) -> Mode {
        let timeline = self
            .player
            .tracker
            .current()
            .is_some_and(|s| s.timeline.is_some());
        Mode::Player(PlayerForm::Expanded { timeline })
    }

    /// The output device is asked when the card opens, not on every redraw.
    pub(super) fn note_player_card(&mut self, mode: Mode) {
        let shown = matches!(mode, Mode::Player(PlayerForm::Expanded { .. }));
        if shown && !self.player.card_shown {
            self.player.headphones = media::output_is_headphones();
        }
        self.player.card_shown = shown;
    }

    pub(super) fn sync_bars(&mut self) {
        let want = self.visible && self.scene.wants_levels();
        if want == self.player.sampling {
            return;
        }
        self.player.sampling = want;
        if want {
            self.player.last_sample = Instant::now();
            self.set_timer(timers::BARS, SAMPLE_MS);
        } else {
            self.kill_timer(timers::BARS);
        }
    }

    pub(super) fn on_bars_timer(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.player.last_sample).as_secs_f32() * 1000.0;
        self.player.last_sample = now;
        match self.player.meter.sample() {
            Ok(Some(mut level)) => {
                self.meter_back();
                let source = self
                    .player
                    .tracker
                    .current()
                    .map_or("", |s| s.app_id.as_str());
                let gain = self
                    .player
                    .leveler
                    .gain(source, level.left.max(level.right), dt);
                #[cfg(debug_assertions)]
                super::levels_debug::show(level, gain, self.player.meter.source());
                level.left *= gain;
                level.right *= gain;
                log(self.scene.feed_levels(level, dt));
            }
            Ok(None) => self.render(),
            Err(e) => {
                self.meter_lost(e);
                self.render();
            }
        }
    }

    /// Journal only: the bars fall back to the decorative animation and the meter usually reconnects
    /// within a second (a headphones switch). The user is told only when the retries run out.
    fn meter_lost(&mut self, e: Failed) {
        if self.player.meter_failure.is_none() {
            journal::warn(tr!(
                "journal.meter_lost",
                error = format!("{} {}", e.cause.code(), e.detail)
            ));
        }
        self.player.meter_failure = Some(e);
    }

    fn meter_back(&mut self) {
        self.player.meter_failure = None;
        self.player.meter_reported = false;
        self.player.retries = 0;
    }

    pub(super) fn player_click(&mut self, hit: PlayerHit) {
        let Some(session) = self.player.tracker.current() else {
            return;
        };
        let command = match hit {
            PlayerHit::Open => {
                media::open_app(&session.app_id);
                return;
            }
            PlayerHit::PlayPause => media::Command::TogglePlayPause,
            PlayerHit::Previous => media::Command::Previous,
            PlayerHit::Next => media::Command::Next,
            PlayerHit::Copy => {
                self.copy_track();
                return;
            }
            PlayerHit::Seek => return,
        };
        self.send_media(command);
    }

    /// "Artist — Title", or the title alone.
    fn copy_track(&mut self) {
        let Some(session) = self.player.tracker.current() else {
            return;
        };
        let text = match (session.artist.is_empty(), session.title.is_empty()) {
            (_, true) => return,
            (true, false) => session.title.clone(),
            (false, false) => format!("{} — {}", session.artist, session.title),
        };
        match SystemClipboard::new(self.island).put_text(&text) {
            Ok(()) => {
                self.player.copied = true;
                self.set_timer(timers::COPIED, COPIED_MS);
                self.render();
            }
            Err(failed) => self.report(Fault::new(What::Copy, failed)),
        }
    }

    pub(super) fn on_copied_timer(&mut self) {
        self.kill_timer(timers::COPIED);
        self.player.copied = false;
        if self.visible {
            self.render();
        }
    }

    /// The line and the time jump to the click at once, without waiting for the player.
    pub(super) fn seek_to(&mut self, fraction: f32) {
        let duration = self
            .player
            .tracker
            .current()
            .and_then(|s| s.timeline)
            .map(|t| t.duration_ms);
        if let Some(duration) = duration {
            let position = (duration as f32 * fraction) as i64;
            self.player
                .tracker
                .seek(position, persist::now_ms(), self.now_ms());
            self.send_media(media::Command::Seek(position));
            if self.visible {
                self.render();
            }
        }
    }

    /// A command to the current player. A track announce after it isn't needed: the track was switched from here.
    fn send_media(&mut self, command: media::Command) {
        let Some(key) = self.player.tracker.current().map(|s| s.key.clone()) else {
            return;
        };
        self.player.tracker.own_command(self.now_ms());
        if let Some(media) = &self.player.media {
            media.send(&key, command);
        }
    }
}
