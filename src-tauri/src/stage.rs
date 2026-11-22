//! What the island shows: the activity ribbon, the background and notices over it.
//!
//! Two layers: the **background** is the activity visible at rest, chosen by rest priorities or by the
//! user's swipe (a manual choice is remembered while it differs from the automatic one); **notices**
//! are shows over the background, for a few seconds or while relevant. They have their own queue and
//! priority, unrelated to the rest priority: e.g. a calendar event is nearly top as a notice and
//! nearly last at rest.
//!
//! Pure logic: time comes in as milliseconds.

use crate::gesture::{Dir, Edges, Pull};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Timers,
    Player,
}

/// The ribbon order is fixed: priorities decide what is on screen but never reorder it, so swiping
/// is predictable. Timers come first: they are the island's "home".
pub const RIBBON: [Activity; 2] = [Activity::Timers, Activity::Player];

impl Activity {
    fn index(self) -> usize {
        RIBBON.iter().position(|&a| a == self).unwrap_or(0)
    }
}

pub const REST_TIMERS: u8 = 10;
pub const REST_PLAYING: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Absent,
    Rest(u8),
    /// In the ribbon, but reachable only by swipe: e.g. a paused player.
    SwipeOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Alert,
    /// After Done the timers stay visible for 1.5 s: it shows the countdown restarted.
    AlertDone,
    Track,
    /// Music was just paused: shows the player didn't vanish, it stopped.
    Paused,
    Sound,
    Microphone,
    /// Screenshot shutter: a short countdown before the shot. Whoever posted it takes the shot.
    Shutter,
    Shot,
    /// The screenshot viewer is open. Not news but a state: it holds while the window is open.
    /// Any notice shows over it and leaves, and the state is back under it.
    Viewing,
    /// Held until the scanner closes; only a reminder and a problem show over it, other news waits.
    Scan,
    Countdown,
    Recording,
    Clip,
    Discarded,
    /// A check for an update is running: held until it ends.
    UpdateChecking,
    /// The check found nothing.
    UpdateDone,
    /// The answer to a button of the update window: postponed or skipped.
    UpdateAnswer,
    Warning,
    Failure,
    /// System notices (`crate::notify`), one per slot: a newer one replaces the shown one.
    LowCharge,
    CriticalCharge,
    /// Plugged in, unplugged, charged.
    Power,
    SlowCharge,
    /// Fast drain or high load.
    Usage,
    /// A download is coming: a state, held until nothing comes any more.
    Downloading,
    Download,
    /// Devices (`crate::devices`): connected with the picture and the battery, left, low.
    DeviceConnected,
    DeviceDisconnected,
    DeviceLow,
    /// Windows moved the sound or the microphone to another device.
    AudioSwitch,
}

impl NoticeKind {
    /// Whose news it is. `None` = a system indicator: it belongs to no activity and shows over whichever is on screen.
    pub fn activity(self) -> Option<Activity> {
        match self {
            Self::Alert | Self::AlertDone => Some(Activity::Timers),
            Self::Track | Self::Paused => Some(Activity::Player),
            Self::Sound
            | Self::Microphone
            | Self::Shutter
            | Self::Shot
            | Self::Viewing
            | Self::Scan
            | Self::Countdown
            | Self::Recording
            | Self::Clip
            | Self::Discarded
            | Self::UpdateChecking
            | Self::UpdateDone
            | Self::UpdateAnswer
            | Self::Warning
            | Self::Failure
            | Self::LowCharge
            | Self::CriticalCharge
            | Self::Power
            | Self::SlowCharge
            | Self::Usage
            | Self::Downloading
            | Self::Download
            | Self::DeviceConnected
            | Self::DeviceDisconnected
            | Self::DeviceLow
            | Self::AudioSwitch => None,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::Alert => 100,
            // Right after a reminder: the computer is about to go off.
            Self::CriticalCharge => 95,
            Self::AlertDone => 90,
            // A problem comes before the card it is about: "stopped, disk full", then the file.
            Self::Failure => 86,
            Self::Warning => 85,
            // The user works with it right now; a problem still comes first.
            Self::Scan => 84,
            // A screenshot answers the button just pressed: more important than news.
            Self::Shutter | Self::Shot | Self::Countdown | Self::Clip => 80,
            Self::Sound | Self::Microphone | Self::Discarded => 70,
            // Answers the cable just plugged in, like the sound indicator.
            Self::AudioSwitch => 70,
            // Answers the plug just pulled, like the sound indicator.
            Self::Power => 70,
            Self::LowCharge | Self::DeviceLow => 62,
            Self::SlowCharge => 61,
            Self::UpdateAnswer => 75,
            Self::UpdateDone => 65,
            Self::UpdateChecking => 35,
            Self::Track
            | Self::Usage
            | Self::Download
            | Self::DeviceConnected
            | Self::DeviceDisconnected => 60,
            Self::Paused => 40,
            // A state ranks lowest: news briefly sits over it.
            Self::Viewing => 10,
            // Over the viewer status, under a recording: three states, one place.
            Self::Downloading => 11,
            // Recording hides the viewer status while it runs: both can't fit.
            Self::Recording => 12,
        }
    }

    /// How long a notice stays on screen. `None` = until the source removes it.
    fn duration_ms(self) -> Option<u64> {
        match self {
            Self::Alert
            | Self::Shutter
            | Self::Viewing
            | Self::Scan
            | Self::Countdown
            | Self::Recording
            | Self::UpdateChecking
            | Self::Downloading => None,
            Self::UpdateDone => Some(3000),
            Self::UpdateAnswer => Some(5000),
            Self::AlertDone => Some(1500),
            Self::Shot | Self::Clip => Some(6000),
            Self::Sound | Self::Microphone | Self::Discarded => Some(2000),
            Self::Warning => Some(3000),
            Self::Failure => Some(5000),
            Self::Track => Some(5000),
            Self::Paused => Some(4000),
            // Each holds 1-1.5 s after its battery's or gauge's animation ends (`view/system.rs`).
            Self::LowCharge => Some(4000),
            Self::CriticalCharge => Some(6000),
            Self::Power => Some(4000),
            Self::SlowCharge => Some(5000),
            Self::Usage => Some(7000),
            Self::Download => Some(4000),
            // The connection's animation takes ~2 s of it (`view/device.rs`).
            Self::DeviceConnected => Some(5000),
            Self::DeviceDisconnected | Self::AudioSwitch => Some(2500),
            Self::DeviceLow => Some(3000),
        }
    }

    /// How long a notice may wait in the queue. Longer is stale: a track announce overtaken by a reminder
    /// is useless after the break.
    fn stale_ms(self) -> Option<u64> {
        match self {
            // A screenshot waits to be shown: its fate can only be decided on the card.
            Self::Alert
            | Self::Shutter
            | Self::Shot
            | Self::Viewing
            | Self::Scan
            | Self::Countdown
            | Self::Recording
            | Self::Clip
            // A problem is not news that goes stale: it waits until it can be shown.
            | Self::UpdateAnswer
            | Self::Warning
            | Self::Failure
            // The charge is still that low when the way is clear.
            | Self::CriticalCharge
            // Still coming when the way is clear.
            | Self::Downloading => None,
            // Nobody wants "checking" or "up to date" after the moment has passed.
            Self::UpdateChecking => Some(500),
            Self::UpdateDone => Some(2000),
            Self::AlertDone | Self::Paused | Self::Sound | Self::Microphone | Self::Discarded => {
                Some(1000)
            }
            Self::Track | Self::LowCharge | Self::SlowCharge | Self::Usage => Some(2000),
            Self::Power | Self::DeviceDisconnected | Self::AudioSwitch => Some(1000),
            Self::Download => Some(5000),
            Self::DeviceConnected | Self::DeviceLow => Some(2000),
        }
    }

    /// A priority notice breaks through the island being hidden from the tray,
    /// as Windows priority notifications break through Do Not Disturb.
    pub fn is_priority(self) -> bool {
        matches!(self, Self::Alert | Self::Failure | Self::CriticalCharge)
    }

    /// Shown over a fullscreen app whatever the fullscreen setting says.
    pub fn breaks_fullscreen(self) -> bool {
        self == Self::CriticalCharge
    }

    pub fn is_problem(self) -> bool {
        matches!(self, Self::Warning | Self::Failure)
    }

    /// Passes the island's Do Not Disturb: a priority notice and replies to the user's own actions
    /// (Done, sound muted by key). Self-arriving news (track announce, player pause) doesn't pass.
    fn passes_quiet(self) -> bool {
        !matches!(
            self,
            Self::Track
                | Self::Paused
                | Self::UpdateChecking
                | Self::UpdateDone
                | Self::LowCharge
                | Self::Power
                | Self::SlowCharge
                | Self::Usage
                | Self::Downloading
                | Self::Download
                | Self::DeviceConnected
                | Self::DeviceDisconnected
                | Self::DeviceLow
                | Self::AudioSwitch
        )
    }

    /// A reminder can't be swiped away: it is closed by Done or by time. Same for a screenshot:
    /// while the card is visible, gestures aren't needed.
    fn swipeable(self) -> bool {
        !matches!(
            self,
            Self::Alert | Self::Shutter | Self::Shot | Self::Countdown | Self::Clip
        )
    }

    /// A state held by the island: it can't be swiped away or paged through,
    /// but the control center opens over it.
    fn fixed(self) -> bool {
        matches!(self, Self::Viewing | Self::Scan | Self::Recording)
    }

    /// A swipe only takes it away. Reminders and track news page with their activity, so they are excluded.
    fn dismissable(self) -> bool {
        self.activity().is_none() && self.swipeable() && !self.fixed()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownNotice {
    /// Same id = same show: an extension doesn't restart the appear animation.
    pub id: u64,
    pub kind: NoticeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub activity: Activity,
    pub notice: Option<ShownNotice>,
    pub menu: bool,
}

#[derive(Debug)]
struct Notice {
    id: u64,
    kind: NoticeKind,
    posted: u64,
    /// When it first reached the screen. The deadline keeps running even if it is covered.
    shown_at: Option<u64>,
}

pub struct Stage {
    presence: [Presence; RIBBON.len()],
    manual: Option<Activity>,
    notices: Vec<Notice>,
    held: bool,
    menu: bool,
    quiet: bool,
    next_id: u64,
    shown: Shown,
}

impl Default for Stage {
    fn default() -> Self {
        Self::new()
    }
}

impl Stage {
    pub fn new() -> Self {
        let mut presence = [Presence::Absent; RIBBON.len()];
        presence[Activity::Timers.index()] = Presence::Rest(REST_TIMERS);
        Self {
            presence,
            manual: None,
            notices: Vec::new(),
            held: false,
            menu: false,
            quiet: false,
            next_id: 1,
            shown: Shown {
                activity: Activity::Timers,
                notice: None,
                menu: false,
            },
        }
    }

    pub fn shown(&self) -> Shown {
        self.shown
    }

    pub fn presence(&self, activity: Activity) -> Presence {
        self.presence[activity.index()]
    }

    /// All state-changing methods return true if what is on screen changed.
    pub fn set_presence(&mut self, activity: Activity, presence: Presence, now: u64) -> bool {
        self.presence[activity.index()] = presence;
        if presence == Presence::Absent {
            self.notices.retain(|n| n.kind.activity() != Some(activity));
        }
        self.settle(now)
    }

    /// A new notice. If one already exists it is extended, not shown again.
    pub fn post(&mut self, kind: NoticeKind, now: u64) -> bool {
        if kind.activity().is_some_and(|a| !self.present(a)) {
            return false;
        }
        // In quiet mode such news doesn't wait in the queue: it is no longer needed afterwards.
        if self.quiet && !kind.passes_quiet() {
            return false;
        }
        match self.notices.iter_mut().find(|n| n.kind == kind) {
            Some(n) if n.shown_at.is_some() => n.shown_at = Some(now),
            Some(n) => n.posted = now,
            None => {
                self.notices.push(Notice {
                    id: self.next_id,
                    kind,
                    posted: now,
                    shown_at: None,
                });
                self.next_id += 1;
            }
        }
        self.settle(now)
    }

    pub fn end(&mut self, kind: NoticeKind, now: u64) -> bool {
        self.notices.retain(|n| n.kind != kind);
        self.settle(now)
    }

    pub fn has(&self, kind: NoticeKind) -> bool {
        self.notices.iter().any(|n| n.kind == kind)
    }

    pub fn hold(&mut self, held: bool, now: u64) -> bool {
        self.held = held;
        self.settle(now)
    }

    pub fn tick(&mut self, now: u64) -> bool {
        self.settle(now)
    }

    /// The control center opens over everything except a reminder.
    /// While it is open, non-priority notices wait in the queue.
    pub fn open_menu(&mut self, now: u64) -> bool {
        if !self.can_swipe() {
            return false;
        }
        // Whoever reaches for the menu has seen the problem: it must not push the menu away.
        self.notices.retain(|n| !n.kind.is_problem());
        self.menu = true;
        self.settle(now)
    }

    pub fn close_menu(&mut self, now: u64) -> bool {
        self.menu = false;
        self.settle(now)
    }

    /// Do Not Disturb: turning it on removes a shown self-arriving news item.
    pub fn set_quiet(&mut self, quiet: bool, now: u64) -> bool {
        self.quiet = quiet;
        if quiet {
            self.notices.retain(|n| n.kind.passes_quiet());
        }
        self.settle(now)
    }

    pub fn can_swipe(&self) -> bool {
        self.shown.notice.is_none_or(|n| n.kind.swipeable())
    }

    pub fn neighbor(&self, dir: Dir) -> Option<Activity> {
        let i = self.shown.activity.index();
        let present = |a: &&Activity| self.presence(**a) != Presence::Absent;
        match dir {
            Dir::Prev => RIBBON[..i].iter().rev().find(present).copied(),
            Dir::Next => RIBBON[i + 1..].iter().find(present).copied(),
        }
    }

    pub fn ribbon_position(&self) -> (usize, usize) {
        let present: Vec<Activity> = RIBBON.into_iter().filter(|&a| self.present(a)).collect();
        let current = present
            .iter()
            .position(|&a| a == self.shown.activity)
            .unwrap_or(0);
        (present.len(), current)
    }

    /// Where one can swipe from here. In the open menu only up, to close it:
    /// the menu isn't a ribbon activity, so paging from it isn't allowed.
    pub fn edges(&self) -> Edges {
        if !self.can_swipe() {
            return Edges::default();
        }
        if self.menu {
            return Edges {
                pull: Some(Pull::Close),
                ..Edges::default()
            };
        }
        if self.shows_dismissable() {
            return Edges {
                dismiss: true,
                ..Edges::default()
            };
        }
        if self.shows_fixed() {
            return Edges {
                pull: Some(Pull::Open),
                ..Edges::default()
            };
        }
        Edges {
            prev: self.neighbor(Dir::Prev).is_some(),
            next: self.neighbor(Dir::Next).is_some(),
            pull: Some(Pull::Open),
            dismiss: false,
        }
    }

    pub fn dismiss(&mut self, now: u64) -> bool {
        match self.shown.notice {
            Some(shown) if self.shows_dismissable() => {
                self.notices.retain(|n| n.id != shown.id);
                self.settle(now)
            }
            _ => false,
        }
    }

    /// Swipe to a neighbour. The neighbour becomes the manual background choice; a shown notice is swiped away.
    pub fn step(&mut self, dir: Dir, now: u64) -> bool {
        if self.shows_dismissable() {
            return self.dismiss(now);
        }
        if !self.can_swipe() || self.menu || self.shows_fixed() {
            return false;
        }
        let Some(target) = self.neighbor(dir) else {
            return false;
        };
        if let Some(shown) = self.shown.notice {
            self.notices.retain(|n| n.id != shown.id);
        }
        self.manual = Some(target);
        self.settle(now)
    }

    pub fn next_wake(&self) -> Option<u64> {
        let top = self.shown.notice.map(|n| n.id);
        self.notices
            .iter()
            .filter_map(|n| match n.shown_at {
                Some(_) if self.held && Some(n.id) == top => None,
                Some(at) => n.kind.duration_ms().map(|d| at + d),
                None => n.kind.stale_ms().map(|s| n.posted + s),
            })
            .min()
    }

    fn shows_dismissable(&self) -> bool {
        !self.menu && self.shown.notice.is_some_and(|n| n.kind.dismissable())
    }

    fn shows_fixed(&self) -> bool {
        self.shown.notice.is_some_and(|n| n.kind.fixed())
    }

    fn present(&self, activity: Activity) -> bool {
        self.presence(activity) != Presence::Absent
    }

    /// Background without a manual choice: the highest rest priority, on a tie the one further left in the ribbon.
    fn auto_rest(&self) -> Activity {
        let mut best = (Activity::Timers, 0u8);
        for &a in &RIBBON {
            if let Presence::Rest(p) = self.presence(a)
                && p > best.1
            {
                best = (a, p);
            }
        }
        best.0
    }

    fn settle(&mut self, now: u64) -> bool {
        let top = self.shown.notice.map(|n| n.id);
        let held = self.held;
        self.notices.retain(|n| match n.shown_at {
            Some(_) if held && Some(n.id) == top => true,
            Some(at) => n.kind.duration_ms().is_none_or(|d| now < at + d),
            None => n.kind.stale_ms().is_none_or(|s| now < n.posted + s),
        });

        // A manual choice lives while it changes anything: if it matches the automatic one, forget it.
        if let Some(m) = self.manual
            && (!self.present(m) || m == self.auto_rest())
        {
            self.manual = None;
        }

        let background = self.manual.unwrap_or_else(|| self.auto_rest());
        let presence = self.presence;
        let menu = self.menu;
        // While the menu is open only a priority notice can break through.
        let visible = |n: &&mut Notice| {
            n.kind
                .activity()
                .is_none_or(|a| presence[a.index()] != Presence::Absent)
                && (!menu || n.kind.is_priority())
        };
        let top = self.notices.iter_mut().filter(visible).max_by(|a, b| {
            a.kind
                .priority()
                .cmp(&b.kind.priority())
                .then(b.posted.cmp(&a.posted))
        });
        let shown = match top {
            Some(n) => {
                n.shown_at.get_or_insert(now);
                if n.kind.is_priority() {
                    self.menu = false;
                }
                Shown {
                    activity: n.kind.activity().unwrap_or(background),
                    notice: Some(ShownNotice {
                        id: n.id,
                        kind: n.kind,
                    }),
                    menu: self.menu,
                }
            }
            None => Shown {
                activity: background,
                notice: None,
                menu: self.menu,
            },
        };
        let changed = shown != self.shown;
        self.shown = shown;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Activity::*;
    use NoticeKind::*;

    fn kind(s: &Stage) -> Option<NoticeKind> {
        s.shown().notice.map(|n| n.kind)
    }

    fn playing(s: &mut Stage, now: u64) {
        s.set_presence(Player, Presence::Rest(REST_PLAYING), now);
    }

    #[test]
    fn timers_are_home() {
        let s = Stage::new();
        assert_eq!(s.shown().activity, Timers);
        assert_eq!(
            s.edges(),
            Edges {
                pull: Some(Pull::Open),
                ..Edges::default()
            }
        );
        assert_eq!(s.ribbon_position(), (1, 0));
    }

    #[test]
    fn open_menu_only_closes_and_does_not_swipe() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert!(s.open_menu(0));
        assert!(s.shown().menu);
        assert_eq!(
            s.edges(),
            Edges {
                pull: Some(Pull::Close),
                ..Edges::default()
            }
        );
        assert!(!s.step(Dir::Prev, 10));
        assert!(s.close_menu(20));
        assert_eq!((s.shown().activity, s.shown().menu), (Player, false));
    }

    #[test]
    fn track_waits_while_menu_is_open() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.open_menu(0);
        s.post(Track, 100);
        assert_eq!(kind(&s), None);
        s.close_menu(1000);
        assert_eq!(kind(&s), Some(Track));
    }

    #[test]
    fn indicator_shows_over_the_current_activity() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert!(s.post(Sound, 100));
        assert_eq!((s.shown().activity, kind(&s)), (Player, Some(Sound)));
        s.tick(2100);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn quiet_drops_news_but_not_answers_to_the_user() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        assert!(s.set_quiet(true, 10));
        assert_eq!(kind(&s), None, "показанный анонс снят");
        assert!(!s.post(Track, 20));
        assert!(!s.post(Paused, 30));
        assert!(s.post(Microphone, 40));
        assert_eq!(kind(&s), Some(Microphone));
        s.post(Alert, 50);
        assert_eq!(kind(&s), Some(Alert));
        s.set_quiet(false, 60);
        s.end(Alert, 70);
        assert!(s.post(Track, 3000));
    }

    #[test]
    fn screenshot_card_waits_for_the_alert_and_then_lasts() {
        let mut s = Stage::new();
        s.set_quiet(true, 0);
        assert!(s.post(Shutter, 0), "проходит «Не беспокоить»");
        assert!(!s.can_swipe());
        s.post(Alert, 100);
        s.end(Shutter, 500);
        s.post(Shot, 500);
        assert_eq!(kind(&s), Some(Alert));
        s.tick(60_000);
        assert!(s.has(Shot), "карточка не устаревает в очереди");
        s.end(Alert, 60_000);
        assert_eq!(kind(&s), Some(Shot));
        s.tick(66_000);
        assert!(!s.has(Shot));
    }

    #[test]
    fn viewing_stays_under_news_and_cannot_be_swiped_away() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert!(s.post(Viewing, 0));
        assert_eq!(kind(&s), Some(Viewing));
        assert_eq!(s.next_wake(), None, "держится, пока окно открыто");
        assert_eq!(
            s.edges(),
            Edges {
                pull: Some(Pull::Open),
                ..Edges::default()
            },
            "ленту не листать, меню — можно"
        );
        assert!(!s.step(Dir::Prev, 10));

        s.post(Track, 100);
        assert_eq!(kind(&s), Some(Track), "новость поверх");
        s.tick(5100);
        assert_eq!(kind(&s), Some(Viewing), "и снова состояние");

        assert!(s.open_menu(6000));
        assert!(s.shown().menu);
        s.close_menu(7000);
        assert_eq!(kind(&s), Some(Viewing));
        s.end(Viewing, 8000);
        assert_eq!((s.shown().activity, kind(&s)), (Player, None));
    }

    #[test]
    fn the_scanner_holds_news_back_but_not_a_reminder() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert!(s.post(Scan, 0));
        assert_eq!(s.next_wake(), None, "держится, пока сканер открыт");
        s.post(Track, 100);
        assert_eq!(kind(&s), Some(Scan), "новость ждёт");
        s.post(Alert, 200);
        assert_eq!(kind(&s), Some(Alert), "напоминание поверх");
        s.end(Alert, 300);
        assert_eq!(kind(&s), Some(Scan));
        s.end(Scan, 400);
        assert_eq!(kind(&s), Some(Track), "новость после сканера");
    }

    #[test]
    fn alert_closes_menu_and_menu_does_not_open_over_it() {
        let mut s = Stage::new();
        s.open_menu(0);
        assert!(s.post(Alert, 100));
        assert!(!s.shown().menu);
        assert_eq!(kind(&s), Some(Alert));
        assert!(!s.open_menu(200));
        assert_eq!(s.edges(), Edges::default());
    }

    #[test]
    fn ribbon_position_counts_present_activities() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert_eq!(s.ribbon_position(), (2, 1));
        s.step(Dir::Prev, 10);
        assert_eq!(s.ribbon_position(), (2, 0));
    }

    #[test]
    fn playing_music_takes_rest_and_pause_returns_to_timers() {
        let mut s = Stage::new();
        assert!(s.set_presence(Player, Presence::Rest(REST_PLAYING), 0));
        assert_eq!(s.shown().activity, Player);
        assert!(s.set_presence(Player, Presence::SwipeOnly, 10));
        assert_eq!(s.shown().activity, Timers);
        assert_eq!(s.neighbor(Dir::Next), Some(Player));
    }

    #[test]
    fn track_notice_shows_player_for_five_seconds() {
        let mut s = Stage::new();
        s.set_presence(Player, Presence::SwipeOnly, 0);
        s.post(Track, 0);
        assert_eq!((s.shown().activity, kind(&s)), (Player, Some(Track)));
        assert_eq!(s.next_wake(), Some(5000));
        assert!(!s.tick(4999));
        assert!(s.tick(5000));
        assert_eq!((s.shown().activity, kind(&s)), (Timers, None));
    }

    #[test]
    fn repost_extends_the_same_notice() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        let id = s.shown().notice.unwrap().id;
        s.post(Track, 2000);
        assert_eq!(s.shown().notice.unwrap().id, id);
        s.tick(6000);
        assert_eq!(kind(&s), Some(Track));
        s.tick(7000);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn alert_overrides_everything_and_cannot_be_swiped() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Alert, 0);
        assert_eq!((s.shown().activity, kind(&s)), (Timers, Some(Alert)));
        assert!(!s.can_swipe());
        assert!(!s.step(Dir::Next, 10));

        s.end(Alert, 30_000);
        s.post(AlertDone, 30_000);
        assert_eq!(kind(&s), Some(AlertDone));
        s.tick(31_500);
        assert_eq!((s.shown().activity, kind(&s)), (Player, None));
    }

    #[test]
    fn track_waiting_behind_alert_goes_stale() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Alert, 0);
        s.post(Track, 1000);
        assert_eq!(kind(&s), Some(Alert));
        s.tick(10_000);
        s.end(Alert, 20_000);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn track_preempted_by_alert_expires_meanwhile() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        s.post(Alert, 1000);
        s.end(Alert, 6000);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn pointer_on_island_keeps_notice_open() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        s.hold(true, 1000);
        assert_eq!(s.next_wake(), None);
        s.tick(10_000);
        assert_eq!(kind(&s), Some(Track));
        assert!(s.hold(false, 11_000));
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn manual_choice_is_remembered_while_it_differs_from_auto() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        assert!(s.step(Dir::Prev, 0));
        assert_eq!(s.shown().activity, Timers);

        s.post(Track, 100);
        assert_eq!(s.shown().activity, Player);
        s.tick(5100);
        assert_eq!(s.shown().activity, Timers);

        // Music stopped: the automatic choice would also pick timers, so the manual choice is forgotten.
        s.set_presence(Player, Presence::SwipeOnly, 4000);
        playing(&mut s, 5000);
        assert_eq!(s.shown().activity, Player);
    }

    #[test]
    fn swipe_dismisses_shown_notice() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        assert_eq!(kind(&s), Some(Track));
        s.step(Dir::Prev, 10);
        assert_eq!((s.shown().activity, kind(&s)), (Timers, None));
    }

    #[test]
    fn absent_activity_loses_notices_and_manual_choice() {
        let mut s = Stage::new();
        s.set_presence(Player, Presence::SwipeOnly, 0);
        s.step(Dir::Next, 0);
        s.post(Paused, 0);
        s.set_presence(Player, Presence::Absent, 10);
        assert_eq!((s.shown().activity, kind(&s)), (Timers, None));
        assert!(!s.post(Track, 20));
    }

    #[test]
    fn higher_priority_waits_for_nothing() {
        let mut s = Stage::new();
        s.set_presence(Player, Presence::SwipeOnly, 0);
        s.post(Paused, 0);
        s.post(Track, 100);
        assert_eq!(kind(&s), Some(Track));
        s.tick(5100);
        assert_eq!(kind(&s), None, "пауза за это время устарела");
    }

    #[test]
    fn recording_stays_under_news_and_opens_the_menu() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Recording, 0);
        assert_eq!(kind(&s), Some(Recording));
        assert_eq!(s.next_wake(), None, "запись держится, пока её не остановят");
        assert!(!s.step(Dir::Next, 10));
        assert!(s.open_menu(20));
        s.close_menu(30);
        s.post(Alert, 40);
        assert_eq!(kind(&s), Some(Alert));
        s.end(Alert, 50);
        assert_eq!(kind(&s), Some(Recording));
    }

    #[test]
    fn problem_comes_before_the_card_and_the_card_follows() {
        let mut s = Stage::new();
        s.post(Recording, 0);
        assert!(s.post(Warning, 100));
        s.post(Clip, 100);
        assert_eq!(kind(&s), Some(Warning));
        s.tick(3100);
        // The card counts its six seconds from the moment it is seen.
        assert_eq!(kind(&s), Some(Clip));
        s.tick(9000);
        assert_eq!(kind(&s), Some(Clip));
        s.tick(9200);
        assert_eq!(kind(&s), Some(Recording));
    }

    #[test]
    fn problem_waits_for_the_alert_and_is_not_lost() {
        let mut s = Stage::new();
        s.post(Alert, 0);
        s.post(Failure, 100);
        assert_eq!(kind(&s), Some(Alert));
        s.end(Alert, 60_000);
        assert_eq!(kind(&s), Some(Failure));
        s.tick(64_900);
        assert_eq!(kind(&s), Some(Failure));
        s.tick(65_100);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn failure_closes_the_menu_and_a_warning_waits_for_it() {
        let mut s = Stage::new();
        s.open_menu(0);
        assert!(!s.post(Warning, 100));
        assert!(s.shown().menu);
        assert!(s.post(Failure, 200));
        assert!(!s.shown().menu);
        assert_eq!(kind(&s), Some(Failure));
        assert!(Failure.is_priority() && !Warning.is_priority());
    }

    #[test]
    fn opening_the_menu_dismisses_a_problem() {
        let mut s = Stage::new();
        s.post(Failure, 0);
        assert!(s.open_menu(100));
        assert!(s.shown().menu);
        assert!(!s.has(Failure));
    }

    #[test]
    fn hovered_problem_stays_and_a_swipe_dismisses_it() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Warning, 0);
        s.hold(true, 100);
        s.tick(60_000);
        assert_eq!(kind(&s), Some(Warning));
        s.hold(false, 60_000);
        assert_eq!(kind(&s), None);
        s.post(Failure, 61_000);
        assert!(s.can_swipe());
        s.step(Dir::Prev, 61_100);
        assert!(!s.has(Failure));
    }

    #[test]
    fn recording_hides_the_viewer_status_while_it_runs() {
        let mut s = Stage::new();
        s.post(Viewing, 0);
        s.post(Recording, 10);
        assert_eq!(kind(&s), Some(Recording));
        s.end(Recording, 20);
        assert_eq!(kind(&s), Some(Viewing));
    }

    #[test]
    fn clip_card_shows_over_recording_for_six_seconds() {
        let mut s = Stage::new();
        s.post(Recording, 0);
        s.post(Clip, 100);
        assert_eq!(kind(&s), Some(Clip));
        assert!(!s.can_swipe());
        s.tick(6100);
        assert_eq!(kind(&s), Some(Recording));
    }

    #[test]
    fn the_critical_charge_comes_right_after_a_reminder_and_through_everything() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Track, 0);
        s.post(Usage, 0);
        s.post(CriticalCharge, 10);
        assert_eq!(kind(&s), Some(CriticalCharge));
        s.post(Alert, 20);
        assert_eq!(kind(&s), Some(Alert));

        let mut quiet = Stage::new();
        quiet.set_quiet(true, 0);
        for k in [LowCharge, Power, SlowCharge, Usage] {
            assert!(!quiet.post(k, 0), "{k:?}");
        }
        quiet.open_menu(0);
        quiet.post(CriticalCharge, 0);
        assert_eq!(kind(&quiet), Some(CriticalCharge));
        assert!(!quiet.shown().menu);
    }

    #[test]
    fn a_swipe_over_a_notice_only_takes_it_away() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Downloading, 0);
        s.post(Usage, 0);
        assert_eq!(
            s.edges(),
            Edges {
                dismiss: true,
                ..Edges::default()
            },
            "ни листать, ни открыть центр управления"
        );
        assert!(s.dismiss(100));
        assert_eq!((s.shown().activity, kind(&s)), (Player, Some(Downloading)));
        assert!(s.step(Dir::Prev, 200), "шаг тоже только убирает");
        assert_eq!((s.shown().activity, kind(&s)), (Player, None));

        s.post(Track, 300);
        assert!(!s.edges().dismiss, "анонс трека листается с плеером");
        assert!(!s.dismiss(400));
    }

    #[test]
    fn a_coming_download_holds_under_news_and_a_recording() {
        let mut s = Stage::new();
        playing(&mut s, 0);
        s.post(Viewing, 0);
        s.post(Downloading, 0);
        assert_eq!(kind(&s), Some(Downloading), "над окном просмотра");
        assert_eq!(s.next_wake(), None, "держится, пока идёт загрузка");
        s.post(Download, 100);
        assert_eq!(kind(&s), Some(Download));
        s.post(Download, 3_000);
        s.tick(6_000);
        assert_eq!(kind(&s), Some(Download), "новый файл продлевает показ");
        s.tick(7_000);
        assert_eq!(kind(&s), Some(Downloading));
        s.post(Recording, 7_000);
        assert_eq!(kind(&s), Some(Recording), "запись важнее");
        s.end(Recording, 8_000);
        assert!(s.can_swipe());
        s.step(Dir::Prev, 8_100);
        assert!(!s.has(Downloading), "смахивается");

        let mut quiet = Stage::new();
        quiet.set_quiet(true, 0);
        assert!(!quiet.post(Downloading, 0));
        assert!(!quiet.post(Download, 0));
    }

    #[test]
    fn device_news_is_quiet_in_do_not_disturb_and_holds_through_its_animation() {
        let mut quiet = Stage::new();
        quiet.set_quiet(true, 0);
        for k in [DeviceConnected, DeviceDisconnected, DeviceLow, AudioSwitch] {
            assert!(!quiet.post(k, 0), "{k:?}");
        }
        let mut s = Stage::new();
        s.post(DeviceConnected, 0);
        s.tick(4_900);
        assert_eq!(kind(&s), Some(DeviceConnected));
        s.tick(5_100);
        assert_eq!(kind(&s), None);
    }

    #[test]
    fn a_newer_system_notice_takes_the_place_of_the_shown_one() {
        let mut s = Stage::new();
        s.post(CriticalCharge, 0);
        let id = s.shown().notice.map(|n| n.id);
        s.post(CriticalCharge, 4_000);
        assert_eq!(s.shown().notice.map(|n| n.id), id);
        s.tick(9_000);
        assert_eq!(kind(&s), Some(CriticalCharge));
        s.tick(10_100);
        assert_eq!(kind(&s), None);
    }
}
