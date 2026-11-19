//! Icons for the player, control center and viewer window: vector shapes drawn by Direct2D. The player
//! icons repeat the mockup's SVG; the menu icons are drawn in the same manner. Drawn once into
//! a Composition surface; afterwards only the compositor moves them.

use windows::{
    UI::Color,
    Win32::Graphics::Direct2D::{
        Common::{
            D2D_RECT_F, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
            D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
        },
        D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND,
        D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE, D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT,
        D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
        D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE, ID2D1DeviceContext, ID2D1Factory,
        ID2D1PathGeometry, ID2D1SolidColorBrush,
    },
    core::Result,
};
use windows_numerics::{Matrix3x2, Vector2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Play,
    Pause,
    Previous,
    Next,
    Headphones,
    Speaker,
    Note,
    Video,
    // Control center, viewBox 20.
    Volume,
    Muted,
    Microphone,
    MicrophoneOff,
    /// Do Not Disturb.
    Moon,
    /// Keep awake.
    Coffee,
    Screenshot,
    RecordDot,
    RecordStop,
    Gear,
    Check,
    /// Cross: cancel a recording.
    Close,
    /// Points where to slide the cancel knob.
    Chevron,
    ChevronDown,
    Monitor,
    MonitorMic,
    Timer,
    More,
    /// Quality: a picture with a hill and the sun.
    Picture,
    /// Codec: a strip of film.
    Film,
    /// Frames per second: a speed gauge.
    Gauge,
    /// The dissolve after a pause: one frame over another.
    Dissolve,
    // Screenshot viewer window, viewBox 20.
    Folder,
    Copy,
    Pencil,
    Trash,
    /// Viewer status on the island: tuck the shot back into the island.
    ArrowUp,
    // Video viewer window, viewBox 20.
    Rewind,
    Forward,
    Replay,
    FullScreen,
    ExitFullScreen,
    /// The frame on screen becomes a screenshot.
    Camera,
    // Screenshot editor, viewBox 20.
    Back,
    Brush,
    Marker,
    Pointer,
    Frame,
    Pixels,
    Crop,
    Undo,
    Redo,
    // Video editor, viewBox 20.
    Scissors,
    ZoomIn,
    // A problem on the island. The shapes are filled; their "!" is drawn over them in black.
    /// Warning: a triangle, viewBox 16.
    Caution,
    CautionMark,
    /// Failure: a disc, viewBox 16.
    Disc,
    DiscMark,
    Bang,
    Problem,
    // System notices. The battery's parts share one square viewBox 32 with the body in its
    // middle band, so they stack into one battery and move apart for the animations.
    BatteryShell,
    BatteryTip,
    BatteryBolt,
    BatteryChevrons,
    /// Energy saver: takes the tip's place.
    BatteryPlus,
    /// The fast drain's gauge without its needle, viewBox 20; the needle turns on its own.
    Dial,
    Needle,
    /// High load: a processor, its core apart to pulse.
    Chip,
    ChipCore,
    /// The control center: on the charger, not charging.
    Plug,
    /// A download: the arrow drops into the tray on its own.
    DownloadArrow,
    DownloadTray,
    // A file without a picture of its own (`view/file_icon.rs`), viewBox 24: a sheet, its
    // folded corner and the band the extension sits on, each in its own color.
    Sheet,
    SheetFold,
    SheetBand,
    // The devices of the small device notices, in the manner of the headphones and the speaker.
    Mouse,
    Keyboard,
    Gamepad,
    Phone,
}

enum Part {
    /// A filled polygon; the corners are rounded by a stroke of the same color of this width.
    Poly(&'static [(f32, f32)], f32),
    /// A filled rectangle: x, y, width, height, corner radius.
    Rect(f32, f32, f32, f32, f32),
    Circle(f32, f32, f32),
    /// A line of `width` with round caps.
    Line(&'static [Seg], f32),
    /// Rectangle outline: x, y, width, height, corner radius, thickness.
    RectOutline(f32, f32, f32, f32, f32, f32),
    /// Circle outline: center, radius, thickness.
    Ring(f32, f32, f32, f32),
    Fill(&'static [Seg]),
}

#[derive(Clone, Copy)]
enum Seg {
    Move(f32, f32),
    To(f32, f32),
    /// Arc clockwise to the point, radius.
    ArcCw(f32, f32, f32),
    /// Large arc (more than a semicircle) counter-clockwise to the point, radius.
    ArcCcwLarge(f32, f32, f32),
}

use Part::*;
use Seg::*;

/// The battery parts' square viewBox; the body is 26 × 12 from (0, 10).
pub const BATTERY_VIEW: f32 = 32.0;

impl Glyph {
    fn view(self) -> f32 {
        match self {
            Self::Play | Self::Pause => 22.0,
            Self::Previous | Self::Next => 18.0,
            Self::Headphones | Self::Speaker | Self::Note | Self::Video => 16.0,
            Self::Mouse | Self::Keyboard | Self::Gamepad | Self::Phone => 16.0,
            Self::Caution | Self::CautionMark | Self::Disc | Self::DiscMark => 16.0,
            Self::BatteryShell
            | Self::BatteryTip
            | Self::BatteryBolt
            | Self::BatteryChevrons
            | Self::BatteryPlus => BATTERY_VIEW,
            Self::Dial | Self::Needle | Self::Chip | Self::ChipCore => 20.0,
            Self::Sheet | Self::SheetFold | Self::SheetBand => 24.0,
            _ => 20.0,
        }
    }

    fn parts(self) -> &'static [Part] {
        match self {
            Self::BatteryShell => &[RectOutline(0.75, 10.75, 21.5, 10.5, 3.0, 1.5)],
            Self::BatteryTip => &[Rect(23.1, 13.8, 1.9, 4.4, 0.95)],
            // Taller than the body: it sticks out above and below.
            Self::BatteryBolt => &[Poly(
                &[
                    (14.2, 7.4),
                    (7.6, 17.0),
                    (11.9, 17.0),
                    (10.0, 23.6),
                    (16.8, 13.8),
                    (12.4, 13.8),
                ],
                0.0,
            )],
            Self::BatteryChevrons => &[Line(
                &[
                    Move(4.6, 12.4),
                    To(8.0, 16.0),
                    To(4.6, 19.6),
                    Move(9.6, 12.4),
                    To(13.0, 16.0),
                    To(9.6, 19.6),
                ],
                1.5,
            )],
            Self::BatteryPlus => &[Line(
                &[
                    Move(27.0, 12.6),
                    To(27.0, 19.4),
                    Move(23.6, 16.0),
                    To(30.4, 16.0),
                ],
                1.8,
            )],
            // Over the top through its highest point: two small arcs make the large one.
            Self::Dial => &[Line(
                &[
                    Move(3.2, 14.5),
                    ArcCw(10.0, 3.51, 7.6),
                    ArcCw(16.8, 14.5, 7.6),
                ],
                1.8,
            )],
            Self::Needle => &[
                Line(&[Move(10.0, 12.2), To(14.2, 6.6)], 1.8),
                Circle(10.0, 12.2, 1.7),
            ],
            Self::Chip => &[
                RectOutline(4.5, 4.5, 11.0, 11.0, 2.4, 1.7),
                Line(
                    &[
                        Move(8.0, 2.0),
                        To(8.0, 4.0),
                        Move(12.0, 2.0),
                        To(12.0, 4.0),
                        Move(8.0, 16.0),
                        To(8.0, 18.0),
                        Move(12.0, 16.0),
                        To(12.0, 18.0),
                        Move(2.0, 8.0),
                        To(4.0, 8.0),
                        Move(2.0, 12.0),
                        To(4.0, 12.0),
                        Move(16.0, 8.0),
                        To(18.0, 8.0),
                        Move(16.0, 12.0),
                        To(18.0, 12.0),
                    ],
                    1.4,
                ),
            ],
            Self::ChipCore => &[Rect(7.6, 7.6, 4.8, 4.8, 1.0)],
            Self::Plug => &[
                Line(
                    &[Move(7.6, 2.6), To(7.6, 6.6), Move(12.4, 2.6), To(12.4, 6.6)],
                    1.8,
                ),
                Rect(4.6, 6.4, 10.8, 4.4, 1.0),
                Poly(&[(4.6, 10.0), (15.4, 10.0), (12.4, 14.4), (7.6, 14.4)], 1.0),
                Line(&[Move(10.0, 14.0), To(10.0, 17.6)], 1.8),
            ],
            Self::DownloadArrow => &[Line(
                &[
                    Move(10.0, 2.8),
                    To(10.0, 11.8),
                    Move(6.4, 8.4),
                    To(10.0, 12.0),
                    To(13.6, 8.4),
                ],
                1.9,
            )],
            Self::DownloadTray => &[Line(
                &[
                    Move(3.4, 12.6),
                    To(3.4, 16.8),
                    To(16.6, 16.8),
                    To(16.6, 12.6),
                ],
                1.9,
            )],
            Self::Sheet => &[Poly(
                &[
                    (4.5, 1.5),
                    (14.5, 1.5),
                    (20.0, 7.0),
                    (20.0, 22.5),
                    (4.5, 22.5),
                ],
                1.5,
            )],
            Self::SheetFold => &[Poly(&[(14.5, 1.5), (14.5, 7.0), (20.0, 7.0)], 1.0)],
            Self::SheetBand => &[Rect(3.75, 13.5, 17.0, 6.5, 0.0)],
            Self::Mouse => &[
                RectOutline(4.5, 1.5, 7.0, 13.0, 3.5, 1.3),
                Line(&[Move(8.0, 4.0), To(8.0, 6.4)], 1.3),
            ],
            Self::Keyboard => &[
                RectOutline(1.0, 4.0, 14.0, 8.5, 1.8, 1.3),
                Circle(4.0, 6.9, 0.75),
                Circle(6.67, 6.9, 0.75),
                Circle(9.33, 6.9, 0.75),
                Circle(12.0, 6.9, 0.75),
                Line(&[Move(5.2, 9.8), To(10.8, 9.8)], 1.3),
            ],
            Self::Gamepad => &[
                RectOutline(1.0, 4.0, 14.0, 8.5, 4.25, 1.3),
                Line(
                    &[
                        Move(4.9, 6.6),
                        To(4.9, 9.9),
                        Move(3.25, 8.25),
                        To(6.55, 8.25),
                    ],
                    1.1,
                ),
                Circle(10.9, 7.4, 0.8),
                Circle(12.5, 9.1, 0.8),
            ],
            Self::Phone => &[
                RectOutline(4.5, 1.5, 7.0, 13.0, 1.8, 1.3),
                Line(&[Move(7.0, 12.2), To(9.0, 12.2)], 1.1),
            ],
            Self::Play => &[Poly(&[(7.6, 4.4), (7.6, 17.6), (18.0, 11.0)], 1.6)],
            Self::Pause => &[
                Rect(5.5, 4.0, 4.0, 14.0, 1.2),
                Rect(12.5, 4.0, 4.0, 14.0, 1.2),
            ],
            Self::Previous => &[
                Rect(3.0, 3.5, 2.2, 11.0, 1.0),
                Poly(&[(14.4, 4.4), (14.4, 13.6), (7.4, 9.0)], 1.2),
            ],
            Self::Next => &[
                Rect(12.8, 3.5, 2.2, 11.0, 1.0),
                Poly(&[(3.6, 4.4), (3.6, 13.6), (10.6, 9.0)], 1.2),
            ],
            Self::Headphones => &[
                Line(
                    &[
                        Move(2.5, 10.0),
                        To(2.5, 8.0),
                        ArcCw(13.5, 8.0, 5.5),
                        To(13.5, 10.0),
                    ],
                    1.3,
                ),
                Rect(2.0, 9.0, 3.0, 5.0, 1.2),
                Rect(11.0, 9.0, 3.0, 5.0, 1.2),
            ],
            Self::Speaker => &[
                Poly(
                    &[
                        (2.5, 6.0),
                        (5.0, 6.0),
                        (8.5, 3.0),
                        (8.5, 13.0),
                        (5.0, 10.0),
                        (2.5, 10.0),
                    ],
                    0.0,
                ),
                Line(&[Move(10.6, 5.6), ArcCw(10.6, 10.4, 3.4)], 1.3),
                Line(&[Move(12.4, 3.8), ArcCw(12.4, 12.2, 6.0)], 1.3),
            ],
            Self::Note => &[
                Line(
                    &[Move(6.0, 12.0), To(6.0, 3.2), To(13.0, 1.8), To(13.0, 10.2)],
                    1.5,
                ),
                Circle(4.3, 12.0, 2.1),
                Circle(11.3, 10.4, 2.1),
            ],
            Self::Video => &[
                RectOutline(1.5, 3.5, 13.0, 9.0, 2.0, 1.4),
                Poly(&[(6.6, 6.0), (6.6, 10.0), (10.0, 8.0)], 0.0),
            ],
            Self::Volume => &[
                SPEAKER_BODY,
                Line(&[Move(12.3, 7.2), ArcCw(12.3, 12.8, 3.9)], 1.6),
                Line(&[Move(14.6, 4.6), ArcCw(14.6, 15.4, 7.2)], 1.6),
            ],
            Self::Muted => &[
                SPEAKER_BODY,
                Line(
                    &[
                        Move(12.5, 7.5),
                        To(17.5, 12.5),
                        Move(17.5, 7.5),
                        To(12.5, 12.5),
                    ],
                    1.6,
                ),
            ],
            Self::Microphone => &[MIC_CAPSULE, MIC_STAND],
            Self::MicrophoneOff => &[
                MIC_CAPSULE,
                MIC_STAND,
                Line(&[Move(3.5, 2.5), To(16.5, 17.5)], 1.6),
            ],
            // Crescent: the outer arc of the circle (10, 10) r 7 and the inner arc of the circle (14, 6.5) r 6.
            Self::Moon => &[Fill(&[
                Move(9.08, 3.06),
                ArcCcwLarge(16.76, 11.83, 7.0),
                ArcCw(9.08, 3.06, 6.0),
            ])],
            Self::Coffee => &[
                Rect(3.0, 8.0, 10.5, 8.0, 2.5),
                Line(&[Move(13.5, 9.5), ArcCw(13.5, 14.0, 2.25)], 1.6),
                Line(
                    &[Move(6.5, 2.5), To(6.5, 5.0), Move(10.0, 2.5), To(10.0, 5.0)],
                    1.6,
                ),
            ],
            Self::Screenshot => &[Line(
                &[
                    Move(3.0, 7.5),
                    To(3.0, 3.0),
                    To(7.5, 3.0),
                    Move(12.5, 3.0),
                    To(17.0, 3.0),
                    To(17.0, 7.5),
                    Move(17.0, 12.5),
                    To(17.0, 17.0),
                    To(12.5, 17.0),
                    Move(7.5, 17.0),
                    To(3.0, 17.0),
                    To(3.0, 12.5),
                ],
                1.7,
            )],
            Self::RecordDot => &[Circle(10.0, 10.0, 5.5)],
            Self::RecordStop => &[Rect(5.5, 5.5, 9.0, 9.0, 2.0)],
            Self::Check => &[Line(&[Move(5.0, 10.4), To(8.6, 14.0), To(15.0, 6.6)], 2.4)],
            Self::Close => &[Line(
                &[
                    Move(5.5, 5.5),
                    To(14.5, 14.5),
                    Move(14.5, 5.5),
                    To(5.5, 14.5),
                ],
                2.2,
            )],
            Self::Chevron => &[Line(&[Move(8.0, 5.0), To(13.0, 10.0), To(8.0, 15.0)], 2.4)],
            Self::ChevronDown => &[Line(&[Move(4.5, 8.0), To(10.0, 13.5), To(15.5, 8.0)], 2.6)],
            Self::Monitor => &[
                RectOutline(2.5, 3.5, 15.0, 10.0, 2.0, 1.5),
                Line(
                    &[
                        Move(7.0, 16.5),
                        To(13.0, 16.5),
                        Move(10.0, 13.5),
                        To(10.0, 16.5),
                    ],
                    1.5,
                ),
            ],
            // A small screen and a small microphone side by side: both inputs.
            Self::MonitorMic => &[
                RectOutline(1.5, 4.5, 10.0, 7.5, 1.6, 1.4),
                Line(
                    &[
                        Move(4.5, 15.5),
                        To(8.5, 15.5),
                        Move(6.5, 12.0),
                        To(6.5, 15.5),
                    ],
                    1.4,
                ),
                Rect(14.0, 3.0, 4.0, 7.5, 2.0),
                Line(
                    &[
                        Move(19.4, 8.5),
                        ArcCw(12.6, 8.5, 3.4),
                        Move(16.0, 12.0),
                        To(16.0, 15.5),
                    ],
                    1.4,
                ),
            ],
            Self::Timer => &[
                Ring(10.0, 11.0, 6.5, 1.5),
                Line(
                    &[
                        Move(10.0, 11.0),
                        To(10.0, 7.6),
                        Move(8.0, 2.6),
                        To(12.0, 2.6),
                    ],
                    1.5,
                ),
            ],
            Self::Picture => &[
                RectOutline(2.5, 3.5, 15.0, 13.0, 2.5, 1.5),
                Line(
                    &[
                        Move(5.5, 13.5),
                        To(8.8, 9.6),
                        To(11.2, 12.2),
                        To(12.8, 10.6),
                        To(14.8, 13.5),
                    ],
                    1.5,
                ),
                Circle(13.2, 7.3, 1.4),
            ],
            Self::Film => &[
                RectOutline(3.0, 3.0, 14.0, 14.0, 2.5, 1.5),
                Line(
                    &[
                        Move(7.0, 3.0),
                        To(7.0, 17.0),
                        Move(13.0, 3.0),
                        To(13.0, 17.0),
                        Move(3.0, 10.0),
                        To(7.0, 10.0),
                        Move(13.0, 10.0),
                        To(17.0, 10.0),
                    ],
                    1.3,
                ),
            ],
            Self::Gauge => &[
                Line(&[Move(3.0, 14.0), ArcCw(17.0, 14.0, 7.0)], 1.6),
                Line(&[Move(10.0, 14.0), To(13.6, 9.2)], 1.6),
                Circle(10.0, 14.0, 1.6),
            ],
            Self::Dissolve => &[
                RectOutline(2.8, 2.8, 9.4, 9.4, 2.2, 1.5),
                Rect(7.8, 7.8, 9.4, 9.4, 2.2),
            ],
            Self::More => &[
                Circle(4.4, 10.0, 1.75),
                Circle(10.0, 10.0, 1.75),
                Circle(15.6, 10.0, 1.75),
            ],
            Self::Folder => &[Line(
                &[
                    Move(2.8, 15.6),
                    To(2.8, 4.4),
                    To(7.6, 4.4),
                    To(9.4, 6.4),
                    To(17.2, 6.4),
                    To(17.2, 15.6),
                    To(2.8, 15.6),
                    Move(2.8, 8.8),
                    To(17.2, 8.8),
                ],
                1.5,
            )],
            // The front sheet in full; the back one shows its top and left edge.
            Self::Copy => &[
                RectOutline(7.2, 6.8, 9.8, 10.4, 2.0, 1.5),
                Line(
                    &[
                        Move(4.0, 12.8),
                        To(4.0, 5.2),
                        ArcCw(6.0, 3.2, 2.0),
                        To(12.6, 3.2),
                    ],
                    1.5,
                ),
            ],
            Self::Pencil => &[Line(
                &[
                    Move(3.5, 16.5),
                    To(4.6, 12.6),
                    To(13.1, 4.1),
                    To(15.9, 6.9),
                    To(7.4, 15.4),
                    To(3.5, 16.5),
                    Move(11.4, 5.8),
                    To(14.2, 8.6),
                ],
                1.5,
            )],
            Self::Trash => &[Line(
                &[
                    Move(3.5, 5.5),
                    To(16.5, 5.5),
                    Move(8.0, 5.5),
                    To(8.0, 3.5),
                    To(12.0, 3.5),
                    To(12.0, 5.5),
                    Move(5.2, 5.5),
                    To(6.0, 16.5),
                    To(14.0, 16.5),
                    To(14.8, 5.5),
                    Move(8.5, 8.6),
                    To(8.7, 13.4),
                    Move(11.5, 8.6),
                    To(11.3, 13.4),
                ],
                1.5,
            )],
            Self::ArrowUp => &[Line(
                &[
                    Move(10.0, 16.0),
                    To(10.0, 4.5),
                    Move(5.0, 9.5),
                    To(10.0, 4.5),
                    To(15.0, 9.5),
                ],
                1.8,
            )],
            Self::Rewind => &[
                Poly(&[(9.6, 5.2), (9.6, 14.8), (2.8, 10.0)], 1.4),
                Poly(&[(16.8, 5.2), (16.8, 14.8), (10.0, 10.0)], 1.4),
            ],
            Self::Forward => &[
                Poly(&[(3.2, 5.2), (3.2, 14.8), (10.0, 10.0)], 1.4),
                Poly(&[(10.4, 5.2), (10.4, 14.8), (17.2, 10.0)], 1.4),
            ],
            Self::Replay => &[
                Line(&[Move(4.0, 10.5), ArcCcwLarge(10.0, 4.5, 6.0)], 1.7),
                Line(&[Move(12.2, 2.4), To(10.0, 4.5), To(12.2, 6.6)], 1.7),
            ],
            Self::FullScreen => &[Line(
                &[
                    Move(3.5, 7.5),
                    To(3.5, 3.5),
                    To(7.5, 3.5),
                    Move(12.5, 3.5),
                    To(16.5, 3.5),
                    To(16.5, 7.5),
                    Move(16.5, 12.5),
                    To(16.5, 16.5),
                    To(12.5, 16.5),
                    Move(7.5, 16.5),
                    To(3.5, 16.5),
                    To(3.5, 12.5),
                ],
                1.7,
            )],
            Self::ExitFullScreen => &[Line(
                &[
                    Move(7.5, 3.5),
                    To(7.5, 7.5),
                    To(3.5, 7.5),
                    Move(12.5, 3.5),
                    To(12.5, 7.5),
                    To(16.5, 7.5),
                    Move(16.5, 12.5),
                    To(12.5, 12.5),
                    To(12.5, 16.5),
                    Move(7.5, 16.5),
                    To(7.5, 12.5),
                    To(3.5, 12.5),
                ],
                1.7,
            )],
            Self::Camera => &[
                RectOutline(2.5, 5.5, 15.0, 11.0, 2.5, 1.5),
                Ring(10.0, 11.0, 3.0, 1.5),
                Line(
                    &[Move(6.8, 5.5), To(8.0, 3.5), To(12.0, 3.5), To(13.2, 5.5)],
                    1.5,
                ),
            ],
            Self::Back => &[Line(&[Move(12.5, 4.5), To(7.0, 10.0), To(12.5, 15.5)], 1.8)],
            Self::Brush => &[Line(
                &[
                    Move(3.0, 14.0),
                    To(6.5, 7.0),
                    To(10.0, 13.0),
                    To(13.5, 7.0),
                    To(17.0, 12.5),
                ],
                1.7,
            )],
            Self::Marker => &[
                Line(&[Move(6.0, 13.0), To(14.5, 4.5)], 5.0),
                Line(&[Move(3.0, 17.0), To(17.0, 17.0)], 1.3),
            ],
            Self::Pointer => &[Line(
                &[
                    Move(4.5, 15.5),
                    To(15.0, 5.0),
                    Move(8.5, 5.0),
                    To(15.0, 5.0),
                    To(15.0, 11.5),
                ],
                1.7,
            )],
            Self::Frame => &[RectOutline(3.5, 5.0, 13.0, 10.0, 2.0, 1.6)],
            Self::Pixels => &[
                Rect(3.0, 3.0, 6.5, 6.5, 1.2),
                Rect(10.5, 10.5, 6.5, 6.5, 1.2),
                RectOutline(11.0, 3.5, 5.5, 5.5, 1.0, 1.2),
                RectOutline(3.5, 11.0, 5.5, 5.5, 1.0, 1.2),
            ],
            Self::Crop => &[Line(
                &[
                    Move(6.0, 2.5),
                    To(6.0, 14.0),
                    To(17.5, 14.0),
                    Move(2.5, 6.0),
                    To(14.0, 6.0),
                    To(14.0, 17.5),
                ],
                1.6,
            )],
            // Curved arrows: back is left and down along the arc on the right, forward is the mirror image.
            Self::Undo => &[Line(
                &[
                    Move(7.5, 4.5),
                    To(4.0, 8.0),
                    To(7.5, 11.5),
                    Move(4.5, 8.0),
                    To(12.0, 8.0),
                    ArcCw(12.0, 16.0, 4.0),
                    To(8.0, 16.0),
                ],
                1.6,
            )],
            Self::Caution => &[Poly(&[(8.0, 2.9), (14.3, 13.0), (1.7, 13.0)], 1.9)],
            Self::CautionMark => &[Rect(7.2, 5.4, 1.6, 4.6, 0.8), Circle(8.0, 11.9, 0.95)],
            Self::Disc => &[Circle(8.0, 8.0, 7.0)],
            Self::DiscMark => &[Rect(7.2, 3.9, 1.6, 5.2, 0.8), Circle(8.0, 11.4, 0.95)],
            Self::Bang => &[Rect(8.9, 3.5, 2.2, 9.0, 1.1), Circle(10.0, 15.6, 1.4)],
            Self::Problem => &[
                Ring(10.0, 10.0, 7.6, 1.6),
                Rect(9.1, 5.4, 1.8, 6.0, 0.9),
                Circle(10.0, 14.0, 1.1),
            ],
            Self::Redo => &[Line(
                &[
                    Move(12.0, 16.0),
                    To(8.0, 16.0),
                    ArcCw(8.0, 8.0, 4.0),
                    To(15.5, 8.0),
                    Move(12.5, 4.5),
                    To(16.0, 8.0),
                    To(12.5, 11.5),
                ],
                1.6,
            )],
            Self::Scissors => &[
                Ring(5.6, 14.4, 2.5, 1.7),
                Ring(14.4, 14.4, 2.5, 1.7),
                Line(
                    &[
                        Move(7.3, 12.4),
                        To(15.0, 3.2),
                        Move(12.7, 12.4),
                        To(5.0, 3.2),
                    ],
                    1.7,
                ),
            ],
            Self::ZoomIn => &[
                Ring(8.8, 8.8, 5.2, 1.7),
                Line(
                    &[
                        Move(12.8, 12.8),
                        To(16.8, 16.8),
                        Move(6.3, 8.8),
                        To(11.3, 8.8),
                        Move(8.8, 6.3),
                        To(8.8, 11.3),
                    ],
                    1.7,
                ),
            ],
            Self::Gear => &[
                Ring(10.0, 10.0, 5.0, 1.8),
                Ring(10.0, 10.0, 1.9, 1.6),
                Line(
                    &[
                        Move(15.9, 10.0),
                        To(17.4, 10.0),
                        Move(14.17, 14.17),
                        To(15.23, 15.23),
                        Move(10.0, 15.9),
                        To(10.0, 17.4),
                        Move(5.83, 14.17),
                        To(4.77, 15.23),
                        Move(4.1, 10.0),
                        To(2.6, 10.0),
                        Move(5.83, 5.83),
                        To(4.77, 4.77),
                        Move(10.0, 4.1),
                        To(10.0, 2.6),
                        Move(14.17, 5.83),
                        To(15.23, 4.77),
                    ],
                    2.6,
                ),
            ],
        }
    }
}

const SPEAKER_BODY: Part = Poly(
    &[
        (2.5, 7.5),
        (5.5, 7.5),
        (9.5, 4.0),
        (9.5, 16.0),
        (5.5, 12.5),
        (2.5, 12.5),
    ],
    1.2,
);
const MIC_CAPSULE: Part = Rect(7.0, 2.0, 6.0, 10.0, 3.0);
const MIC_STAND: Part = Line(
    &[
        Move(15.5, 9.5),
        ArcCw(4.5, 9.5, 5.5),
        Move(10.0, 15.0),
        To(10.0, 17.5),
        Move(7.0, 17.5),
        To(13.0, 17.5),
    ],
    1.6,
);

fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// `base` is the context's current transform (offset within the surface atlas); the icon is scaled on top of it.
pub fn draw(
    dc: &ID2D1DeviceContext,
    base: &Matrix3x2,
    glyph: Glyph,
    size: f32,
    color: Color,
) -> Result<()> {
    unsafe {
        let k = size / glyph.view();
        let transform = Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: 0.0,
            M32: 0.0,
        } * *base;
        dc.SetTransform(&transform);
        let brush = dc.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: color.R as f32 / 255.0,
                g: color.G as f32 / 255.0,
                b: color.B as f32 / 255.0,
                a: color.A as f32 / 255.0,
            },
            None,
        )?;
        let factory = dc.GetFactory()?;
        let round = factory.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 1.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            },
            None,
        )?;
        for part in glyph.parts() {
            draw_part(dc, &factory, &brush, &round, part)?;
        }
        dc.SetTransform(base);
        Ok(())
    }
}

unsafe fn draw_part(
    dc: &ID2D1DeviceContext,
    factory: &ID2D1Factory,
    brush: &ID2D1SolidColorBrush,
    round: &windows::Win32::Graphics::Direct2D::ID2D1StrokeStyle,
    part: &Part,
) -> Result<()> {
    unsafe {
        match *part {
            Poly(points, corner) => {
                let path = polygon(factory, points)?;
                dc.FillGeometry(&path, brush, None);
                if corner > 0.0 {
                    dc.DrawGeometry(&path, brush, corner, round);
                }
            }
            Rect(x, y, w, h, r) => dc.FillRoundedRectangle(&rounded(x, y, w, h, r), brush),
            Circle(cx, cy, r) => dc.FillEllipse(
                &D2D1_ELLIPSE {
                    point: v(cx, cy),
                    radiusX: r,
                    radiusY: r,
                },
                brush,
            ),
            Line(segments, width) => {
                let path = path(factory, segments, false)?;
                dc.DrawGeometry(&path, brush, width, round);
            }
            RectOutline(x, y, w, h, r, width) => {
                dc.DrawRoundedRectangle(&rounded(x, y, w, h, r), brush, width, round)
            }
            Ring(cx, cy, r, width) => dc.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: v(cx, cy),
                    radiusX: r,
                    radiusY: r,
                },
                brush,
                width,
                None,
            ),
            Fill(segments) => dc.FillGeometry(&path(factory, segments, true)?, brush, None),
        }
        Ok(())
    }
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        radiusX: r,
        radiusY: r,
    }
}

fn polygon(factory: &ID2D1Factory, points: &[(f32, f32)]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        let (x, y) = points[0];
        sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
        for &(x, y) in &points[1..] {
            sink.AddLine(v(x, y));
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(path)
    }
}

fn path(factory: &ID2D1Factory, segments: &[Seg], filled: bool) -> Result<ID2D1PathGeometry> {
    let (begin, end) = if filled {
        (D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED)
    } else {
        (D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN)
    };
    let arc = |x: f32, y: f32, r: f32, sweep, size| D2D1_ARC_SEGMENT {
        point: v(x, y),
        size: windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_F {
            width: r,
            height: r,
        },
        rotationAngle: 0.0,
        sweepDirection: sweep,
        arcSize: size,
    };
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        let mut open = false;
        for seg in segments {
            match *seg {
                Move(x, y) => {
                    if open {
                        sink.EndFigure(end);
                    }
                    sink.BeginFigure(v(x, y), begin);
                    open = true;
                }
                To(x, y) => sink.AddLine(v(x, y)),
                ArcCw(x, y, r) => sink.AddArc(&arc(
                    x,
                    y,
                    r,
                    D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    D2D1_ARC_SIZE_SMALL,
                )),
                ArcCcwLarge(x, y, r) => sink.AddArc(&arc(
                    x,
                    y,
                    r,
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    D2D1_ARC_SIZE_LARGE,
                )),
            }
        }
        if open {
            sink.EndFigure(end);
        }
        sink.Close()?;
        Ok(path)
    }
}
