//! The recording thread: Windows.Graphics.Capture → crop (and scale) on the video card →
//! encoder, with the sound inputs' packets mixed into the sound track on the way.
//!
//! Frames stay on the video card the whole way. The thread sleeps between frames;
//! frames arrive only when the screen changes.

use super::super::{
    Clock, Codec, DISK_RESERVE, FADE_MS, Fade, Pacer, Plan,
    choice::Choice,
    sound::{Sound, Track, frames as frames_in},
};
use super::{
    encoder::{Encoder, SECOND},
    fade::Mixer,
    files,
    scale::Scaler,
    sound::{self, Heard, Listening, Note},
};
use crate::{
    capture::{
        Image, Source,
        adapters::{graphics, graphics::item as capture_item},
    },
    fault::{Cause, Failed},
    gpu, journal,
    lang::tr,
};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Graphics::Direct3D11::{D3D11_BOX, ID3D11Texture2D},
        Media::MediaFoundation::{MF_VERSION, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup},
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    },
    core::{Error, Result},
};

const DISK_CHECK: Duration = Duration::from_secs(2);
/// Sound is cut this far behind the present: packets arrive a few ms after they were
/// heard, and the slower input must be in before its part of the track is mixed.
const SOUND_LAG: u64 = SECOND / 5;
const SOUND_EVERY: u64 = SECOND / 4;

const THREAD: &str = "record";
const RESCUE_WAIT: Duration = Duration::from_secs(3);

/// The running recording, reachable when the process is going down.
static LIVE: Mutex<Option<Sender<Msg>>> = Mutex::new(None);
static WRITING: AtomicBool = AtomicBool::new(false);

/// The process is crashing in another thread: ask the recording to close its file and
/// give it a moment. An MP4 that is not closed has no index and does not play.
pub fn rescue() {
    if std::thread::current().name() == Some(THREAD) {
        return;
    }
    let Ok(live) = LIVE.try_lock() else {
        return;
    };
    let Some(tx) = live.as_ref() else {
        return;
    };
    let _ = tx.send(Msg::Stop);
    let deadline = Instant::now() + RESCUE_WAIT;
    while WRITING.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub enum Event {
    Started,
    Finished {
        path: PathBuf,
        duration_ms: u64,
        bytes: u64,
        /// The video's own size.
        size: (u32, u32),
        thumb: Option<Image>,
        ending: Ending,
    },
    Cancelled,
    /// No file. "started" — it broke after frames were already being written.
    Failed {
        started: bool,
        failed: Failed,
    },
    /// The recording goes on; something changed with its sound.
    Sound(Note),
    /// HEVC could not be opened: the recording goes in H.264.
    FellBack(Failed),
}

pub enum Ending {
    Asked,
    Stopped(Cause),
    Broken(Failed),
}

enum Msg {
    Frame,
    Heard(Heard),
    Lost,
    Pause,
    Resume,
    Stop,
    Cancel,
}

#[derive(Clone)]
pub struct Setup {
    pub thumb_width: u32,
    /// Its sound is what will really be heard: without the microphone if it is missing.
    pub choice: Choice,
}

/// A running recording. Dropping it stops the recording and keeps the file.
pub struct Engine {
    tx: Sender<Msg>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}

impl Engine {
    /// Start recording `source` into a new file in `dir`.
    /// `notify` is called from the recording thread when there is an [`Event`].
    pub fn start(
        source: Source,
        dir: &Path,
        stem: &str,
        setup: Setup,
        notify: impl Fn() + Send + 'static,
    ) -> std::result::Result<Self, Failed> {
        let path = files::reserve(dir, stem)?;
        let (tx, rx) = mpsc::channel();
        let (out, events) = mpsc::channel();
        let frames = tx.clone();
        let file = path.clone();
        let thread = std::thread::Builder::new()
            .name(THREAD.into())
            .spawn(move || {
                WRITING.store(true, Ordering::Release);
                let report = move |event| {
                    let _ = out.send(event);
                    notify();
                };
                let event = run(source, &file, setup, frames, rx, &report);
                WRITING.store(false, Ordering::Release);
                report(event);
            })?;
        if let Ok(mut live) = LIVE.lock() {
            *live = Some(tx.clone());
        }
        Ok(Self {
            tx,
            events,
            thread: Some(thread),
            path,
        })
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Msg::Pause);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(Msg::Resume);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Msg::Stop);
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(Msg::Cancel);
    }

    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map_or(0, |m| m.len())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(mut live) = LIVE.lock() {
            *live = None;
        }
        let _ = self.tx.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    source: Source,
    path: &Path,
    setup: Setup,
    frames: Sender<Msg>,
    rx: Receiver<Msg>,
    report: &dyn Fn(Event),
) -> Event {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) {
            CoUninitialize();
            let _ = std::fs::remove_file(path);
            return Event::Failed {
                started: false,
                failed: Failed::because(Cause::Unsupported, e),
            };
        }
    }
    let started = Cell::new(false);
    let on_start = || {
        started.set(true);
        report(Event::Started);
    };
    let on_sound = |note| report(Event::Sound(note));
    let on_fallback = |failed| report(Event::FellBack(failed));
    let says = Says {
        start: &on_start,
        sound: &on_sound,
        fallback: &on_fallback,
    };
    let event = match record(source, path, setup, frames, &rx, &says) {
        Ok(event) => event,
        Err(failed) => {
            let _ = std::fs::remove_file(path);
            Event::Failed {
                started: started.get(),
                failed,
            }
        }
    };
    unsafe {
        let _ = MFShutdown();
        CoUninitialize();
    }
    event
}

enum End {
    Asked,
    Stopped(Cause),
    Discard,
    Broken(Error),
}

/// A setup step failed. The code alone rarely says which part of the system it was:
/// when it says nothing, the step's own cause stands.
fn step<T>(result: Result<T>, cause: Cause) -> std::result::Result<T, Failed> {
    result.map_err(|e| {
        let failed = Failed::from(e);
        match failed.cause {
            Cause::Unknown => Failed { cause, ..failed },
            _ => failed,
        }
    })
}

/// What the recording thread tells the island while it records.
struct Says<'a> {
    start: &'a dyn Fn(),
    sound: &'a dyn Fn(Note),
    fallback: &'a dyn Fn(Failed),
}

fn record(
    source: Source,
    path: &Path,
    setup: Setup,
    frames: Sender<Msg>,
    rx: &Receiver<Msg>,
    says: &Says,
) -> std::result::Result<Event, Failed> {
    let Setup {
        thumb_width,
        choice,
    } = setup;
    let (fade, sound) = (choice.fade, choice.sound);
    let graphics = |e: Error| Failed::because(Cause::Graphics, e);
    let device = gpu::video_device().map_err(graphics)?;
    let item = step(capture_item(source.monitor), Cause::Missing)?;
    let size = step(item.Size(), Cause::Missing)?;
    let mut plan = Plan::new(
        source.area,
        (size.Width as u32, size.Height as u32),
        &choice,
    )
    .ok_or_else(|| Failed::because(Cause::Missing, tr!("journal.empty_area")))?;
    let crop = (plan.crop.width, plan.crop.height);

    let (on_frame, lost) = (frames.clone(), frames.clone());
    let opened = graphics::open(
        &device,
        &item,
        size,
        move || {
            let _ = on_frame.send(Msg::Frame);
        },
        move || {
            let _ = lost.send(Msg::Lost);
        },
    );
    let (pool, session) = step(opened, Cause::Unsupported)?;
    let _ = session.SetIsCursorCaptureEnabled(true);
    graphics::borderless(&session);

    let mut encoder = match Encoder::new(&device, path, &plan) {
        // Not every video card encodes HEVC: a recording in H.264 beats none.
        Err(e) if plan.codec == Codec::Hevc => {
            (says.fallback)(e.into());
            plan.codec = Codec::H264;
            step(Encoder::new(&device, path, &plan), Cause::Unsupported)?
        }
        opened => step(opened, Cause::Unsupported)?,
    };
    let latest = gpu::texture(&device, crop).map_err(graphics)?;
    let mut scaler = match (plan.width, plan.height) == crop {
        true => None,
        false => Some(Scaler::new(&device, (plan.width, plan.height)).map_err(graphics)?),
    };
    let context = unsafe { device.GetImmediateContext() }.map_err(graphics)?;
    // Started before the picture: the inputs take a moment to open, and what they hear
    // before the first frame is dropped by its stamp.
    let heard = frames.clone();
    let listening = match sound {
        Sound::Off => None,
        _ => Some(Listening::start(
            sound,
            choice.microphone.clone(),
            move |h| {
                let _ = heard.send(Msg::Heard(h));
            },
        )?),
    };
    step(session.StartCapture(), Cause::Unsupported)?;
    (says.start)();

    // The performance counter: the sound packets are stamped with it.
    let started = sound::ticks();
    let now = || sound::ticks().saturating_sub(started);
    let mut track = Track::new(sound, choice.balance);
    let mut sound_due = SOUND_EVERY;
    let mut clock = Clock::start(0);
    let mut pacer = Pacer::new(plan.fps, SECOND);
    let mut have_frame = false;
    // Made on the first pause: two more frame-size textures on the video card.
    let mut mixer: Option<Mixer> = None;
    let mut held = false;
    let mut dissolve: Option<Fade> = None;
    let mut out = Out {
        encoder: &mut encoder,
        latest: &latest,
        scaler: scaler.as_mut(),
        pacer: &mut pacer,
    };
    let mut disk_checked = Instant::now();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

    // A failure past this point ends the recording but keeps what is already written.
    let mut frames_loop = || -> Result<End> {
        loop {
            let wait = match (out.pacer.due(), clock.is_paused()) {
                (Some(due), false) => Duration::from_nanos(due.saturating_sub(now()) * 100),
                _ => DISK_CHECK,
            };
            let sound_wait = Duration::from_nanos(sound_due.saturating_sub(now()) * 100);
            let msg = match rx.recv_timeout(wait.min(DISK_CHECK).min(sound_wait)) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(End::Asked),
            };
            match msg {
                Some(Msg::Frame) => {
                    let mut changed = false;
                    while let Ok(frame) = pool.TryGetNextFrame() {
                        if frame.ContentSize()? != size {
                            changed = true;
                        }
                        let surface = graphics::texture(&frame)?;
                        let area = D3D11_BOX {
                            left: plan.crop.x,
                            top: plan.crop.y,
                            front: 0,
                            right: plan.crop.x + plan.crop.width,
                            bottom: plan.crop.y + plan.crop.height,
                            back: 1,
                        };
                        unsafe {
                            context.CopySubresourceRegion(
                                &latest,
                                0,
                                0,
                                0,
                                0,
                                &surface,
                                0,
                                Some(&area),
                            );
                        }
                        have_frame = true;
                    }
                    if changed {
                        return Ok(End::Stopped(Cause::DisplayChanged));
                    }
                    let t = now();
                    if have_frame && !clock.is_paused() && out.pacer.arrived(t) {
                        out.put(mixer.as_ref(), &mut dissolve, t, clock.elapsed(t))?;
                    }
                }
                Some(Msg::Heard(Heard::Packet { input, at, samples })) => {
                    let at = if at >= started {
                        frames_in(at - started) as i64
                    } else {
                        -(frames_in(started - at) as i64)
                    };
                    track.push(input, at, &samples);
                }
                Some(Msg::Heard(Heard::Note(note))) => (says.sound)(note),
                Some(Msg::Pause) => {
                    let t = now();
                    clock.pause(t);
                    track.pause(frames_in(t));
                    if fade && have_frame && !held {
                        if mixer.is_none() {
                            mixer = Mixer::new(&device, &latest, crop)
                                .inspect_err(|e| {
                                    journal::warn(tr!("journal.pause_dissolve_failed", error = e))
                                })
                                .ok();
                        }
                        if let Some(mixer) = &mixer {
                            mixer.hold(&context, &latest);
                            held = true;
                        }
                    }
                }
                Some(Msg::Resume) => {
                    let t = now();
                    clock.resume(t);
                    track.resume(frames_in(t));
                    if std::mem::take(&mut held) {
                        dissolve = Some(Fade::new(t, FADE_MS * SECOND / 1000));
                    }
                    if have_frame {
                        out.put(mixer.as_ref(), &mut dissolve, t, clock.elapsed(t))?;
                    }
                }
                Some(Msg::Stop) => return Ok(End::Asked),
                Some(Msg::Lost) => return Ok(End::Stopped(Cause::MonitorGone)),
                Some(Msg::Cancel) => return Ok(End::Discard),
                None => {
                    let t = now();
                    if have_frame
                        && !clock.is_paused()
                        && out.pacer.due().is_some_and(|due| t >= due)
                    {
                        out.put(mixer.as_ref(), &mut dissolve, t, clock.elapsed(t))?;
                    }
                }
            }
            let t = now();
            if t >= sound_due {
                sound_due = t + SOUND_EVERY;
                let until = frames_in(t.saturating_sub(SOUND_LAG));
                out.encoder.write_sound(&track.take(until))?;
            }
            if disk_checked.elapsed() >= DISK_CHECK {
                disk_checked = Instant::now();
                if files::free_space(&dir).is_some_and(|free| free < DISK_RESERVE) {
                    return Ok(End::Stopped(Cause::DiskFull));
                }
            }
        }
    };
    let end = frames_loop().unwrap_or_else(End::Broken);

    let _ = session.Close();
    let _ = pool.Close();
    drop(listening);
    let t = now();
    let duration = clock.elapsed(t);
    // What the inputs had no time to deliver is silence. A refused tail only leaves the
    // track short, and closing the file must not wait on it.
    let _ = encoder.write_sound(&track.take(frames_in(t)));
    let ending = match end {
        End::Discard => {
            drop(encoder);
            let _ = std::fs::remove_file(path);
            return Ok(Event::Cancelled);
        }
        End::Asked => Ending::Asked,
        End::Stopped(cause) => Ending::Stopped(cause),
        End::Broken(e) => Ending::Broken(e.into()),
    };
    if !have_frame {
        drop(encoder);
        let _ = std::fs::remove_file(path);
        return match ending {
            // Stopped before the first frame: nothing was recorded, nothing is lost.
            Ending::Asked => Ok(Event::Cancelled),
            Ending::Stopped(cause) => Err(Failed::because(cause, tr!("journal.no_frames"))),
            Ending::Broken(failed) => Err(failed),
        };
    }
    if let Err(e) = encoder.finish(duration) {
        // What broke the recording explains more than the file that then failed to close.
        return Err(match ending {
            Ending::Broken(failed) => failed,
            _ => e.into(),
        });
    }
    let thumb = gpu::read_back(&device, &latest, crop).ok().map(|pixels| {
        Image {
            width: crop.0,
            height: crop.1,
            pixels,
        }
        .preview(thumb_width)
    });
    Ok(Event::Finished {
        path: path.to_path_buf(),
        duration_ms: duration / 10_000,
        bytes: std::fs::metadata(path).map_or(0, |m| m.len()),
        size: (plan.width, plan.height),
        thumb,
        ending,
    })
}

struct Out<'a> {
    encoder: &'a mut Encoder,
    latest: &'a ID3D11Texture2D,
    scaler: Option<&'a mut Scaler>,
    pacer: &'a mut Pacer,
}

impl Out<'_> {
    /// The live picture at `at` (recorded time), or during a dissolve its mix with the
    /// picture from before the pause. `t` — the thread's own time.
    fn put(
        &mut self,
        mixer: Option<&Mixer>,
        dissolve: &mut Option<Fade>,
        t: u64,
        at: u64,
    ) -> Result<()> {
        let old = dissolve.and_then(|fade| fade.old(t));
        let frame = match (old, mixer) {
            (Some(old), Some(mixer)) => mixer.mix(old).unwrap_or_else(|e| {
                journal::warn(tr!("journal.pause_dissolve_failed", error = e));
                self.latest
            }),
            _ => self.latest,
        };
        let frame = match &mut self.scaler {
            Some(scaler) => scaler.scale(frame)?,
            None => frame,
        };
        self.encoder.write(frame, at)?;
        self.pacer.wrote(t);
        if old.is_some() {
            // A still screen sends no frames: the dissolve needs every slot.
            self.pacer.hurry();
        } else {
            *dissolve = None;
        }
        Ok(())
    }
}
