//! "Now playing": Windows media sessions, the same source as the system volume flyout. Works with any
//! player that reports there: Spotify, browsers, VLC, etc.
//!
//! WinRT events arrive on background threads, so a separate thread (`worker`) talks to the system and
//! the island receives ready snapshots of the sessions through a channel.

mod app;
mod cover;
mod icon;
mod meter;
mod tracker;
mod worker;

pub use app::open_app;
pub use cover::{Cover, NEUTRAL_TINT};
pub use icon::Icon;
pub use meter::{Level, LevelSource, Meter, output_is_headphones};
pub use tracker::{PlayerEvent, Tracker};

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub position_ms: i64,
    pub duration_ms: i64,
    /// Unix ms when the player reported the position.
    pub updated_at_ms: u64,
}

impl Timeline {
    pub fn position_at(&self, now_ms: u64, playing: bool) -> i64 {
        let drift = if playing {
            now_ms.saturating_sub(self.updated_at_ms) as i64
        } else {
            0
        };
        (self.position_ms + drift).clamp(0, self.duration_ms)
    }

    /// While playing, both run at the same speed: the gap between them is the same at any moment.
    pub fn agrees(&self, newer: &Timeline, tolerance_ms: i64) -> bool {
        let start = |t: &Timeline| t.updated_at_ms as i64 - t.position_ms;
        self.duration_ms == newer.duration_ms && (start(self) - start(newer)).abs() <= tolerance_ms
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub play_pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub key: String,
    /// The app's AppUserModelID: used to find the player's window and audio.
    pub app_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub video: bool,
    pub status: Status,
    pub timeline: Option<Timeline>,
    pub controls: Controls,
    pub cover: Option<Arc<Cover>>,
    pub icon: Option<Arc<Icon>>,
}

impl Session {
    pub fn is_playing(&self) -> bool {
        self.status == Status::Playing
    }

    pub fn same_track(&self, other: &Session) -> bool {
        self.title == other.title && self.artist == other.artist
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        self.key == other.key
            && self.app_id == other.app_id
            && self.same_track(other)
            && self.album == other.album
            && self.video == other.video
            && self.status == other.status
            && self.timeline == other.timeline
            && self.controls == other.controls
            && same(&self.cover, &other.cover)
            && same(&self.icon, &other.icon)
    }
}

#[derive(Debug)]
pub enum Update {
    Changed(Session),
    Gone(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    TogglePlayPause,
    Next,
    Previous,
    /// Position in ms from the track start.
    Seek(i64),
}

/// Link to the media-session thread. The thread lives as long as this object.
pub struct Media {
    to_worker: Sender<worker::Msg>,
    updates: Receiver<Update>,
    thread: Option<JoinHandle<()>>,
}

impl Media {
    /// `notify` is called from a background thread when there are new snapshots:
    /// the island picks them up via `drain` on its own thread.
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let (to_worker, inbox) = mpsc::channel();
        let (out, updates) = mpsc::channel();
        let events = to_worker.clone();
        let thread = std::thread::Builder::new()
            .name("media".into())
            .spawn(move || worker::run(inbox, events, out, Box::new(notify)))
            .ok();
        Self {
            to_worker,
            updates,
            thread,
        }
    }

    pub fn drain(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }

    pub fn send(&self, key: &str, command: Command) {
        let _ = self
            .to_worker
            .send(worker::Msg::Command(key.to_string(), command));
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        let _ = self.to_worker.send(worker::Msg::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
