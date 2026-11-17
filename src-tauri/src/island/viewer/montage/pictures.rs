//! What the timeline is drawn from: the recording's thumbnails and its sound's peaks. They are
//! read on two threads of their own the first time the editor opens, so the timeline shows at
//! once and fills in; the window keeps them for the next time.

use super::super::super::window::WM_STRIP;
use crate::fault::Failed;
use crate::gpu::Card;
use crate::montage::{
    adapters::{Session, reader},
    strip::{Peaks, thumb_times},
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

/// Thumbnail height in pixels: sharp on the timeline at 250% scale.
const THUMB_HEIGHT: u32 = 90;
/// Thumbnails come in batches of this many: a long recording fills in as it goes.
const BATCH: usize = 24;

pub struct Thumbs {
    pub times: Vec<f64>,
    pub size: (u32, u32),
    /// BGRA, in the order of `times`; may be shorter while they come.
    pub pixels: Vec<Vec<u8>>,
}

impl Thumbs {
    /// The arrived thumbnail closest to a source moment.
    pub fn near(&self, source: f64) -> Option<usize> {
        let count = self.pixels.len().min(self.times.len());
        self.times[..count]
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - source).abs().total_cmp(&(b.1 - source).abs()))
            .map(|(i, _)| i)
    }
}

enum Arrival {
    Fps(u32),
    Thumbs(Vec<f64>, (u32, u32), Vec<Vec<u8>>),
    /// `None`: the recording has no sound.
    Peaks(Option<Vec<f32>>),
    Failed(Failed),
}

#[derive(Default)]
pub struct Pictures {
    pub thumbs: Option<Thumbs>,
    pub peaks: Option<Vec<f32>>,
    pub silent: bool,
    /// The recording's frame rate, for the frame step and the size estimates.
    pub fps: Option<u32>,
    loading: Option<Receiver<Arrival>>,
    /// Raised when the pictures go: the two reading threads, each with a decoder of its own, must not
    /// finish the whole recording for a window that is closed.
    stop: Arc<AtomicBool>,
}

impl Drop for Pictures {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Pictures {
    /// Starts reading once, on the window's `card`; `size` is the video's own size, for the
    /// thumbnails' proportions.
    pub fn load(&mut self, hwnd: HWND, card: &Card, path: &Path, duration: f64, size: (u32, u32)) {
        if self.loading.is_some() || self.thumbs.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.loading = Some(rx);
        let width =
            (THUMB_HEIGHT as f32 * size.0.max(1) as f32 / size.1.max(1) as f32).round() as u32 & !1;
        let thumb = (width.max(2), THUMB_HEIGHT);
        let address = hwnd.0 as isize;
        let file = path.to_path_buf();
        let sender = tx.clone();
        let stop = self.stop.clone();
        let card = card.clone();
        let spawned = std::thread::Builder::new()
            .name("timeline thumbnails".into())
            .spawn(move || read_thumbs(&card, &file, duration, thumb, &sender, address, &stop));
        if let Err(e) = spawned {
            let _ = tx.send(Arrival::Failed(e.into()));
        }
        let file: PathBuf = path.to_path_buf();
        let sender = tx.clone();
        let stop = self.stop.clone();
        let spawned = std::thread::Builder::new()
            .name("timeline wave".into())
            .spawn(move || read_wave(&file, &sender, address, &stop));
        if let Err(e) = spawned {
            let _ = tx.send(Arrival::Failed(e.into()));
        }
    }

    /// Takes what arrived. Returns whether anything changed and the failures, for the journal.
    pub fn take(&mut self) -> (bool, Vec<Failed>) {
        let Some(rx) = &self.loading else {
            return (false, Vec::new());
        };
        let (mut changed, mut failures) = (false, Vec::new());
        while let Ok(arrival) = rx.try_recv() {
            match arrival {
                Arrival::Thumbs(times, size, mut pixels) => {
                    changed = true;
                    match &mut self.thumbs {
                        Some(thumbs) => thumbs.pixels.append(&mut pixels),
                        None => {
                            self.thumbs = Some(Thumbs {
                                times,
                                size,
                                pixels,
                            })
                        }
                    }
                }
                Arrival::Fps(fps) => self.fps = Some(fps.max(1)),
                Arrival::Peaks(peaks) => {
                    changed = true;
                    self.silent = peaks.is_none();
                    self.peaks = peaks;
                }
                Arrival::Failed(failed) => failures.push(failed),
            }
        }
        (changed, failures)
    }
}

fn post(address: isize) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(address as *mut _)),
            WM_STRIP,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

fn read_thumbs(
    card: &Card,
    path: &Path,
    duration: f64,
    size: (u32, u32),
    tx: &Sender<Arrival>,
    address: isize,
    stop: &AtomicBool,
) {
    let send = |arrival| {
        let _ = tx.send(arrival);
        post(address);
    };
    let read = || -> Result<(), Failed> {
        let _session = Session::start()?;
        if let Ok(info) = reader::info(path) {
            send(Arrival::Fps(info.fps));
        }
        let times = thumb_times(duration);
        for (i, batch) in times.chunks(BATCH).enumerate() {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let pixels = reader::thumbnails(&card.manager, path, batch, size)?;
            let last = pixels.len() < batch.len();
            let all = if i == 0 { times.clone() } else { Vec::new() };
            send(Arrival::Thumbs(all, size, pixels));
            if last {
                break;
            }
        }
        Ok(())
    };
    if let Err(failed) = read() {
        send(Arrival::Failed(failed));
    }
}

fn read_wave(path: &Path, tx: &Sender<Arrival>, address: isize, stop: &AtomicBool) {
    let read = || -> Result<Option<Vec<f32>>, Failed> {
        let _session = Session::start()?;
        let mut peaks = None;
        let had = reader::sound(path, |(rate, channels), samples| {
            peaks
                .get_or_insert_with(|| Peaks::new(rate, channels))
                .push(samples);
            !stop.load(Ordering::Acquire)
        })?;
        Ok(had.then(|| peaks.map(Peaks::finish).unwrap_or_default()))
    };
    let arrival = match read() {
        Ok(peaks) => Arrival::Peaks(peaks),
        Err(failed) => Arrival::Failed(failed),
    };
    let _ = tx.send(arrival);
    post(address);
}
