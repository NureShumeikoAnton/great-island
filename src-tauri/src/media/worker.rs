//! Media-session thread: subscribes to Windows events, builds session snapshots and runs commands.
//! Event handlers only put a message in the queue; all the work happens here, one message at a time,
//! with repeats merged.

use super::{Command, Controls, Icon, Session, Status, Timeline, Update, cover, icon};
use crate::lang::tr;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
};
use windows::{
    Foundation::{DateTime, TypedEventHandler},
    Media::{
        Control::{
            GlobalSystemMediaTransportControlsSession as WinSession,
            GlobalSystemMediaTransportControlsSessionManager as WinManager,
            GlobalSystemMediaTransportControlsSessionPlaybackStatus as WinStatus,
        },
        MediaPlaybackType,
    },
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    core::Result,
};

pub(super) enum Msg {
    Sessions,
    Refresh(String, Part),
    Command(String, Command),
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Part {
    Media,
    Playback,
    Timeline,
}

struct Tracked {
    session: WinSession,
    tokens: [i64; 3],
    snapshot: Session,
}

struct Worker {
    manager: WinManager,
    tracked: HashMap<String, Tracked>,
    icons: HashMap<String, Option<Arc<Icon>>>,
    events: Sender<Msg>,
    out: Sender<Update>,
}

pub(super) fn run(
    inbox: Receiver<Msg>,
    events: Sender<Msg>,
    out: Sender<Update>,
    notify: Box<dyn Fn() + Send>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let manager = match WinManager::RequestAsync().and_then(|op| op.join()) {
        Ok(manager) => manager,
        Err(e) => {
            crate::journal::warn(tr!("journal.media_unavailable", error = e));
            return;
        }
    };
    let tx = events.clone();
    let sessions_token = manager
        .SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Sessions);
            Ok(())
        }))
        .ok();

    let mut worker = Worker {
        manager,
        tracked: HashMap::new(),
        icons: HashMap::new(),
        events,
        out,
    };
    if worker.sync_sessions() {
        notify();
    }

    while let Ok(first) = inbox.recv() {
        // Players send events in bursts: update the same thing once.
        let mut batch = vec![first];
        batch.extend(inbox.try_iter());
        let mut refreshes: Vec<(String, Part)> = Vec::new();
        let mut sessions = false;
        let mut commands = Vec::new();
        for msg in batch {
            match msg {
                Msg::Quit => {
                    worker.shutdown(sessions_token);
                    unsafe { CoUninitialize() };
                    return;
                }
                Msg::Sessions => sessions = true,
                Msg::Refresh(key, part) => {
                    if !refreshes.contains(&(key.clone(), part)) {
                        refreshes.push((key, part));
                    }
                }
                Msg::Command(key, command) => commands.push((key, command)),
            }
        }

        let mut sent = false;
        for (key, command) in commands {
            worker.command(&key, command);
        }
        if sessions {
            sent |= worker.sync_sessions();
        }
        for (key, part) in refreshes {
            sent |= worker.refresh(&key, part);
        }
        if sent {
            notify();
        }
    }
}

impl Worker {
    fn sync_sessions(&mut self) -> bool {
        let current = match self.list_sessions() {
            Ok(list) => list,
            Err(e) => {
                crate::journal::warn(tr!("journal.media_get_failed", error = e));
                return false;
            }
        };
        let mut sent = false;

        let gone: Vec<String> = self
            .tracked
            .keys()
            .filter(|key| !current.iter().any(|(k, _)| k == *key))
            .cloned()
            .collect();
        for key in gone {
            if let Some(tracked) = self.tracked.remove(&key) {
                unsubscribe(&tracked);
            }
            sent |= self.out.send(Update::Gone(key)).is_ok();
        }

        for (key, session) in current {
            if self.tracked.contains_key(&key) {
                continue;
            }
            let Ok(app_id) = session.SourceAppUserModelId().map(|s| s.to_string()) else {
                continue;
            };
            crate::journal::info(tr!("journal.player_seen", app = app_id));
            let tokens = self.subscribe(&session, &key);
            let icon = self
                .icons
                .entry(app_id.clone())
                .or_insert_with(|| icon::load(&app_id).map(Arc::new))
                .clone();
            let snapshot = Session {
                key: key.clone(),
                app_id,
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                video: false,
                status: Status::Stopped,
                timeline: None,
                controls: Controls::default(),
                cover: None,
                icon,
            };
            self.tracked.insert(
                key.clone(),
                Tracked {
                    session,
                    tokens,
                    snapshot,
                },
            );
            for part in [Part::Media, Part::Playback, Part::Timeline] {
                self.update_part(&key, part);
            }
            if let Some(tracked) = self.tracked.get(&key) {
                sent |= self
                    .out
                    .send(Update::Changed(tracked.snapshot.clone()))
                    .is_ok();
            }
        }
        sent
    }

    fn list_sessions(&self) -> Result<Vec<(String, WinSession)>> {
        let sessions = self.manager.GetSessions()?;
        let mut list: Vec<(String, WinSession)> = Vec::new();
        for i in 0..sessions.Size()? {
            let session = sessions.GetAt(i)?;
            let app_id = session.SourceAppUserModelId()?.to_string();
            let numbered = format!("{app_id}#");
            let same = list
                .iter()
                .filter(|(k, _)| *k == app_id || k.starts_with(&numbered))
                .count();
            let key = if same == 0 {
                app_id
            } else {
                format!("{app_id}#{}", same + 1)
            };
            list.push((key, session));
        }
        Ok(list)
    }

    fn subscribe(&self, session: &WinSession, key: &str) -> [i64; 3] {
        let handler = |part: Part| {
            let tx = self.events.clone();
            let key = key.to_string();
            move || {
                let _ = tx.send(Msg::Refresh(key.clone(), part));
            }
        };
        let media = handler(Part::Media);
        let playback = handler(Part::Playback);
        let timeline = handler(Part::Timeline);
        [
            session
                .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    media();
                    Ok(())
                }))
                .unwrap_or_default(),
            session
                .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    playback();
                    Ok(())
                }))
                .unwrap_or_default(),
            session
                .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    timeline();
                    Ok(())
                }))
                .unwrap_or_default(),
        ]
    }

    fn refresh(&mut self, key: &str, part: Part) -> bool {
        let Some(before) = self.tracked.get(key).map(|t| t.snapshot.clone()) else {
            return false;
        };
        self.update_part(key, part);
        match self.tracked.get(key) {
            Some(t) if t.snapshot != before => {
                self.out.send(Update::Changed(t.snapshot.clone())).is_ok()
            }
            _ => false,
        }
    }

    fn update_part(&mut self, key: &str, part: Part) {
        let Some(tracked) = self.tracked.get_mut(key) else {
            return;
        };
        // No error is shown: the session may have closed between the event and the read.
        let _ = match part {
            Part::Media => read_media(&tracked.session, &mut tracked.snapshot),
            Part::Playback => read_playback(&tracked.session, &mut tracked.snapshot),
            Part::Timeline => read_timeline(&tracked.session, &mut tracked.snapshot),
        };
    }

    fn command(&self, key: &str, command: Command) {
        let Some(tracked) = self.tracked.get(key) else {
            return;
        };
        let session = &tracked.session;
        let result = match command {
            Command::TogglePlayPause => session.TryTogglePlayPauseAsync().and_then(|op| op.join()),
            Command::Next => session.TrySkipNextAsync().and_then(|op| op.join()),
            Command::Previous => session.TrySkipPreviousAsync().and_then(|op| op.join()),
            Command::Seek(ms) => session
                .GetTimelineProperties()
                .and_then(|t| t.StartTime())
                .and_then(|start| {
                    session.TryChangePlaybackPositionAsync(start.Duration + ms * TICKS_PER_MS)
                })
                .and_then(|op| op.join()),
        };
        match result {
            Ok(true) => {}
            Ok(false) => crate::journal::warn(tr!(
                "journal.player_refused",
                command = format!("{command:?}")
            )),
            Err(e) => crate::journal::warn(tr!(
                "journal.player_rejected",
                command = format!("{command:?}"),
                error = e
            )),
        }
    }

    fn shutdown(&mut self, sessions_token: Option<i64>) {
        for tracked in self.tracked.values() {
            unsubscribe(tracked);
        }
        self.tracked.clear();
        if let Some(token) = sessions_token {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
    }
}

fn unsubscribe(tracked: &Tracked) {
    let [media, playback, timeline] = tracked.tokens;
    let s = &tracked.session;
    let _ = s.RemoveMediaPropertiesChanged(media);
    let _ = s.RemovePlaybackInfoChanged(playback);
    let _ = s.RemoveTimelinePropertiesChanged(timeline);
}

/// WinRT time is in 100 ns units.
const TICKS_PER_MS: i64 = 10_000;
/// 100 ns units between 1601 and 1970.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

fn read_media(session: &WinSession, snapshot: &mut Session) -> Result<()> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = props.Title()?.to_string();
    let mut artist = props.Artist()?.to_string();
    if artist.is_empty() {
        artist = props.AlbumArtist()?.to_string();
    }
    snapshot.title = title;
    snapshot.artist = artist;
    snapshot.album = props.AlbumTitle()?.to_string();
    if let Ok(kind) = props.PlaybackType().and_then(|t| t.Value()) {
        snapshot.video = kind == MediaPlaybackType::Video;
    }
    // Read the cover on every change: on a tab or track change browsers first send the new title with the
    // old picture, and the new one with the next event. The same pixels give the same object, so the view
    // doesn't rebuild for nothing.
    let cover = props.Thumbnail().ok().and_then(|r| cover::decode(&r).ok());
    let same = match (&snapshot.cover, &cover) {
        (Some(old), Some(new)) => old.bgra == new.bgra,
        (None, None) => true,
        _ => false,
    };
    if !same {
        snapshot.cover = cover.map(Arc::new);
    }
    Ok(())
}

fn read_playback(session: &WinSession, snapshot: &mut Session) -> Result<()> {
    let info = session.GetPlaybackInfo()?;
    snapshot.status = match info.PlaybackStatus()? {
        WinStatus::Playing => Status::Playing,
        WinStatus::Paused => Status::Paused,
        // "Changing" is a short moment between tracks: keep the previous status,
        // otherwise a pause would flash on every track change.
        WinStatus::Changing => snapshot.status,
        _ => Status::Stopped,
    };
    let controls = info.Controls()?;
    snapshot.controls = Controls {
        play_pause: controls.IsPlayPauseToggleEnabled()?
            || controls.IsPlayEnabled()?
            || controls.IsPauseEnabled()?,
        next: controls.IsNextEnabled()?,
        previous: controls.IsPreviousEnabled()?,
        seek: controls.IsPlaybackPositionEnabled()?,
    };
    if let Ok(kind) = info.PlaybackType().and_then(|t| t.Value()) {
        snapshot.video = kind == MediaPlaybackType::Video;
    }
    Ok(())
}

fn read_timeline(session: &WinSession, snapshot: &mut Session) -> Result<()> {
    let t = session.GetTimelineProperties()?;
    let start = t.StartTime()?.Duration;
    let end = t.EndTime()?.Duration;
    let position = t.Position()?.Duration;
    snapshot.timeline = (end > start).then(|| Timeline {
        position_ms: (position - start) / TICKS_PER_MS,
        duration_ms: (end - start) / TICKS_PER_MS,
        updated_at_ms: unix_ms(t.LastUpdatedTime().unwrap_or_default()),
    });
    Ok(())
}

fn unix_ms(time: DateTime) -> u64 {
    if time.UniversalTime <= UNIX_EPOCH_TICKS {
        return crate::persist::now_ms();
    }
    ((time.UniversalTime - UNIX_EPOCH_TICKS) / TICKS_PER_MS) as u64
}
