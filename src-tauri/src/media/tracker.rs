//! Which player to show and when to announce it. Pure logic, time comes from outside.
//!
//! The session with the latest event (started playing or changed track) is shown. If video and music
//! are both playing, tracks change more often: the music is shown; if the video changed, the video is
//! shown until a track changes.
//!
//! A session that played less than 0.5 s doesn't count: so short previews don't take the island and
//! don't linger as "paused". A track is announced after the same 0.5 s, so fast skipping announces
//! only the last one.
//!
//! Players report the position now and then, each time a little off from the one before; a report
//! within 0.5 s of the shown position is ignored, so the progress doesn't twitch. A seek from the island
//! moves the position at once, and for 0.5 s the player's reports don't move it back.

use super::{Session, Update};
use crate::stage::{Presence, REST_PLAYING};

pub const CONFIRM_MS: u64 = 500;
/// The track was switched from the island itself: it is already visible, no announce needed.
const OWN_COMMAND_MS: u64 = 3000;
pub const PAUSED_LIFETIME_MS: u64 = 10 * 60_000;
const POSITION_TOLERANCE_MS: i64 = 500;
/// Right after a seek the player may still report the old position.
const SEEK_HOLD_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerEvent {
    Announce,
    Paused,
}

struct Entry {
    session: Session,
    confirmed: bool,
    /// "Playing" but silent for long (e.g. a video without sound in a background tab).
    /// Dropped from the ribbon until the next event.
    silent: bool,
    last_event: u64,
    paused_at: Option<u64>,
    pending: Option<Pending>,
    quiet_until: u64,
    hold_until: u64,
}

#[derive(Clone, Copy)]
struct Pending {
    at: u64,
    announce: bool,
}

#[derive(Default)]
pub struct Tracker {
    entries: Vec<Entry>,
    current: Option<String>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&Session> {
        self.current_entry().map(|e| &e.session)
    }

    pub fn presence(&self, now: u64) -> Presence {
        match self.current_entry() {
            Some(e) if e.session.is_playing() => Presence::Rest(REST_PLAYING),
            Some(e) if e.paused_at.is_none_or(|at| now < at + PAUSED_LIFETIME_MS) => {
                Presence::SwipeOnly
            }
            _ => Presence::Absent,
        }
    }

    pub fn own_command(&mut self, now: u64) {
        if let Some(key) = self.current.clone()
            && let Some(e) = self.entry_mut(&key)
        {
            e.quiet_until = now + OWN_COMMAND_MS;
            if let Some(p) = &mut e.pending {
                p.announce = false;
            }
        }
    }

    /// `wall_ms` is Unix time, the clock of `Timeline`; `now` is the tracker's clock.
    pub fn seek(&mut self, position_ms: i64, wall_ms: u64, now: u64) {
        if let Some(key) = self.current.clone()
            && let Some(e) = self.entry_mut(&key)
            && let Some(t) = &mut e.session.timeline
        {
            t.position_ms = position_ms.clamp(0, t.duration_ms);
            t.updated_at_ms = wall_ms;
            e.hold_until = now + SEEK_HOLD_MS;
        }
    }

    pub fn mark_silent(&mut self) {
        if let Some(key) = self.current.clone()
            && let Some(e) = self.entry_mut(&key)
        {
            e.silent = true;
            self.current = self.choose();
        }
    }

    pub fn apply(&mut self, update: Update, now: u64) -> Vec<PlayerEvent> {
        let was_playing = self.current().is_some_and(Session::is_playing);
        match update {
            Update::Changed(session) => self.changed(session, now),
            Update::Gone(key) => self.entries.retain(|e| e.session.key != key),
        }
        let mut events = self.tick(now);
        if was_playing && self.current().is_some_and(|s| !s.is_playing()) {
            events.push(PlayerEvent::Paused);
        }
        events
    }

    pub fn tick(&mut self, now: u64) -> Vec<PlayerEvent> {
        let mut announce = None;
        for e in &mut self.entries {
            if let Some(p) = e.pending
                && now >= p.at
            {
                e.pending = None;
                e.confirmed = true;
                if p.announce {
                    announce = Some(e.session.key.clone());
                }
            }
        }
        self.current = self.choose();
        if announce.is_some() && announce == self.current {
            vec![PlayerEvent::Announce]
        } else {
            Vec::new()
        }
    }

    pub fn next_wake(&self) -> Option<u64> {
        self.entries
            .iter()
            .filter_map(|e| e.pending.map(|p| p.at))
            .min()
    }

    fn changed(&mut self, mut session: Session, now: u64) {
        let Some(e) = self.entry_mut(&session.key) else {
            let playing = session.is_playing();
            self.entries.push(Entry {
                // A player that was already paused when the island started is real.
                confirmed: !playing,
                silent: false,
                last_event: now,
                paused_at: (!playing).then_some(now),
                pending: playing.then_some(Pending {
                    at: now + CONFIRM_MS,
                    announce: true,
                }),
                quiet_until: 0,
                hold_until: 0,
                session,
            });
            return;
        };

        let was_playing = e.session.is_playing();
        let new_track = !e.session.same_track(&session);
        let playing = session.is_playing();
        if !new_track
            && let (Some(shown), Some(reported)) = (e.session.timeline, session.timeline)
            && (now < e.hold_until
                || was_playing && playing && shown.agrees(&reported, POSITION_TOLERANCE_MS))
        {
            session.timeline = Some(shown);
        }
        if playing && (!was_playing || new_track) {
            e.last_event = now;
            e.paused_at = None;
            e.silent = false;
            e.pending = Some(Pending {
                at: now + CONFIRM_MS,
                announce: now >= e.quiet_until,
            });
        } else if !playing {
            e.pending = None;
            if was_playing {
                e.paused_at = Some(now);
            }
        }
        e.session = session;
    }

    fn choose(&self) -> Option<String> {
        let confirmed = || self.entries.iter().filter(|e| e.confirmed && !e.silent);
        confirmed()
            .filter(|e| e.session.is_playing())
            .max_by_key(|e| e.last_event)
            .or_else(|| confirmed().max_by_key(|e| e.paused_at.unwrap_or(e.last_event)))
            .map(|e| e.session.key.clone())
    }

    fn current_entry(&self) -> Option<&Entry> {
        let key = self.current.as_ref()?;
        self.entries.iter().find(|e| &e.session.key == key)
    }

    fn entry_mut(&mut self, key: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.session.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Controls, Status, Timeline};

    fn session(key: &str, title: &str, status: Status) -> Session {
        Session {
            key: key.into(),
            app_id: key.into(),
            title: title.into(),
            artist: "artist".into(),
            album: String::new(),
            video: false,
            status,
            timeline: None,
            controls: Controls::default(),
            cover: None,
            icon: None,
        }
    }

    fn play(t: &mut Tracker, key: &str, title: &str, now: u64) -> Vec<PlayerEvent> {
        t.apply(Update::Changed(session(key, title, Status::Playing)), now)
    }

    fn pause(t: &mut Tracker, key: &str, title: &str, now: u64) -> Vec<PlayerEvent> {
        t.apply(Update::Changed(session(key, title, Status::Paused)), now)
    }

    fn current(t: &Tracker) -> Option<&str> {
        t.current().map(|s| s.key.as_str())
    }

    #[test]
    fn announces_after_a_moment_of_playback() {
        let mut t = Tracker::new();
        assert!(play(&mut t, "spotify", "A", 0).is_empty());
        assert_eq!(current(&t), None);
        assert_eq!(t.next_wake(), Some(CONFIRM_MS));
        assert!(t.tick(CONFIRM_MS - 1).is_empty());
        assert_eq!(t.tick(CONFIRM_MS), vec![PlayerEvent::Announce]);
        assert_eq!(current(&t), Some("spotify"));
        assert!(t.tick(3000).is_empty());
    }

    #[test]
    fn short_preview_is_ignored_completely() {
        let mut t = Tracker::new();
        play(&mut t, "chrome", "preview", 0);
        assert!(pause(&mut t, "chrome", "preview", 300).is_empty());
        assert!(t.tick(2000).is_empty());
        assert_eq!(current(&t), None);
        assert_eq!(t.presence(2000), Presence::Absent);
    }

    #[test]
    fn fast_skipping_announces_only_the_last_track() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(1500);
        play(&mut t, "spotify", "B", 2000);
        play(&mut t, "spotify", "C", 2300);
        assert!(t.tick(2300 + CONFIRM_MS - 1).is_empty());
        assert_eq!(t.tick(2300 + CONFIRM_MS), vec![PlayerEvent::Announce]);
        assert_eq!(t.current().unwrap().title, "C");
    }

    #[test]
    fn latest_event_wins_between_players() {
        let mut t = Tracker::new();
        play(&mut t, "chrome", "video", 0);
        t.tick(1500);
        play(&mut t, "spotify", "A", 10_000);
        t.tick(11_500);
        assert_eq!(current(&t), Some("spotify"));
        play(&mut t, "chrome", "next video", 20_000);
        t.tick(21_500);
        assert_eq!(current(&t), Some("chrome"));
        play(&mut t, "spotify", "B", 200_000);
        t.tick(201_500);
        assert_eq!(current(&t), Some("spotify"));
    }

    #[test]
    fn preview_does_not_steal_the_island() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(1500);
        play(&mut t, "chrome", "preview", 5000);
        assert_eq!(current(&t), Some("spotify"));
        pause(&mut t, "chrome", "preview", 5300);
        assert!(t.tick(7000).is_empty());
        assert_eq!(current(&t), Some("spotify"));
    }

    #[test]
    fn playing_player_beats_recently_paused_one() {
        let mut t = Tracker::new();
        play(&mut t, "chrome", "video", 0);
        play(&mut t, "spotify", "A", 1000);
        t.tick(2500);
        assert_eq!(current(&t), Some("spotify"));
        assert!(pause(&mut t, "spotify", "A", 3000).is_empty());
        assert_eq!(current(&t), Some("chrome"));
    }

    #[test]
    fn pause_lives_ten_minutes() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(1500);
        assert_eq!(
            pause(&mut t, "spotify", "A", 5000),
            vec![PlayerEvent::Paused]
        );
        assert_eq!(t.presence(5000), Presence::SwipeOnly);
        let later = 5000 + PAUSED_LIFETIME_MS;
        assert_eq!(t.presence(later), Presence::Absent);
        play(&mut t, "spotify", "A", later);
        assert_eq!(t.presence(later), Presence::Rest(REST_PLAYING));
    }

    #[test]
    fn player_paused_before_start_is_available_by_swipe() {
        let mut t = Tracker::new();
        pause(&mut t, "spotify", "A", 0);
        assert_eq!(current(&t), Some("spotify"));
        assert_eq!(t.presence(0), Presence::SwipeOnly);
    }

    #[test]
    fn own_command_suppresses_announce() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(2000);
        t.own_command(5000);
        play(&mut t, "spotify", "B", 5200);
        assert!(t.tick(8000).is_empty());
        play(&mut t, "spotify", "C", 9000);
        assert_eq!(t.tick(10_500), vec![PlayerEvent::Announce]);
    }

    #[test]
    fn silent_session_is_dropped_until_next_event() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(1500);
        play(&mut t, "chrome", "muted video", 5000);
        t.tick(6500);
        assert_eq!(current(&t), Some("chrome"));

        t.mark_silent();
        assert_eq!(current(&t), Some("spotify"));
        play(&mut t, "chrome", "next video", 700_000);
        t.tick(701_500);
        assert_eq!(current(&t), Some("chrome"));
    }

    fn at(position_ms: i64, updated_at_ms: u64) -> Option<Timeline> {
        Some(Timeline {
            position_ms,
            duration_ms: 180_000,
            updated_at_ms,
        })
    }

    fn report(t: &mut Tracker, status: Status, timeline: Option<Timeline>, now: u64) {
        let mut s = session("spotify", "A", status);
        s.timeline = timeline;
        t.apply(Update::Changed(s), now);
    }

    #[test]
    fn a_slightly_off_position_is_ignored() {
        let mut t = Tracker::new();
        report(&mut t, Status::Playing, at(10_000, 1_000), 0);
        t.tick(CONFIRM_MS);
        report(&mut t, Status::Playing, at(14_300, 5_000), 4000);
        assert_eq!(t.current().unwrap().timeline, at(10_000, 1_000));
    }

    #[test]
    fn a_seek_moves_the_position() {
        let mut t = Tracker::new();
        report(&mut t, Status::Playing, at(10_000, 1_000), 0);
        t.tick(CONFIRM_MS);
        report(&mut t, Status::Playing, at(60_000, 5_000), 4000);
        assert_eq!(t.current().unwrap().timeline, at(60_000, 5_000));
    }

    #[test]
    fn a_pause_takes_the_reported_position() {
        let mut t = Tracker::new();
        report(&mut t, Status::Playing, at(10_000, 1_000), 0);
        t.tick(CONFIRM_MS);
        report(&mut t, Status::Paused, at(14_100, 5_000), 4000);
        assert_eq!(t.current().unwrap().timeline, at(14_100, 5_000));
    }

    #[test]
    fn a_seek_from_the_island_holds_against_the_old_position() {
        let mut t = Tracker::new();
        report(&mut t, Status::Playing, at(10_000, 1_000), 0);
        t.tick(CONFIRM_MS);
        t.seek(90_000, 4_000, 3000);
        assert_eq!(t.current().unwrap().timeline, at(90_000, 4_000));
        report(&mut t, Status::Playing, at(13_000, 4_000), 3300);
        assert_eq!(t.current().unwrap().timeline, at(90_000, 4_000));
        report(&mut t, Status::Playing, at(90_600, 5_000), 4000);
        assert_eq!(t.current().unwrap().timeline, at(90_000, 4_000));
    }

    #[test]
    fn after_the_hold_the_player_has_the_last_word() {
        let mut t = Tracker::new();
        report(&mut t, Status::Playing, at(10_000, 1_000), 0);
        t.tick(CONFIRM_MS);
        t.seek(90_000, 4_000, 3000);
        report(&mut t, Status::Playing, at(14_000, 5_000), 4000);
        assert_eq!(t.current().unwrap().timeline, at(14_000, 5_000));
    }

    #[test]
    fn closed_player_disappears() {
        let mut t = Tracker::new();
        play(&mut t, "spotify", "A", 0);
        t.tick(1500);
        t.apply(Update::Gone("spotify".into()), 2000);
        assert!(t.current().is_none());
        assert_eq!(t.presence(2000), Presence::Absent);
    }
}
