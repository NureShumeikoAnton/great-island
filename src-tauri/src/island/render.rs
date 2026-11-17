//! From the island's state to the picture: which view is on ([`Mode`]), the model the scene
//! draws from, and the redraw. The scene (`view`) draws what the model says and nothing else.
//!
//! A new view gets its line in `activity_mode` (when it is on) and in `view_input` (what it shows).

use super::{
    App,
    capture::Card,
    log,
    record_panel::PanelInput,
    timers,
    view::{CardModel, Mode, Model, Modes, PlayerForm, PlayerModel, ProblemModel, RecordModel},
};
use crate::{
    capture::Image,
    control::{ControlId, Item},
    fault::Level,
    gesture::Dir,
    media::{LevelSource, Session},
    persist,
    scheduler::Snapshot,
    stage::{Activity, NoticeKind},
};
use std::sync::Arc;

const PURGE_DELAY_MS: u32 = 250;

/// A copy of what the scene needs: the model refers to it, not to the island,
/// so the scene can be changed while the model is alive.
pub(super) struct ViewInput {
    snapshot: Snapshot,
    mode: Mode,
    frozen: bool,
    skipping: Option<String>,
    session: Option<Session>,
    headphones: bool,
    decorative: bool,
    copied: bool,
    pub(super) dots: Option<(usize, usize)>,
    controls: Vec<Item>,
    now_ms: u64,
    modes: Modes,
    card: Option<Card>,
    viewing: Option<Arc<Image>>,
    viewing_play: Option<bool>,
    /// Recording: elapsed ms, still (paused or closing), file bytes, hover.
    record: Option<(u64, bool, u64, bool)>,
    countdown: Option<u32>,
    /// A problem: is it critical, what went wrong, why.
    problem: Option<(bool, String, String)>,
    panel: Option<PanelInput>,
}

impl ViewInput {
    pub(super) fn model(&self) -> Model<'_> {
        Model {
            snapshot: &self.snapshot,
            mode: self.mode,
            frozen: self.frozen,
            skipping: self.skipping.as_deref(),
            player: self.session.as_ref().map(|session| PlayerModel {
                session,
                now_unix_ms: persist::now_ms(),
                headphones: self.headphones,
                decorative: self.decorative,
                copied: self.copied,
            }),
            dots: self.dots,
            controls: &self.controls,
            now_ms: self.now_ms,
            modes: self.modes,
            card: self.card.as_ref().map(|card| CardModel {
                thumb: &card.thumb,
                status: card.status,
                phase: card.phase,
                video: card.video,
            }),
            viewing: self.viewing.as_ref(),
            viewing_play: self.viewing_play,
            record: self
                .record
                .map(|(elapsed_ms, paused, bytes, hover)| RecordModel {
                    elapsed_ms,
                    paused,
                    bytes,
                    hover,
                }),
            countdown: self.countdown,
            problem: self
                .problem
                .as_ref()
                .map(|(critical, title, reason)| ProblemModel {
                    critical: *critical,
                    title,
                    reason,
                }),
            panel: self.panel.as_ref().map(PanelInput::model),
        }
    }
}

impl App {
    fn mode(&self) -> Mode {
        if self.stage.shown().menu {
            match self.record_panel.page() {
                Some(page) => Mode::RecordPanel(page),
                None => Mode::Menu,
            }
        } else {
            self.activity_mode()
        }
    }

    pub(super) fn activity_mode(&self) -> Mode {
        let shown = self.stage.shown();
        let notice = shown.notice.map(|n| n.kind);
        let indicator = |id| Mode::Indicator(id, self.controls.is_on(id));
        match shown.activity {
            _ if notice == Some(NoticeKind::Alert) && self.scheduler.has_alert() => Mode::Alert,
            _ if notice.is_some_and(NoticeKind::is_problem) && self.problems.shown.is_some() => {
                Mode::Problem {
                    open: self.expanded,
                }
            }
            _ if notice == Some(NoticeKind::Sound) => indicator(ControlId::Sound),
            _ if notice == Some(NoticeKind::Microphone) => indicator(ControlId::Microphone),
            _ if notice == Some(NoticeKind::Shutter) => Mode::Shutter,
            _ if notice == Some(NoticeKind::Shot) && self.shots.taken.is_some() => Mode::Card,
            _ if notice == Some(NoticeKind::Countdown) && self.recording.countdown.is_some() => {
                Mode::Countdown
            }
            _ if notice == Some(NoticeKind::Recording) && self.recording.live.is_some() => {
                Mode::Recording
            }
            _ if notice == Some(NoticeKind::Clip) && self.recording.clip.is_some() => Mode::Card,
            _ if notice == Some(NoticeKind::Discarded) => Mode::Discarded,
            _ if notice == Some(NoticeKind::Viewing) => Mode::Viewing,
            Activity::Timers if self.expanded || self.reminders.skip.is_some() => Mode::Expanded,
            Activity::Timers => Mode::Collapsed,
            Activity::Player if self.expanded => self.player_card(),
            Activity::Player if notice == Some(NoticeKind::Track) => {
                Mode::Player(PlayerForm::Announce)
            }
            Activity::Player => Mode::Player(PlayerForm::Collapsed),
        }
    }

    pub(super) fn view_input(&self, mode: Mode) -> ViewInput {
        let expanded = matches!(
            mode,
            Mode::Expanded | Mode::Player(PlayerForm::Expanded { .. })
        );
        ViewInput {
            snapshot: self.scheduler.snapshot(),
            mode,
            frozen: self.frozen(mode),
            skipping: self.reminders.skip.as_ref().map(|s| s.id.clone()),
            session: match mode {
                Mode::Player(_) => self.player.tracker.current().cloned(),
                _ => None,
            },
            headphones: self.player.headphones,
            decorative: self.player.meter.source() == LevelSource::None,
            copied: self.player.copied,
            dots: expanded.then(|| self.stage.ribbon_position()),
            controls: match mode {
                Mode::Menu => self.controls.items(),
                _ => Vec::new(),
            },
            now_ms: self.now_ms(),
            modes: Modes {
                quiet: self.controls.is_on(ControlId::DoNotDisturb),
                awake: self.controls.is_on(ControlId::KeepAwake),
                problem: !self.problems.missed.is_empty(),
            },
            card: match mode {
                Mode::Card => self.card(),
                _ => None,
            },
            viewing: match mode {
                Mode::Viewing => self.viewing_thumb(),
                _ => None,
            },
            viewing_play: match mode {
                Mode::Viewing => self.viewing_play(),
                _ => None,
            },
            record: match (mode, &self.recording.live) {
                (Mode::Recording, Some(live)) => {
                    let now = self.now_ms();
                    Some((live.elapsed(now), live.still(), live.bytes(), self.expanded))
                }
                _ => None,
            },
            countdown: self.recording.countdown,
            problem: match (mode, &self.problems.shown) {
                (Mode::Problem { .. }, Some(shown)) => Some((
                    shown.fault.level() == Level::Critical,
                    shown.fault.title().to_string(),
                    shown.fault.reason(),
                )),
                _ => None,
            },
            panel: match mode {
                Mode::RecordPanel(_) => self.panel_input(),
                _ => None,
            },
        }
    }

    pub(super) fn neighbor_input(&self, dir: Dir) -> Option<ViewInput> {
        let activity = self.stage.neighbor(dir)?;
        let mode = match (activity, self.expanded) {
            (Activity::Timers, false) => Mode::Collapsed,
            (Activity::Timers, true) => Mode::Expanded,
            (Activity::Player, false) => Mode::Player(PlayerForm::Collapsed),
            (Activity::Player, true) => self.player_card(),
        };
        let mut input = self.view_input(mode);
        if let Some((count, current)) = input.dots {
            let next = match dir {
                Dir::Next => current + 1,
                Dir::Prev => current.saturating_sub(1),
            };
            input.dots = Some((count, next));
        }
        Some(input)
    }

    pub(super) fn render(&mut self) {
        let mode = self.mode();
        if mode == (Mode::Problem { open: true }) {
            self.mark_problem_seen();
        }
        self.update_rest(mode);
        self.note_player_card(mode);
        let input = self.view_input(mode);
        let model = input.model();
        match self.scene.render(&model) {
            Ok((size, rebuilt)) => {
                if rebuilt {
                    log(self.scene.sync_rings(&model));
                    self.purge_later();
                }
                self.set_shape(size);
            }
            Err(e) => log(Err(e)),
        }
        self.sync_bars();
    }

    pub(super) fn sync_rings(&self) {
        let input = self.view_input(self.mode());
        log(self.scene.sync_rings(&input.model()));
    }

    /// The views that faded out are removed once their fade is over.
    pub(super) fn purge_later(&self) {
        self.set_timer(timers::PURGE, PURGE_DELAY_MS);
    }

    pub(super) fn on_purge_timer(&mut self) {
        self.kill_timer(timers::PURGE);
        log(self.scene.purge());
    }
}
