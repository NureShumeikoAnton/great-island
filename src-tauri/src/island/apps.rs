//! The apps on the island (the layer is `crate::apps`).
//!
//! 1. The list follows the settings file: an app switched on is made and told `Started`, one
//!    switched off is dropped with its timers (its page closes); the rest keep their state and
//!    hear `Values` when their settings changed. A connector turned on after the start says
//!    "Service connected" (`app_news.rs`).
//! 2. The island pulled up (with the mouse or two fingers) opens the row of apps that are on,
//!    in the catalog's order; it closes by a pull down or, like the control center, when the
//!    cursor leaves it. With no app on, the pull up leads nowhere.
//! 3. A tile opens the app's page in the row's place. An app that wants the keyboard gets it:
//!    the island takes the focus, and gives it back to the window that had it when the page
//!    closes. A press on the island while the page is open takes it again.
//! 4. The page stays while the cursor is away. The ‹ goes back to the row; Esc closes the row
//!    too. A closed page ends what the user did in an app that lives one opening (the
//!    calculator): the island makes it anew (`Manifest::lifetime`).
//! 5. Every effect an app asks for is checked against its manifest first (`apps::host`); a
//!    refusal is the app's bug: a journal line. Timers share one Windows timer
//!    (`host::Timers`); a connector's request runs on a thread of its own and its answer comes
//!    back as `WM_APPS`. A service that stops answering is reported once, until it answers.
//! 6. After every event the island asks the app what it shows outside its page: a notice, a
//!    page of the ribbon, a badge (`app_news.rs`).

use super::{App, timers, view::AppTile, window::WM_APPS};
use crate::{
    apps::{
        Applet, Context, Effect, Event, Key, Kind, Live, catalog,
        host::{self, Refusal, Timers},
        net::{Request, Response},
        prefs::Values,
    },
    capture::adapters::SystemClipboard,
    fault::{Failed, Fault, What},
    journal,
    lang::tr,
    persist, platform,
    stage::Sheet,
    update::adapters::http,
};
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            GetKeyState, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_RETURN,
        },
        WindowsAndMessaging::{GetForegroundWindow, IsWindow, PostMessageW, SetForegroundWindow},
    },
};

/// A feed of a whole country is a few dozen KB; anything far bigger is not an answer.
const ANSWER_LIMIT: usize = 4 * 1024 * 1024;

pub(super) struct Apps {
    /// The apps that are on, in the row's order.
    pub(super) list: Vec<Slot>,
    open: Option<usize>,
    /// The window that had the keyboard before a page took it, as a number.
    focus_back: Option<isize>,
    /// The page was opened from the ribbon: the ‹ goes back there, not to the row.
    pub(super) from_ribbon: bool,
    timers: Timers,
    answers: (Sender<Answer>, Receiver<Answer>),
}

pub(super) struct Slot {
    pub(super) app: Box<dyn Applet>,
    pub(super) id: &'static str,
    /// The settings as the app last heard them: a change is an `Event::Values`.
    values: Values,
    pub(super) live: Option<Live>,
    /// Its service failed and that was said: said again only after an answer.
    failing: bool,
}

struct Answer {
    app: &'static str,
    tag: u16,
    result: Result<Response, Failed>,
}

impl Default for Apps {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            open: None,
            focus_back: None,
            from_ribbon: false,
            timers: Timers::default(),
            answers: channel(),
        }
    }
}

/// What the scene needs from the apps, copied: see `render::ViewInput`.
pub(super) struct AppsInput {
    pub(super) tiles: Vec<AppTile>,
    /// The open page: the app's id, its title and its tree.
    pub(super) page: Option<(&'static str, String, crate::apps::ui::Node)>,
}

fn short(manifest: &crate::apps::Manifest) -> String {
    if manifest.short.is_empty() {
        manifest.title.clone()
    } else {
        manifest.short.clone()
    }
}

impl App {
    /// At start (`booting`) and after the settings file changed.
    pub(super) fn sync_apps(&mut self) {
        let wanted = catalog::on(&self.settings.apps);
        let wanted_ids: Vec<&'static str> = wanted.iter().map(|a| a.manifest().id).collect();
        let open_id = self.apps.open.map(|i| self.apps.list[i].id);
        if open_id.is_some_and(|id| !wanted_ids.contains(&id)) {
            self.close_app();
        }
        for gone in self
            .apps
            .list
            .iter()
            .filter(|s| !wanted_ids.contains(&s.id))
        {
            self.apps.timers.drop_app(gone.id);
        }
        let mut old = std::mem::take(&mut self.apps.list);
        let mut started = Vec::new();
        for app in wanted {
            let manifest = app.manifest();
            let values = Values::resolve(&manifest.fields, self.settings.apps.get(manifest.id));
            match old.iter().position(|s| s.id == manifest.id) {
                Some(at) => {
                    let mut slot = old.remove(at);
                    let changed = slot.values != values;
                    slot.values = values;
                    self.apps.list.push(slot);
                    if changed {
                        started.push((self.apps.list.len() - 1, Event::Values));
                    }
                }
                None => {
                    self.apps.list.push(Slot {
                        app,
                        id: manifest.id,
                        values,
                        live: None,
                        failing: false,
                    });
                    started.push((self.apps.list.len() - 1, Event::Started));
                    if !self.problems.booting && matches!(manifest.kind, Kind::Connector(_)) {
                        self.show_connected(&manifest);
                    }
                }
            }
        }
        self.apps.open = open_id.and_then(|id| self.apps.list.iter().position(|s| s.id == id));
        if self.apps.list.is_empty() && self.stage.shown().sheet == Some(Sheet::Apps) {
            self.close_menu();
        }
        for (i, event) in started {
            self.send_to(i, event);
        }
        self.sync_live();
    }

    pub(super) fn has_apps(&self) -> bool {
        !self.apps.list.is_empty()
    }

    pub(super) fn app_open(&self) -> bool {
        self.apps.open.is_some()
    }

    fn context<'a>(&self, values: &'a Values) -> Context<'a> {
        let unix_ms = persist::now_ms();
        Context {
            now: self.now_ms(),
            unix_ms,
            utc_offset_ms: platform::utc_offset_ms(unix_ms),
            values,
        }
    }

    /// The tallest page of the apps that are on: the island's window must hold it.
    pub(super) fn tallest_app(&self) -> f32 {
        self.apps
            .list
            .iter()
            .map(|s| {
                let cx = self.context(&s.values);
                self.scene.page_height(&s.app.page(&cx))
            })
            .fold(0.0, f32::max)
    }

    pub(super) fn apps_input(&self) -> AppsInput {
        let tiles = self
            .apps
            .list
            .iter()
            .map(|s| {
                let manifest = s.app.manifest();
                AppTile {
                    title: short(&manifest),
                    icon: manifest.icon,
                }
            })
            .collect();
        let page = self.apps.open.map(|i| {
            let slot = &self.apps.list[i];
            let manifest = slot.app.manifest();
            let cx = self.context(&slot.values);
            (slot.id, short(&manifest), slot.app.page(&cx))
        });
        AppsInput { tiles, page }
    }

    pub(super) fn launch_app(&mut self, index: usize) {
        if index >= self.apps.list.len() {
            return;
        }
        self.apps.open = Some(index);
        if self.apps.list[index].app.manifest().keyboard {
            self.take_keyboard();
        }
        self.send_to(index, Event::Opened);
        self.after_apps();
    }

    /// The ‹: back to the row, or to the ribbon the page was opened from.
    pub(super) fn close_app(&mut self) {
        let Some(i) = self.apps.open else {
            return;
        };
        let to_ribbon = self.apps.from_ribbon;
        self.send_to(i, Event::Closed);
        self.end_page();
        self.after_apps();
        if to_ribbon {
            self.close_menu();
        } else if !self.pointer.inside {
            // Back in the row with the cursor away: the row goes the way the control center does.
            self.close_menu_soon();
        }
    }

    pub(super) fn app_press(&mut self, action: crate::apps::ui::Action) {
        if let Some(i) = self.apps.open {
            self.send_to(i, Event::Press(action));
        }
        self.after_apps();
    }

    /// The row is gone (a pull down, the cursor away, a capture that closed every sheet): so is
    /// the page.
    pub(super) fn sync_apps_sheet(&mut self) {
        if let Some(i) = self.apps.open
            && self.stage.shown().sheet != Some(Sheet::Apps)
        {
            self.send_to(i, Event::Closed);
            self.end_page();
        }
    }

    /// The page is gone, however it went: the keyboard goes back, and an app that lives one
    /// opening is made anew.
    fn end_page(&mut self) {
        let Some(i) = self.apps.open.take() else {
            return;
        };
        self.apps.from_ribbon = false;
        self.give_keyboard_back();
        catalog::after_page(&mut self.apps.list[i].app);
    }

    /// A press on the island while a page that types is open: the keyboard comes back to it.
    pub(super) fn reclaim_keyboard(&mut self) {
        if self.page_types() && unsafe { GetForegroundWindow() } != self.island {
            unsafe {
                let _ = SetForegroundWindow(self.island);
            }
        }
    }

    /// WM_KEYDOWN on the island: the keys that type no character.
    pub(super) fn on_app_key(&mut self, wparam: WPARAM) {
        let (true, Some(i)) = (self.page_types(), self.apps.open) else {
            return;
        };
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        let key = match wparam.0 as u16 {
            k if k == VK_ESCAPE.0 => {
                self.close_app();
                self.close_menu();
                return;
            }
            k if k == VK_RETURN.0 => Key::Enter,
            k if k == VK_BACK.0 => Key::Backspace,
            k if k == VK_DELETE.0 => Key::Delete,
            k if k == u16::from(b'C') && ctrl => Key::Copy,
            _ => return,
        };
        self.send_to(i, Event::Key(key));
        self.after_apps();
    }

    /// WM_CHAR on the island: a typed character, after the layout. Control characters (Enter,
    /// Backspace, Ctrl+C) already came as keys.
    pub(super) fn on_app_char(&mut self, wparam: WPARAM) {
        let (true, Some(i)) = (self.page_types(), self.apps.open) else {
            return;
        };
        let Some(c) = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control()) else {
            return;
        };
        self.send_to(i, Event::Key(Key::Char(c)));
        self.after_apps();
    }

    fn page_types(&self) -> bool {
        self.apps
            .open
            .is_some_and(|i| self.apps.list[i].app.manifest().keyboard)
    }

    fn take_keyboard(&mut self) {
        let before = unsafe { GetForegroundWindow() };
        if before != self.island {
            self.apps.focus_back = Some(before.0 as isize);
        }
        unsafe {
            let _ = SetForegroundWindow(self.island);
        }
    }

    /// Only if the island still has it: a window the user went to keeps it.
    fn give_keyboard_back(&mut self) {
        let Some(back) = self.apps.focus_back.take() else {
            return;
        };
        let back = HWND(back as *mut _);
        unsafe {
            if GetForegroundWindow() == self.island && IsWindow(Some(back)).as_bool() {
                let _ = SetForegroundWindow(back);
            }
        }
    }

    pub(super) fn on_apps_timer(&mut self) {
        self.kill_timer(timers::APPS);
        let open = self.apps.open.map(|i| self.apps.list[i].id);
        let due = self.apps.timers.due(self.now_ms(), open);
        for (id, timer) in due {
            if let Some(i) = self.apps.list.iter().position(|s| s.id == id) {
                self.send_to(i, Event::Timer(timer));
            }
        }
        self.arm_apps_timer();
        self.after_apps();
    }

    fn arm_apps_timer(&self) {
        match self.apps.timers.next() {
            Some(at) => {
                let ms = at
                    .saturating_sub(self.now_ms())
                    .clamp(1, u64::from(u32::MAX));
                self.set_timer(timers::APPS, ms as u32);
            }
            None => self.kill_timer(timers::APPS),
        }
    }

    /// `WM_APPS`: answers from the network. One for an app that was turned off meanwhile is
    /// dropped.
    pub(super) fn on_apps_answers(&mut self) {
        let answers: Vec<Answer> = self.apps.answers.1.try_iter().collect();
        for answer in answers {
            let Some(i) = self.apps.list.iter().position(|s| s.id == answer.app) else {
                continue;
            };
            match &answer.result {
                Err(failed) if !self.apps.list[i].failing => {
                    self.apps.list[i].failing = true;
                    self.report(Fault::new(What::Connector, failed.clone()));
                }
                Err(_) => {}
                Ok(_) => self.apps.list[i].failing = false,
            }
            self.send_to(
                i,
                Event::Fetched {
                    tag: answer.tag,
                    result: answer.result,
                },
            );
        }
        self.after_apps();
    }

    fn fetch(&self, app: &'static str, tag: u16, request: Request) {
        let sender = self.apps.answers.0.clone();
        // A HWND can't cross threads; a number can.
        let island = self.island.0 as isize;
        std::thread::spawn(move || {
            let result =
                http::get(&request.url, ANSWER_LIMIT).map(|body| Response { status: 200, body });
            if sender.send(Answer { app, tag, result }).is_ok() {
                unsafe {
                    let _ =
                        PostMessageW(Some(HWND(island as *mut _)), WM_APPS, WPARAM(0), LPARAM(0));
                }
            }
        });
    }

    /// One event to one app, and what it asks for carried out. The caller redraws
    /// (`after_apps`).
    fn send_to(&mut self, i: usize, event: Event) {
        let now = self.now_ms();
        let unix_ms = persist::now_ms();
        let slot = &mut self.apps.list[i];
        let manifest = slot.app.manifest();
        let cx = Context {
            now,
            unix_ms,
            utc_offset_ms: platform::utc_offset_ms(unix_ms),
            values: &slot.values,
        };
        let effects = slot.app.update(event, &cx);
        let live = slot.app.live(&cx);
        slot.live = live;
        for effect in effects {
            if let Err(refusal) = host::admit(&manifest, &effect) {
                let reason = match refusal {
                    Refusal::NotAConnector => "not a connector".to_string(),
                    Refusal::Undeclared(host) => format!("undeclared host {host}"),
                };
                journal::warn(tr!(
                    "journal.app_refused",
                    id = manifest.id,
                    effect = effect_name(&effect),
                    reason = reason
                ));
                continue;
            }
            match effect {
                Effect::Copy(text) => {
                    if let Err(failed) = SystemClipboard::new(self.island).put_text(&text) {
                        self.report(Fault::new(What::Copy, failed));
                    }
                }
                Effect::Notify(notice) => self.show_app_notice(notice),
                Effect::After { timer, ms } | Effect::Every { timer, ms } => {
                    let repeat = matches!(effect, Effect::Every { .. });
                    let now = self.now_ms();
                    self.apps.timers.set(manifest.id, timer, ms, repeat, now);
                    self.arm_apps_timer();
                }
                Effect::Stop { timer } => {
                    self.apps.timers.stop(manifest.id, timer);
                    self.arm_apps_timer();
                }
                Effect::Fetch { tag, request } => self.fetch(manifest.id, tag, request),
                Effect::Close if self.apps.open == Some(i) => self.end_page(),
                Effect::Close => {}
                // Built with its first app: a saved state (TASK_12, stage 4).
                Effect::Save => journal::warn(tr!(
                    "journal.app_later",
                    id = manifest.id,
                    effect = effect_name(&effect)
                )),
            }
        }
    }

    /// After the apps changed: what they show outside their pages, then the picture.
    fn after_apps(&mut self) {
        self.sync_live();
        self.render();
    }
}

fn effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::Copy(_) => "Copy",
        Effect::Notify(_) => "Notify",
        Effect::After { .. } => "After",
        Effect::Every { .. } => "Every",
        Effect::Stop { .. } => "Stop",
        Effect::Fetch { .. } => "Fetch",
        Effect::Save => "Save",
        Effect::Close => "Close",
    }
}
