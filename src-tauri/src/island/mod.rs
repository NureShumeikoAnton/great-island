//! The island: a native Win32 + Composition window. Everything runs on one thread with the Windows
//! message loop, so no locks or synchronization are needed.
//!
//! Once a second the scheduler ticks and the labels update. The capsule and ring animations are computed
//! by the Windows compositor: the process sleeps meanwhile.
//!
//! This file is the switchboard: the state ([`App`]), start and stop, and which handler gets
//! a window message or a timer. What a handler does lives with its feature:
//!
//! - `reminders`, `player`, `controls`, `screenshot`, `record`, `record_panel`, `watch`, `problem`, `notify`,
//!   `downloads`, `devices`, `open`, `explorer` — a feature each: its state and what it does on the island;
//! - `capture` — what the screenshot and the recording share: the source, hiding the island
//!   from captures, the card; `select` — the area selection both use; `hotkeys` — every
//!   global key; `dialogs` — the dialog windows every feature opens through;
//! - `input` — pointer and gestures; `render` — which view is on and its model; `layout` —
//!   where the window sits; `visibility` — when it is on screen; `config` — the settings file;
//! - `view`, `viewer`, `dialog`, `flight`, `selector`, `marks`, `overlay`, `tray`, `window`,
//!   `comp`, `paragraph`, `glyph` — drawing and Win32. They don't decide anything: they return a request and the
//!   feature carries it out.

mod capture;
mod comp;
mod config;
mod controls;
mod devices;
mod dialog;
mod dialogs;
mod downloads;
mod explorer;
mod flight;
mod glyph;
mod hotkeys;
mod input;
mod layout;
#[cfg(debug_assertions)]
mod levels_debug;
mod marks;
mod notify;
mod open;
mod overlay;
mod paragraph;
mod player;
mod problem;
mod record;
mod record_panel;
mod reminders;
mod render;
mod screenshot;
mod select;
mod selector;
mod tray;
mod update;
mod view;
mod viewer;
mod visibility;
mod watch;
mod watch_save;
mod window;

use crate::{
    control::{ControlCenter, ControlId},
    crash,
    fault::{Cause, Failed, Fault, Level, What},
    journal,
    lang::{self, tr},
    persist::{self, Hidden, SavedState},
    platform::{self, MonitorId},
    scheduler::Scheduler,
    settings::{Language, Settings},
    stage::Stage,
};
use devices::Devices;
use dialogs::Dialogs;
use downloads::Downloads;
use explorer::Explorer;
use hotkeys::Hotkeys;
use input::{Pointer, point, wheel_delta};
use notify::Notify;
use open::{Arrival, Opened};
use player::Player;
use problem::Problems;
use record::Recording;
use record_panel::RecordPanel;
use reminders::Reminders;
use screenshot::Shots;
use select::Select;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant, SystemTime},
};
use tray::{Command, HiddenState, Tray};
use update::Updates;
use view::Scene;
use watch::Watch;
use window::{
    WM_CONTROLS, WM_DEMO, WM_DEVICES, WM_DOWNLOAD, WM_EXPLORER, WM_FAULT, WM_MEDIA,
    WM_OPEN_SETTINGS, WM_OPENED, WM_RECORD, WM_SAVE, WM_UPDATE,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{
                DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, KillTimer,
                MA_NOACTIVATE, MSG, PostMessageW, PostQuitMessage, RegisterWindowMessageW,
                SetTimer, TranslateMessage, WM_CAPTURECHANGED, WM_COPYDATA, WM_DISPLAYCHANGE,
                WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_INPUT, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MOUSEACTIVATE, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCACTIVATE,
                WM_NCCALCSIZE, WM_NCLBUTTONDOWN, WM_POWERBROADCAST, WM_QUERYENDSESSION,
                WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER,
            },
        },
    },
    core::w,
};

/// Not in the windows crate: the cursor left the window (after TrackMouseEvent).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// WM_COPYDATA's tag for a file a second start hands over ("Open with").
const OPEN_FILE: usize = 0x4F50_454E;

/// WM_DEMO's wparam for a system notice; 0 and 1 are a warning and a failure.
const DEMO_SYSTEM: usize = 2;
const DEMO_DEVICES: usize = 3;
const DEMO_UPDATE: usize = 4;

const TICK_MS: u32 = 1000;
const SAVE_EVERY_TICKS: u32 = 15;

mod timers {
    pub const TICK: usize = 1;
    pub const HOVER: usize = 2;
    pub const SHRINK: usize = 3;
    pub const PURGE: usize = 4;
    pub const SKIP: usize = 5;
    pub const HIDE: usize = 6;
    pub const DIM: usize = 7;
    /// A notice expires or it is time to confirm the player.
    pub const STAGE: usize = 8;
    /// Hold, or the end of a touchpad gesture.
    pub const GESTURE: usize = 9;
    pub const BARS: usize = 10;
    pub const MENU: usize = 11;
    pub const SHUTTER: usize = 12;
    pub const FLIGHT: usize = 13;
    pub const SHOT_CLOSE: usize = 14;
    /// The next step of meeting a screenshot: capsule for the thumbnail, the shot landed, the card.
    pub const LANDING: usize = 15;
    pub const VIEWER_SHOW: usize = 16;
    pub const VIEWER_HIDE: usize = 17;
    /// The shot leaves the center for the island: a click on it no longer opens the window.
    pub const CATCH: usize = 18;
    /// The arrow in the viewer status was held: the shot goes to the island.
    pub const HOLD: usize = 19;
    pub const COUNTDOWN: usize = 20;
    pub const CLIP_CLOSE: usize = 21;
    pub const REPLAY: usize = 22;
    pub const COPIED: usize = 23;
    /// The control center's Screenshot button is held: the area selection opens.
    pub const BUTTON_HOLD: usize = 24;
    /// The bars of the panel's microphone.
    pub const METER: usize = 25;
    /// Alt is held in the area selection.
    pub const SELECT: usize = 26;
    pub const MARKS_GONE: usize = 27;
    pub const WATCH_SHOW: usize = 28;
    pub const WATCH_HIDE: usize = 29;
    /// The spinner of an update check has not turned once yet: the answer waits for it.
    pub const UPDATE: usize = 30;
    /// The system notifications' minute: the processes and the battery.
    pub const NOTIFY: usize = 31;
    /// The first low step waits for Energy saver.
    pub const SAVER_WAIT: usize = 32;
    /// The control center's clock: the next second.
    pub const CLOCK: usize = 33;
    /// A device notice waits for the battery, or a sound switch for its device's link.
    pub const DEVICE_WAIT: usize = 34;
    /// A press ended past the island: it stays open until the cursor comes back, or this long.
    pub const OUTSIDE: usize = 35;
    /// A closed dialog has faded out.
    pub const DIALOG_GONE: usize = 36;
    /// The news of Explorer's selection has settled: it is read again.
    pub const FOLLOW: usize = 37;
}

const INSTANCE: &str = "Local\\GreatIsland.Island";
/// An island started to replace the running one waits this long for it to leave.
const HANDOVER: Duration = Duration::from_secs(10);
/// With autostart the island can come before Windows is ready to draw it:
/// it starts over a few times before giving up.
const START_ATTEMPTS: u32 = 3;
const START_RETRY: Duration = Duration::from_secs(3);
/// Drawing errors repeat with every frame of a broken state: the journal takes the first ones.
const DRAW_ERRORS_LOGGED: u32 = 20;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Demo {
    Problem(Level),
    /// The next system notice of the list, at each start with the flag.
    System,
    /// The next device notice of the list, the same way.
    Devices,
    /// The update window with a sample release.
    Update,
}

/// `restart` — this island replaces a running one (tray, after a crash, a start retry).
/// `attempt` — which try to start this is. `open` — a file from "Open with".
pub fn run(restart: bool, attempt: u32, demo: Option<Demo>, open: Option<PathBuf>) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if restart {
        if !platform::wait_single_instance(INSTANCE, HANDOVER) {
            journal::error(tr!("journal.restart_old_still_running"));
            return;
        }
    } else if !platform::claim_single_instance(INSTANCE) {
        if let Some(path) = &open {
            if let Ok(hwnd) = unsafe { FindWindowW(window::SERVICE_CLASS, None) } {
                platform::hand_over_path(hwnd, OPEN_FILE, path);
            }
            return;
        }
        let (msg, wparam) = match demo {
            Some(Demo::Problem(level)) => (WM_DEMO, usize::from(level == Level::Critical)),
            Some(Demo::System) => (WM_DEMO, DEMO_SYSTEM),
            Some(Demo::Devices) => (WM_DEMO, DEMO_DEVICES),
            Some(Demo::Update) => (WM_DEMO, DEMO_UPDATE),
            None => (WM_OPEN_SETTINGS, 0),
        };
        unsafe {
            if let Ok(hwnd) = FindWindowW(window::SERVICE_CLASS, None) {
                let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(0));
            }
        }
        return;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    match App::new() {
        Ok(app) => APP.with(|cell| *cell.borrow_mut() = Some(app)),
        Err(e) => {
            journal::error(tr!("journal.start_failed", attempt = attempt, error = e));
            if attempt < START_ATTEMPTS {
                std::thread::sleep(START_RETRY);
                let next = format!("--attempt={}", attempt + 1);
                if platform::relaunch(&["--restart", &next]).is_ok() {
                    return;
                }
            }
            platform::message_box(&tr!(
                "start.failed",
                error = e,
                journal = journal::path().display()
            ));
            return;
        }
    }
    with_app(App::start);
    if let Some(path) = open {
        with_app(|app| app.open_path(path, Arrival::Asked));
    }

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|cell| cell.borrow_mut().take());
}

/// Access to the state from the message handler. If the state is already busy (Windows sent a message
/// in the middle of our own call), the message goes to the default handler.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut app| app.as_mut().map(f))
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        tray::WM_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                show_tray_menu(hwnd);
            }
            return LRESULT(0);
        }
        _ => {}
    }
    match with_app(|app| app.handle(hwnd, msg, wparam, lparam)) {
        Some(Some(result)) => result,
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The screenshot viewer window. The frame and its redraw are decided right here: these messages also
/// arrive in the middle of our calls (window creation, resize), when the state is busy.
unsafe extern "system" fn viewer_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        // Don't redraw the frame when the active window changes: it isn't visible, but it could flicker.
        WM_NCACTIVATE => return unsafe { DefWindowProcW(hwnd, msg, wparam, LPARAM(-1)) },
        _ => {}
    }
    let handled = with_app(|app| {
        if app.is_video_window(hwnd) {
            app.on_video_message(msg, wparam, lparam)
        } else if app.is_opened_window(hwnd) {
            app.on_opened_message(msg, wparam, lparam)
        } else {
            app.on_viewer_message(hwnd, msg, wparam, lparam)
        }
    });
    if let Some(Some(result)) = handled {
        return result;
    }
    let result = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    // A press on the video's picture runs Windows' move loop inside the default handler; it returns
    // when the button is let go, with the state free again.
    if msg == WM_NCLBUTTONDOWN {
        with_app(|app| {
            app.on_video_pressed(hwnd);
            app.on_opened_pressed(hwnd);
        });
    }
    result
}

/// A dialog window: it takes focus on a click, unlike the island, so that Esc reaches it.
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match with_app(|app| app.on_dialog_message(hwnd, msg, wparam, lparam)) {
        Some(Some(result)) => result,
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The area selection window: it takes focus and keys, unlike the island.
unsafe extern "system" fn select_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match with_app(|app| app.on_select_message(hwnd, msg, wparam, lparam)) {
        Some(Some(result)) => result,
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The tray menu is modal: show it without holding the state, and run the command afterwards.
fn show_tray_menu(hwnd: HWND) {
    let Some((paused, show, hidden)) = with_app(|app| {
        let hidden = match app.hidden {
            Some(h) => HiddenState::Hidden(h.minutes_left()),
            None => HiddenState::Shown,
        };
        (app.scheduler.is_paused(), app.settings.show_island, hidden)
    }) else {
        return;
    };
    if let Some(command) = tray::show_menu(hwnd, paused, show, hidden) {
        with_app(|app| app.command(command));
    }
}

fn log(result: windows::core::Result<()>) {
    static LOGGED: AtomicU32 = AtomicU32::new(0);
    if let Err(e) = result
        && LOGGED.fetch_add(1, Ordering::Relaxed) < DRAW_ERRORS_LOGGED
    {
        journal::warn(tr!("journal.render_failed", error = e));
    }
}

struct App {
    island: HWND,
    service: HWND,
    scene: Scene,
    tray: Tray,
    taskbar_created: u32,

    started: Instant,
    last_tick: SystemTime,
    ticks: u32,

    settings: Settings,
    settings_path: PathBuf,
    settings_mtime: Option<SystemTime>,

    scheduler: Scheduler,
    stage: Stage,
    controls: ControlCenter,
    /// The "Keep awake also holds the screen" setting: shared with the sleep-block port.
    keep_screen: Rc<Cell<bool>>,

    reminders: Reminders,
    player: Player,
    shots: Shots,
    watch: Watch,
    recording: Recording,
    record_panel: RecordPanel,
    select: Select,
    hotkeys: Hotkeys,
    problems: Problems,
    updates: Updates,
    dialogs: Dialogs,
    notify: Notify,
    downloads: Downloads,
    devices: Devices,
    opened: Opened,
    explorer: Explorer,
    pointer: Pointer,

    monitor: MonitorId,
    scale: f32,
    frame: (f32, f32),
    /// Current and target click area, logical pixels.
    shape: (f32, f32),
    target_shape: (f32, f32),

    visible: bool,
    fullscreen: bool,
    hidden: Option<Hidden>,
    /// Opened by the pointer: the hover view of whatever the island shows.
    expanded: bool,
    dimmed: bool,
    /// The island is at rest: collapsed and the mouse isn't over it (the dim countdown runs).
    resting: bool,
    /// The timers could not be saved last time: said once per run of failures.
    save_failing: bool,
}

impl App {
    fn new() -> windows::core::Result<Self> {
        let settings_path = persist::settings_path();
        let first_run = !settings_path.exists();
        let (settings, recovery) = Settings::recover(&settings_path);
        // Before the first text is built, window titles included.
        let language = lang::select(settings.language);

        let (island, service) = window::create(wndproc, viewer_proc, select_proc, dialog_proc)?;
        let scene = Scene::new(comp::Comp::new(island)?)?;

        let mut problems = Problems::new();
        // The installer's language is only a hint at the first start: from now on the file has it.
        if first_run
            && settings.language != Language::Auto
            && let Err(e) = settings.save(&settings_path)
        {
            problems.early(Fault::new(What::SettingsSave, e.into()));
        }
        if let Some(failed) = language {
            problems.early(Fault::new(What::Language, failed));
        }
        if let Some(text) = crash::take() {
            let failed = Failed::because(Cause::Unknown, text);
            problems.early(Fault::new(What::Crashed, failed));
        }
        if let Some(fault) = config::recovery_fault(recovery) {
            problems.early(fault);
        }
        let mut scheduler = Scheduler::new(&settings.reminders, settings.idle_min);
        let mut hidden = None;
        let mut quiet = false;
        let state_path = persist::state_path();
        if let Some(saved) = SavedState::load(&state_path) {
            scheduler.restore(&saved.timers, saved.paused, saved.offline_ms());
            hidden = saved.hidden.filter(|h| !h.expired());
            quiet = saved.quiet;
        } else if state_path.exists() {
            journal::warn(tr!("journal.timers_unreadable"));
        }
        let monitor = match settings.island_monitor.as_str() {
            "" => platform::cursor_monitor(),
            name => platform::monitor_named(name).unwrap_or_else(platform::primary_monitor),
        };
        let keep_screen = Rc::new(Cell::new(settings.keep_screen_on));
        let record_since = Rc::new(Cell::new(None));
        let controls = ControlCenter::standard(controls::ports(
            service,
            quiet,
            keep_screen.clone(),
            record_since.clone(),
        ));
        let mut stage = Stage::new();
        stage.set_quiet(quiet, 0);
        // The exe path may change after a reinstall: refresh the autostart entry.
        platform::set_autostart(settings.autostart);

        let tray = Tray::new(service);
        tray.add();
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };

        Ok(Self {
            island,
            service,
            scene,
            tray,
            taskbar_created,
            started: Instant::now(),
            last_tick: SystemTime::now(),
            ticks: 0,
            settings_mtime: persist::modified(&settings_path),
            settings,
            settings_path,
            scheduler,
            stage,
            controls,
            keep_screen,
            reminders: Reminders::new(),
            player: Player::new(),
            shots: Shots::default(),
            watch: Watch::default(),
            recording: Recording::new(record_since),
            record_panel: RecordPanel::default(),
            select: Select::default(),
            hotkeys: Hotkeys::default(),
            problems,
            updates: Updates::new(),
            dialogs: Dialogs::default(),
            notify: Notify::default(),
            downloads: Downloads::default(),
            devices: Devices::default(),
            opened: Opened::default(),
            explorer: Explorer::default(),
            pointer: Pointer::new(),
            monitor,
            scale: 1.0,
            frame: (layout::FRAME_WIDTH, 100.0),
            shape: (0.0, 0.0),
            target_shape: (0.0, 0.0),
            visible: false,
            fullscreen: platform::is_busy(),
            hidden,
            expanded: false,
            dimmed: false,
            resting: false,
            save_failing: false,
        })
    }

    fn start(&mut self) {
        self.set_timer(timers::TICK, TICK_MS);
        self.relayout();
        self.sync_visibility();
        self.sync_media();
        #[cfg(debug_assertions)]
        levels_debug::open();
        self.sync_shot_key();
        self.sync_record_key();
        self.sync_area_key();
        self.sweep_recordings();
        self.start_notify();
        self.sync_downloads();
        self.sync_devices();
        self.register_viewer();
        self.sync_explorer();
        self.problems.booting = false;
        journal::info(tr!(
            "journal.started",
            version = env!("CARGO_PKG_VERSION"),
            windows = platform::windows_build(),
            scale = self.scale
        ));
    }

    fn shut_down(&mut self) {
        self.close_watch();
        self.close_opened();
        self.stop_explorer();
        self.save();
        self.flush_shots();
        self.finish_recording();
        self.stop_notify();
        self.stop_downloads();
        self.stop_devices();
        self.close_settings();
        self.drop_dialogs();
        journal::info(tr!("journal.stopped"));
    }

    fn quit(&mut self) {
        self.shut_down();
        self.tray.remove();
        unsafe { PostQuitMessage(0) };
    }

    fn handle(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let island = hwnd == self.island;
        match msg {
            WM_TIMER => self.on_timer(wparam.0),
            WM_MOUSEMOVE if island => self.on_mouse_move(point(lparam)),
            WM_MOUSELEAVE if island => self.on_mouse_leave(),
            WM_LBUTTONDOWN if island => self.on_mouse_down(point(lparam)),
            WM_LBUTTONDOWN
                if self
                    .shots
                    .flight
                    .as_ref()
                    .is_some_and(|f| f.is_catcher(hwnd)) =>
            {
                self.on_catch()
            }
            WM_LBUTTONUP if island => self.on_mouse_up(point(lparam)),
            WM_MOUSEWHEEL if island => self.on_wheel(wheel_delta(wparam), false),
            WM_MOUSEHWHEEL if island => self.on_wheel(wheel_delta(wparam), true),
            WM_CAPTURECHANGED if island => self.on_capture_lost(),
            WM_HOTKEY if island => self.on_hotkey(wparam.0 as i32),
            WM_MEDIA => self.on_media(),
            WM_CONTROLS => self.on_controls(),
            WM_RECORD => self.on_record_events(),
            WM_SAVE => self.on_video_save(),
            WM_UPDATE => self.on_update_message(),
            WM_DOWNLOAD => self.on_downloads(),
            WM_DEVICES => self.on_devices(),
            WM_OPENED => self.on_opened(),
            WM_EXPLORER => self.on_explorer(wparam),
            // The default handler frees the copy of the key after us.
            WM_INPUT => {
                self.on_explorer_key(lparam);
                return None;
            }
            WM_COPYDATA => {
                if let Some(path) = platform::handed_path(lparam, OPEN_FILE) {
                    self.open_path(path, Arrival::Asked);
                }
                return Some(LRESULT(1));
            }
            crate::update::COMMAND_MESSAGE => self.on_update_command(wparam.0),
            WM_FAULT => self.on_faults(),
            WM_POWERBROADCAST => {
                self.on_power();
                return Some(LRESULT(1));
            }
            WM_DEMO if wparam.0 == DEMO_SYSTEM => self.demo_system(),
            WM_DEMO if wparam.0 == DEMO_DEVICES => self.demo_devices(),
            WM_DEMO if wparam.0 == DEMO_UPDATE => self.demo_update(),
            WM_DEMO => self.demo_problem(wparam.0 != 0),
            WM_OPEN_SETTINGS => self.open_settings(),
            WM_DISPLAYCHANGE => {
                self.relayout();
                self.follow_monitor();
            }
            // PrtSc could have been handed to the island or given back to Snipping Tool.
            WM_SETTINGCHANGE if island => self.sync_shot_key(),
            // The island's position and scale are set by us.
            WM_DPICHANGED => {}
            WM_QUERYENDSESSION => {
                self.shut_down();
                return Some(LRESULT(1));
            }
            WM_ENDSESSION => self.shut_down(),
            m if m == self.taskbar_created && m != 0 => self.tray.add(),
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            timers::TICK => self.on_tick(),
            timers::STAGE => self.on_stage_timer(),
            timers::HOVER => self.on_hover_timer(),
            timers::GESTURE => self.on_gesture_timer(),
            timers::HOLD => self.on_hold_timer(),
            timers::SKIP => self.on_skip_step(),
            timers::BARS => self.on_bars_timer(),
            timers::MENU => self.on_menu_timer(),
            timers::SHRINK => self.on_shrink_timer(),
            timers::PURGE => self.on_purge_timer(),
            timers::HIDE => self.on_hide_timer(),
            timers::DIM => self.on_dim_timer(),
            timers::SHUTTER => self.on_shutter(),
            timers::FLIGHT => self.on_flight_end(),
            timers::CATCH => self.on_catch_timer(),
            timers::LANDING => self.on_landing_step(),
            timers::SHOT_CLOSE => self.on_shot_close(),
            timers::VIEWER_SHOW => self.on_viewer_show(),
            timers::VIEWER_HIDE => self.on_viewer_hide(),
            timers::WATCH_SHOW => self.on_watch_show(),
            timers::WATCH_HIDE => self.on_watch_hide(),
            timers::UPDATE => self.on_update_timer(),
            timers::COUNTDOWN => self.on_countdown_timer(),
            timers::CLIP_CLOSE => self.on_clip_close(),
            timers::REPLAY => self.on_replay(),
            timers::COPIED => self.on_copied_timer(),
            timers::BUTTON_HOLD => self.on_button_hold_timer(),
            timers::SELECT => self.on_select_timer(),
            timers::MARKS_GONE => self.on_marks_gone(),
            timers::METER => self.on_meter_timer(),
            timers::NOTIFY => self.on_notify_sample(),
            timers::SAVER_WAIT => self.on_saver_wait(),
            timers::CLOCK => self.on_clock_timer(),
            timers::DEVICE_WAIT => self.on_device_wait(),
            timers::OUTSIDE => self.on_outside_timer(),
            timers::DIALOG_GONE => self.on_dialog_gone(),
            timers::FOLLOW => self.on_follow_timer(),
            _ => {}
        }
    }

    fn on_tick(&mut self) {
        let now = SystemTime::now();
        // If the clock was set back, the tick is just skipped.
        let dt = now
            .duration_since(self.last_tick)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_tick = now;

        self.check_settings();
        self.follow_monitor();
        self.tick_updates();
        self.sync_record_time();
        self.tick_visibility();
        self.tick_reminders(dt);

        let now = self.now_ms();
        self.check_silence(now);
        self.advance_stage(now);

        if self.visible {
            self.render();
        }

        self.ticks += 1;
        if self.ticks.is_multiple_of(SAVE_EVERY_TICKS) {
            self.save();
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::TogglePause => self.toggle_reminders(),
            Command::ToggleIsland => self.toggle_island(),
            Command::Hide(minutes) => self.set_hidden(Some(Hidden::for_minutes(minutes))),
            Command::Unhide => self.set_hidden(None),
            Command::Settings => self.open_settings(),
            // The new island waits for this one to leave; started first, so that an exe
            // that can't be started does not leave the user without an island.
            Command::Restart => match platform::relaunch(&["--restart"]) {
                Ok(()) => self.quit(),
                Err(e) => self.report(Fault::new(What::Restart, e.into())),
            },
            Command::Quit => self.quit(),
        }
    }

    fn save(&mut self) {
        let mut state = SavedState::new(
            self.scheduler.is_paused(),
            self.scheduler.export(),
            self.hidden,
        );
        state.quiet = self.controls.is_on(ControlId::DoNotDisturb);
        match state.save(&persist::state_path()) {
            Ok(()) => self.save_failing = false,
            // Saved every 15 s: a failure that lasts is said once.
            Err(e) if !self.save_failing => {
                self.save_failing = true;
                self.report(Fault::new(What::TimersSave, e.into()));
            }
            Err(_) => {}
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Moves the notice queue and the player to `now`: a notice runs out, a paused player leaves.
    fn advance_stage(&mut self, now: u64) {
        let events = self.player.tracker.tick(now);
        let mut changed = self.apply_player(&events);
        changed |= self.stage.tick(now);
        self.after_stage(changed);
    }

    /// After anything that could change what the island shows: redraw, and let each feature
    /// see whether its notice is gone.
    fn after_stage(&mut self, changed: bool) {
        if changed {
            self.sync_visibility();
            self.render();
            self.sync_rings();
        }
        self.schedule_stage();
        self.sync_shot();
        self.sync_clip();
        self.sync_problem();
        self.sync_record_panel();
        self.sync_notify();
        self.sync_downloads_stage();
        self.sync_devices_stage();
        self.sync_clock();
    }

    fn schedule_stage(&self) {
        let now = self.now_ms();
        let wake = [self.stage.next_wake(), self.player.tracker.next_wake()]
            .into_iter()
            .flatten()
            .min();
        match wake {
            Some(at) => self.set_timer(timers::STAGE, at.saturating_sub(now).max(1) as u32),
            None => self.kill_timer(timers::STAGE),
        }
    }

    fn on_stage_timer(&mut self) {
        self.kill_timer(timers::STAGE);
        self.advance_stage(self.now_ms());
    }

    fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.island), id, ms, None);
        }
    }

    fn kill_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.island), id);
        }
    }
}
