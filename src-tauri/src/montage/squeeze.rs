//! The compression modes of a save: the size, frame rate and bitrate the result is encoded at,
//! and about how large the file will be (the dropdown's estimates, the island's megabytes).

/// The sound is encoded again at the recordings' rate.
pub const SOUND_BITRATE: u32 = 96_000;
/// Feather's limit: the smallest common one (Discord without a subscription, many mail servers).
pub const FEATHER_BYTES: u64 = 10_000_000;
/// The encoder overshoots its mean bitrate a little and the container adds its headers.
const FEATHER_MARGIN: f64 = 0.92;
const CHAT_HEIGHT: u32 = 1080;
const CHAT_FPS: u32 = 30;
/// For chat the video keeps this share of the recording's bitrate, at most.
const CHAT_SHARE: f64 = 0.6;
const CHAT_BITS_PER_PIXEL: f64 = 0.04;
/// Below this text on screen smears: Feather lowers the size before the bitrate goes under it.
const MIN_BITS_PER_PIXEL: f64 = 0.015;
const MIN_BITRATE: u32 = 150_000;
const FEATHER_HEIGHTS: [u32; 4] = [1080, 720, 540, 360];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Squeeze {
    AsIs,
    Chat,
    Feather,
}

impl Squeeze {
    pub const ALL: [Self; 3] = [Self::AsIs, Self::Chat, Self::Feather];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bytes: u64,
    pub duration: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Video bits per second; `None` = the encoder's quality mode, as recordings are made.
    pub bitrate: Option<u32>,
    pub bytes: u64,
    /// Feather got under its limit (the other modes always "fit").
    pub fits: bool,
}

pub fn plan(squeeze: Squeeze, source: &Source, result: f64) -> Output {
    let total = source.bytes as f64 * 8.0 / source.duration.max(0.1);
    let bytes = |bits_per_second: f64| (bits_per_second * result / 8.0).round() as u64;
    match squeeze {
        Squeeze::AsIs => Output {
            width: source.width,
            height: source.height,
            fps: source.fps,
            bitrate: None,
            bytes: bytes(total),
            fits: true,
        },
        Squeeze::Chat => {
            let (width, height) = scaled(source, CHAT_HEIGHT);
            let fps = source.fps.min(CHAT_FPS);
            let video = (total - f64::from(SOUND_BITRATE)).max(f64::from(MIN_BITRATE));
            let ceiling =
                f64::from(width) * f64::from(height) * f64::from(fps) * CHAT_BITS_PER_PIXEL;
            let bitrate = (video * CHAT_SHARE)
                .min(ceiling)
                .max(f64::from(MIN_BITRATE)) as u32;
            Output {
                width,
                height,
                fps,
                bitrate: Some(bitrate),
                bytes: bytes(f64::from(bitrate + SOUND_BITRATE)),
                fits: true,
            }
        }
        Squeeze::Feather => {
            let chat = plan(Squeeze::Chat, source, result);
            let budget = FEATHER_BYTES as f64 * 8.0 * FEATHER_MARGIN / result.max(0.1);
            let video = budget - f64::from(SOUND_BITRATE);
            if video >= f64::from(chat.bitrate.unwrap_or(0)) {
                return chat;
            }
            let video = video.max(f64::from(MIN_BITRATE));
            let fps = chat.fps;
            let sizes = FEATHER_HEIGHTS
                .iter()
                .filter(|&&h| h <= chat.height)
                .map(|&h| scaled(source, h));
            let mut chosen = (chat.width, chat.height);
            for (width, height) in std::iter::once(chosen).chain(sizes) {
                chosen = (width, height);
                let per_pixel = video / (f64::from(width) * f64::from(height) * f64::from(fps));
                if per_pixel >= MIN_BITS_PER_PIXEL {
                    break;
                }
            }
            let bytes = bytes(video + f64::from(SOUND_BITRATE));
            Output {
                width: chosen.0,
                height: chosen.1,
                fps,
                bitrate: Some(video as u32),
                bytes,
                fits: bytes <= FEATHER_BYTES,
            }
        }
    }
}

/// The source's size brought down to `height` at most, sides even for H.264. Never up.
fn scaled(source: &Source, height: u32) -> (u32, u32) {
    if source.height <= height {
        return (source.width & !1, source.height & !1);
    }
    let width = (f64::from(source.width) * f64::from(height) / f64::from(source.height)).round();
    ((width as u32) & !1, height & !1)
}

pub fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Source = Source {
        width: 2560,
        height: 1440,
        fps: 60,
        bytes: 80_000_000,
        duration: 120.0,
    };

    #[test]
    fn as_is_keeps_everything_and_scales_with_the_result() {
        let out = plan(Squeeze::AsIs, &WIDE, 60.0);
        assert_eq!((out.width, out.height, out.fps), (2560, 1440, 60));
        assert_eq!(out.bitrate, None);
        assert_eq!(out.bytes, 40_000_000);
    }

    #[test]
    fn chat_is_full_hd_at_thirty_and_lighter_than_as_is() {
        let out = plan(Squeeze::Chat, &WIDE, 120.0);
        assert_eq!((out.width, out.height, out.fps), (1920, 1080, 30));
        assert!(out.bytes < plan(Squeeze::AsIs, &WIDE, 120.0).bytes);
    }

    #[test]
    fn chat_never_outweighs_a_quiet_recording() {
        let quiet = Source {
            bytes: 5_000_000,
            ..WIDE
        };
        let chat = plan(Squeeze::Chat, &quiet, 120.0);
        assert!(chat.bytes < plan(Squeeze::AsIs, &quiet, 120.0).bytes);
    }

    #[test]
    fn feather_fits_ten_megabytes() {
        let out = plan(Squeeze::Feather, &WIDE, 120.0);
        assert!(out.fits);
        assert!(out.bytes <= FEATHER_BYTES);
        assert!(out.height <= 1080);
    }

    #[test]
    fn feather_lowers_the_size_for_a_long_video() {
        let long = Source {
            bytes: 200_000_000,
            duration: 300.0,
            ..WIDE
        };
        let out = plan(Squeeze::Feather, &long, 300.0);
        assert!(out.height < 1080);
        assert!(out.fits);
    }

    #[test]
    fn feather_says_when_it_cannot_fit() {
        let hour = Source {
            bytes: 2_000_000_000,
            duration: 3600.0,
            ..WIDE
        };
        assert!(!plan(Squeeze::Feather, &hour, 3600.0).fits);
    }

    #[test]
    fn a_small_short_video_is_not_squeezed_more_than_chat() {
        let small = Source {
            width: 1280,
            height: 720,
            fps: 30,
            bytes: 3_000_000,
            duration: 20.0,
        };
        assert_eq!(
            plan(Squeeze::Feather, &small, 20.0),
            plan(Squeeze::Chat, &small, 20.0)
        );
        let chat = plan(Squeeze::Chat, &small, 20.0);
        assert_eq!((chat.width, chat.height), (1280, 720));
    }

    #[test]
    fn odd_sides_become_even() {
        let odd = Source {
            width: 1727,
            height: 1009,
            ..WIDE
        };
        let out = plan(Squeeze::Chat, &odd, 10.0);
        assert_eq!((out.width % 2, out.height % 2), (0, 0));
    }
}
