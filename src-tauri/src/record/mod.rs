//! Screen recording: everything that does not need Windows.
//!
//! - [`choice`] — what the next recording will be like (the pre-start panel), [`panel`] — what
//!   a press in that panel does, [`meter`] — the bars of the microphone it listens to;
//! - [`Plan`] — what the encoder gets: crop on the captured monitor, video size, codec;
//! - [`Clock`] — recorded time without pauses (for the island and for frame timestamps);
//! - [`Pacer`] — when a frame goes to the encoder;
//! - [`Fade`] — the dissolve after a pause;
//! - [`Take`] — a finished recording on the card: copy, or clipboard only (Ctrl+X);
//! - [`sound`] — what is heard in the recording and how its inputs become one track.
//!
//! What to record ([`Source`]) and the status of the file come from `capture`: a screenshot
//! uses the same. The Windows side (capture, encoder, files) lives in `adapters/`.

pub mod adapters;
pub mod choice;
pub mod meter;
pub mod panel;
pub mod sound;

use crate::{
    capture::{Area, Stamp, Status},
    fault::Failed,
    lang::tr,
};
use choice::Choice;
pub use choice::Codec;
use std::path::{Path, PathBuf};

pub const DISK_RESERVE: u64 = 500 * 1024 * 1024;
/// AAC at its lowest rate: enough for speech and system sounds.
pub const SOUND_BITRATE: u32 = 96_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub crop: Area,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: Codec,
}

/// Smaller than this the video makes no sense.
const MIN_SIDE: u32 = 16;

/// H.264 takes even sides. HEVC encoders take multiples of 8 and pad the rest themselves:
/// the padding shows as a strip of garbage along the edge of the video.
fn side(v: u32, codec: Codec) -> u32 {
    match codec {
        Codec::H264 => v & !1,
        Codec::Hevc => v & !7,
    }
}

impl Plan {
    /// `area` clipped to a monitor of `monitor` size; the quality divides the video size.
    /// `None` — nothing left to record.
    pub fn new(area: Area, monitor: (u32, u32), choice: &Choice) -> Option<Self> {
        let x = area.x.min(monitor.0);
        let y = area.y.min(monitor.1);
        let codec = choice.codec;
        let width = side(area.width.min(monitor.0 - x), codec);
        let height = side(area.height.min(monitor.1 - y), codec);
        if width < MIN_SIDE || height < MIN_SIDE {
            return None;
        }
        let shrink = choice.quality.shrink().max(1.0);
        let scaled = |v: u32| side((v as f32 / shrink) as u32, codec).max(MIN_SIDE);
        Some(Self {
            crop: Area {
                x,
                y,
                width,
                height,
            },
            width: scaled(width),
            height: scaled(height),
            fps: choice.rate.fps(),
            codec,
        })
    }

    /// Bitrate ceiling, bits per second. Screen content mostly stands still and the
    /// encoder spends far less; the ceiling only limits busy moments (scrolling, video).
    pub fn bitrate(&self) -> u32 {
        const BITS_PER_PIXEL: f64 = 0.06;
        let bits = self.width as f64 * self.height as f64 * self.fps as f64 * BITS_PER_PIXEL;
        (bits as u32).clamp(1_000_000, 40_000_000)
    }

    /// About how many megabytes a minute of ordinary work on screen takes: windows, text,
    /// scrolling now and then. A film or a game on screen takes several times more.
    ///
    /// Fitted to recordings of such work on this machine (2026-11-15): the video grows
    /// slower than its pixels (text stays sharp at any size and costs about the same), 60
    /// fps adds about a sixth (only changed frames are written), HEVC takes about 70% of
    /// H.264. The sound track is there always: silent, it weighs the same.
    pub fn megabytes_per_minute(&self) -> u32 {
        const MB_PER_MEGAPIXEL: f64 = 3.2;
        const PIXEL_GROWTH: f64 = 0.7;
        const SIXTY_FPS: f64 = 1.15;
        const HEVC_SHARE: f64 = 0.7;
        let megapixels = f64::from(self.width) * f64::from(self.height) / 1_000_000.0;
        let rate = if self.fps > 30 { SIXTY_FPS } else { 1.0 };
        let codec = match self.codec {
            Codec::H264 => 1.0,
            Codec::Hevc => HEVC_SHARE,
        };
        let video = MB_PER_MEGAPIXEL * megapixels.powf(PIXEL_GROWTH) * rate * codec;
        let sound = f64::from(SOUND_BITRATE) * 60.0 / 8.0 / (1024.0 * 1024.0);
        (video + sound).round().max(1.0) as u32
    }
}

/// Recorded time without pauses. Units are the caller's (island ms, 100 ns ticks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    started: u64,
    paused_at: Option<u64>,
    paused_total: u64,
}

impl Clock {
    pub fn start(now: u64) -> Self {
        Self {
            started: now,
            paused_at: None,
            paused_total: 0,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn pause(&mut self, now: u64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    pub fn resume(&mut self, now: u64) {
        if let Some(at) = self.paused_at.take() {
            self.paused_total += now.saturating_sub(at);
        }
    }

    pub fn elapsed(&self, now: u64) -> u64 {
        let end = self.paused_at.unwrap_or(now);
        end.saturating_sub(self.started)
            .saturating_sub(self.paused_total)
    }
}

/// When a frame goes to the encoder. Frames arrive only when the screen changes.
/// They are written at most `fps` times a second; one that came too early waits for
/// its slot (so the last state is never lost). On a still screen the last frame is
/// repeated once a second: players can seek through still parts.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    interval: u64,
    max_gap: u64,
    last: Option<u64>,
    waiting: bool,
}

impl Pacer {
    pub fn new(fps: u32, second: u64) -> Self {
        Self {
            interval: second / u64::from(fps.max(1)),
            max_gap: second,
            last: None,
            waiting: false,
        }
    }

    pub fn arrived(&mut self, now: u64) -> bool {
        match self.last {
            Some(last) if now < last + self.interval => {
                self.waiting = true;
                false
            }
            _ => true,
        }
    }

    pub fn due(&self) -> Option<u64> {
        let last = self.last?;
        Some(if self.waiting {
            last + self.interval
        } else {
            last + self.max_gap
        })
    }

    pub fn wrote(&mut self, now: u64) {
        self.last = Some(now);
        self.waiting = false;
    }

    pub fn hurry(&mut self) {
        self.waiting = true;
    }
}

pub const FADE_MS: u64 = 400;

#[derive(Clone, Copy, Debug)]
pub struct Fade {
    start: u64,
    length: u64,
}

impl Fade {
    pub fn new(start: u64, length: u64) -> Self {
        Self {
            start,
            length: length.max(1),
        }
    }

    /// How much of the old picture still shows at `now`: from 1 down to 0, eased at
    /// both ends. `None` — the dissolve is over.
    pub fn old(&self, now: u64) -> Option<f32> {
        let t = now.saturating_sub(self.start) as f32 / self.length as f32;
        (t < 1.0).then_some(1.0 - t * t * (3.0 - 2.0 * t))
    }
}

/// A stem like the screenshots' names: prefix, date and time ("<prefix> 2026-11-12 201530").
pub fn file_stem(stamp: &Stamp) -> String {
    stamp.named(&tr!("file.recording"))
}

/// Clipboard that takes files (a video can only be pasted as a file).
pub trait FileClipboard {
    fn put_file(&mut self, path: &Path) -> Result<(), Failed>;
}

/// Where a recording goes when it should stay only in the clipboard: out of the
/// recordings folder, but still on disk, because the clipboard holds a path.
pub trait Shelf {
    fn stash(&mut self, path: &Path) -> Result<PathBuf, Failed>;
}

pub struct Take {
    path: PathBuf,
    status: Status,
    duration_ms: u64,
    bytes: u64,
    size: (u32, u32),
}

impl Take {
    pub fn new(path: PathBuf, duration_ms: u64, bytes: u64, size: (u32, u32)) -> Self {
        Self {
            path,
            status: Status::Fresh,
            duration_ms,
            bytes,
            size,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn copy(&mut self, clipboard: &mut impl FileClipboard) -> Result<(), Failed> {
        clipboard.put_file(&self.path)?;
        if self.status == Status::Fresh {
            self.status = Status::Copied;
        }
        Ok(())
    }

    /// Ctrl+X: only in the clipboard — the file leaves the recordings folder.
    /// If it can't be moved, this is just a copy.
    pub fn cut(
        &mut self,
        clipboard: &mut impl FileClipboard,
        shelf: &mut impl Shelf,
    ) -> Result<(), Failed> {
        if self.status == Status::ClipboardOnly {
            return clipboard.put_file(&self.path);
        }
        let Ok(stashed) = shelf.stash(&self.path) else {
            return self.copy(clipboard);
        };
        self.path = stashed;
        self.status = Status::ClipboardOnly;
        // The file is already off the folder; a busy clipboard is retried by the next Ctrl+X.
        clipboard.put_file(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use choice::{Quality, Rate};

    fn planned(area: Area, monitor: (u32, u32), quality: Quality) -> Option<Plan> {
        let choice = Choice {
            quality,
            ..Choice::default()
        };
        Plan::new(area, monitor, &choice)
    }

    fn area(x: u32, y: u32, width: u32, height: u32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn whole_monitor_keeps_its_size() {
        let plan = planned(area(0, 0, 2496, 1664), (2496, 1664), Quality::High).expect("a plan");
        assert_eq!(plan.crop, area(0, 0, 2496, 1664));
        assert_eq!((plan.width, plan.height), (2496, 1664));
        assert_eq!(plan.fps, 30);
        assert_eq!(plan.codec, Codec::H264);
        let fast = Plan::new(
            area(0, 0, 1920, 1080),
            (1920, 1080),
            &Choice {
                rate: Rate::Sixty,
                codec: Codec::Hevc,
                ..Choice::default()
            },
        )
        .expect("a plan");
        assert_eq!((fast.fps, fast.codec), (60, Codec::Hevc));
    }

    #[test]
    fn sides_become_even_and_stay_on_the_monitor() {
        let plan = planned(area(101, 51, 999, 5000), (1920, 1080), Quality::High).expect("a plan");
        assert_eq!(plan.crop, area(101, 51, 998, 1028));
        assert_eq!((plan.width, plan.height), (998, 1028));
    }

    #[test]
    fn shrink_divides_the_video_not_the_crop() {
        let plan = planned(area(0, 0, 2496, 1664), (2496, 1664), Quality::Low).expect("a plan");
        assert_eq!(plan.crop, area(0, 0, 2496, 1664));
        assert_eq!((plan.width, plan.height), (1248, 832));
        let medium =
            planned(area(0, 0, 2560, 1440), (2560, 1440), Quality::Medium).expect("a plan");
        assert_eq!((medium.width, medium.height), (1920, 1080));
        let odd = planned(area(0, 0, 1366, 768), (1366, 768), Quality::Medium).expect("a plan");
        assert_eq!((odd.width, odd.height), (1024, 576));
    }

    #[test]
    fn hevc_sides_are_multiples_of_eight() {
        let hevc = |quality| Choice {
            codec: Codec::Hevc,
            quality,
            ..Choice::default()
        };
        let whole =
            Plan::new(area(0, 0, 1366, 768), (1366, 768), &hevc(Quality::High)).expect("a plan");
        assert_eq!((whole.width, whole.height), (1360, 768));
        assert_eq!((whole.crop.width, whole.crop.height), (1360, 768));
        let probe =
            Plan::new(area(0, 0, 1664, 1109), (1664, 1109), &hevc(Quality::High)).expect("a plan");
        assert_eq!(probe.height, 1104);
        let medium =
            Plan::new(area(0, 0, 1366, 768), (1366, 768), &hevc(Quality::Medium)).expect("a plan");
        assert_eq!((medium.width % 8, medium.height % 8), (0, 0));
    }

    #[test]
    fn nothing_to_record_off_the_monitor() {
        assert_eq!(
            planned(area(1910, 0, 100, 100), (1920, 1080), Quality::High),
            None
        );
        assert_eq!(planned(area(0, 0, 0, 0), (1920, 1080), Quality::High), None);
    }

    #[test]
    fn bitrate_grows_with_size_within_bounds() {
        let small = planned(area(0, 0, 320, 240), (1920, 1080), Quality::High).expect("a plan");
        let full = planned(area(0, 0, 1920, 1080), (1920, 1080), Quality::High).expect("a plan");
        let huge = planned(area(0, 0, 7680, 4320), (7680, 4320), Quality::High).expect("a plan");
        assert_eq!(small.bitrate(), 1_000_000);
        assert_eq!(full.bitrate(), 3_732_480);
        assert_eq!(huge.bitrate(), 40_000_000);
    }

    #[test]
    fn the_weight_follows_size_frames_and_codec() {
        let at = |quality, rate, codec| {
            Plan::new(
                area(0, 0, 2496, 1664),
                (2496, 1664),
                &Choice {
                    quality,
                    rate,
                    codec,
                    ..Choice::default()
                },
            )
            .expect("a plan")
            .megabytes_per_minute()
        };
        assert_eq!(at(Quality::High, Rate::Thirty, Codec::H264), 9);
        assert_eq!(at(Quality::Low, Rate::Thirty, Codec::H264), 4);
        assert_eq!(at(Quality::High, Rate::Sixty, Codec::H264), 11);
        assert_eq!(at(Quality::High, Rate::Thirty, Codec::Hevc), 7);
    }

    #[test]
    fn clock_skips_pauses() {
        let mut clock = Clock::start(1_000);
        assert_eq!(clock.elapsed(6_000), 5_000);
        clock.pause(6_000);
        assert!(clock.is_paused());
        assert_eq!(clock.elapsed(60_000), 5_000);
        clock.resume(10_000);
        assert_eq!(clock.elapsed(12_000), 7_000);
        clock.resume(13_000);
        clock.pause(14_000);
        clock.pause(15_000);
        clock.resume(16_000);
        assert_eq!(clock.elapsed(16_000), 9_000);
    }

    #[test]
    fn pacer_writes_at_most_fps_and_keeps_the_last_frame() {
        let mut pacer = Pacer::new(30, 3000);
        assert_eq!(pacer.due(), None);
        assert!(pacer.arrived(0));
        pacer.wrote(0);
        assert!(!pacer.arrived(40));
        assert!(!pacer.arrived(90));
        assert_eq!(pacer.due(), Some(100));
        pacer.wrote(100);
        assert!(pacer.arrived(250));
        pacer.wrote(250);
    }

    #[test]
    fn pacer_repeats_a_still_frame_once_a_second() {
        let mut pacer = Pacer::new(30, 3000);
        pacer.wrote(500);
        assert_eq!(pacer.due(), Some(3500));
    }

    #[test]
    fn hurried_pacer_wants_the_next_slot_on_a_still_screen() {
        let mut pacer = Pacer::new(30, 3000);
        pacer.wrote(500);
        pacer.hurry();
        assert_eq!(pacer.due(), Some(600));
        pacer.wrote(600);
        assert_eq!(pacer.due(), Some(3600));
    }

    #[test]
    fn fade_goes_from_the_old_picture_to_the_new() {
        let fade = Fade::new(1000, 400);
        assert_eq!(fade.old(1000), Some(1.0));
        assert_eq!(fade.old(1200), Some(0.5));
        let late = fade.old(1350).expect("still fading");
        assert!(late > 0.0 && late < 0.1);
        assert_eq!(fade.old(1400), None);
        assert_eq!(fade.old(5000), None);
    }

    #[test]
    fn stem_is_like_screenshots() {
        let stamp = Stamp {
            year: 2026,
            month: 11,
            day: 2,
            hour: 9,
            minute: 5,
            second: 7,
        };
        crate::lang::with(crate::lang::Lang::Ru, || {
            assert_eq!(file_stem(&stamp), "Запись экрана 2026-11-02 090507");
        });
        crate::lang::with(crate::lang::Lang::En, || {
            assert_eq!(file_stem(&stamp), "Screen recording 2026-11-02 090507");
        });
    }

    use crate::fault::Cause;

    #[derive(Default)]
    struct Board {
        busy: bool,
        files: Vec<PathBuf>,
    }

    impl FileClipboard for Board {
        fn put_file(&mut self, path: &Path) -> Result<(), Failed> {
            if self.busy {
                return Err(Failed::new(Cause::Busy));
            }
            self.files.push(path.to_path_buf());
            Ok(())
        }
    }

    struct Temp {
        stuck: bool,
    }

    impl Shelf for Temp {
        fn stash(&mut self, path: &Path) -> Result<PathBuf, Failed> {
            if self.stuck {
                return Err(Failed::new(Cause::Busy));
            }
            Ok(Path::new("temp").join(path.file_name().expect("a name")))
        }
    }

    fn take() -> Take {
        Take::new(
            PathBuf::from("rec").join("a.mp4"),
            42_000,
            14_000_000,
            (1920, 1080),
        )
    }

    #[test]
    fn copy_keeps_the_file_in_place() {
        let mut t = take();
        let mut board = Board::default();
        assert!(t.copy(&mut board).is_ok());
        assert_eq!(t.status(), Status::Copied);
        assert_eq!(board.files, vec![PathBuf::from("rec").join("a.mp4")]);
    }

    #[test]
    fn busy_clipboard_changes_nothing() {
        let mut t = take();
        let mut board = Board {
            busy: true,
            ..Board::default()
        };
        assert_eq!(t.copy(&mut board).map_err(|f| f.cause), Err(Cause::Busy));
        assert_eq!(t.status(), Status::Fresh);
    }

    #[test]
    fn cut_moves_the_file_off_the_folder() {
        let mut t = take();
        let mut board = Board::default();
        assert!(t.cut(&mut board, &mut Temp { stuck: false }).is_ok());
        assert_eq!(t.status(), Status::ClipboardOnly);
        assert_eq!(t.path(), Path::new("temp").join("a.mp4"));
        assert_eq!(board.files, vec![Path::new("temp").join("a.mp4")]);
        assert!(t.copy(&mut board).is_ok());
        assert_eq!(t.status(), Status::ClipboardOnly);
    }

    #[test]
    fn stuck_file_makes_cut_a_copy() {
        let mut t = take();
        let mut board = Board::default();
        assert!(t.cut(&mut board, &mut Temp { stuck: true }).is_ok());
        assert_eq!(t.status(), Status::Copied);
        assert_eq!(t.path(), PathBuf::from("rec").join("a.mp4"));
    }
}
