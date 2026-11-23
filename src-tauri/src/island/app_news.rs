//! What apps show outside their pages (the layer is `crate::apps`; the apps themselves are
//! `apps.rs`).
//!
//! 1. A notice: urgent (`NoticeKind::AppUrgent`: it passes Do Not Disturb and goes before a
//!    reminder) or ordinary (`AppNews`). A newer one of the same kind takes the shown one's
//!    place. Its sound plays only if the notice got onto the stage; one marked
//!    `over_fullscreen` shows over a fullscreen app whatever the island's fullscreen setting.
//! 2. "Service connected": the island's own notice when the user turns a connector on.
//! 3. While an app has something going on, it is a page of the ribbon (`Activity::App`), reached
//!    by a swipe; a press on it opens the app's page. Its badge is a grey sign on the collapsed
//!    island, next to the moon.

use super::App;
use crate::{
    apps::{Chime, Icon, Manifest, Notice, Ribbon, ui::Symbol, ui::Tint},
    lang::tr,
    sound,
    stage::{Activity, NoticeKind, Presence, Sheet},
};

#[derive(Default)]
pub(super) struct AppNews {
    urgent: Option<Notice>,
    news: Option<Notice>,
}

impl AppNews {
    fn slot(&mut self, kind: NoticeKind) -> &mut Option<Notice> {
        if kind == NoticeKind::AppUrgent {
            &mut self.urgent
        } else {
            &mut self.news
        }
    }
}

fn kind_of(notice: &Notice) -> NoticeKind {
    if notice.urgent {
        NoticeKind::AppUrgent
    } else {
        NoticeKind::AppNews
    }
}

fn cue(chime: Chime) -> sound::Cue {
    match chime {
        Chime::Alarm => sound::Cue::Alarm,
        Chime::Caution => sound::Cue::Caution,
        Chime::Clear => sound::Cue::Clear,
    }
}

impl App {
    pub(super) fn show_app_notice(&mut self, notice: Notice) {
        let kind = kind_of(&notice);
        let chime = notice.sound;
        *self.app_news.slot(kind) = Some(notice);
        let now = self.now_ms();
        self.stage.post(kind, now);
        if self.stage.has(kind)
            && let Some(chime) = chime
        {
            sound::cue(cue(chime), self.settings.volume);
        }
        // A newer notice of a shown kind keeps the stage as it is: the content still changed.
        self.after_stage(true);
    }

    pub(super) fn show_connected(&mut self, manifest: &Manifest) {
        self.show_app_notice(Notice {
            symbol: Symbol::Check,
            tint: Tint::Clear,
            title: tr!("apps.connected").into(),
            text: manifest.title.clone(),
            sound: None,
            urgent: false,
            over_fullscreen: false,
        });
    }

    pub(super) fn app_notice(&self) -> Option<&Notice> {
        match self.stage.shown().notice?.kind {
            NoticeKind::AppUrgent => self.app_news.urgent.as_ref(),
            NoticeKind::AppNews => self.app_news.news.as_ref(),
            _ => None,
        }
    }

    pub(super) fn app_notice_over_fullscreen(&self) -> bool {
        self.app_notice().is_some_and(|n| n.over_fullscreen)
    }

    /// After the stage changed: a notice that left it is forgotten.
    pub(super) fn sync_app_news(&mut self) {
        for kind in [NoticeKind::AppUrgent, NoticeKind::AppNews] {
            if !self.stage.has(kind) {
                *self.app_news.slot(kind) = None;
            }
        }
    }

    /// The first app with a page of the ribbon, by its place in the list.
    pub(super) fn ribbon(&self) -> Option<(usize, &Ribbon)> {
        self.apps
            .list
            .iter()
            .enumerate()
            .find_map(|(i, s)| Some((i, s.live.as_ref()?.ribbon.as_ref()?)))
    }

    pub(super) fn app_badge(&self) -> Option<Icon> {
        self.apps.list.iter().find_map(|s| s.live.as_ref()?.badge)
    }

    pub(super) fn sync_live(&mut self) {
        let presence = if self.ribbon().is_some() {
            Presence::SwipeOnly
        } else {
            Presence::Absent
        };
        if self.stage.presence(Activity::App) != presence {
            let now = self.now_ms();
            let changed = self.stage.set_presence(Activity::App, presence, now);
            self.after_stage(changed);
        }
    }

    pub(super) fn open_ribbon_app(&mut self) {
        let Some((i, _)) = self.ribbon() else {
            return;
        };
        let now = self.now_ms();
        let changed = self.stage.open_sheet(Sheet::Apps, now);
        self.after_stage(changed);
        if self.stage.shown().sheet == Some(Sheet::Apps) {
            self.launch_app(i);
            self.apps.from_ribbon = true;
        }
    }
}
