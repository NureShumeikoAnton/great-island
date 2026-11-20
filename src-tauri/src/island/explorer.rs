//! Space in Explorer, as in QuickLook (the rules are `files::follow`).
//!
//! 1. While the setting is on, Windows tells the island whenever another window comes to the
//!    front. With an Explorer window or the desktop in front, the island gets a copy of the
//!    keys (`hotkeys.rs`); with anything else in front, none.
//! 2. Space alone with the focus in the list of files: the selection is read on a thread of its
//!    own, and a photo or a video opens (`open.rs`) without taking the focus. Explorer gets its
//!    Space too.
//! 3. While that window is open and Explorer is in front, the window stays above it and follows
//!    its selection: after a key, and when Explorer says its focus or selection changed, the
//!    selection is read again, a moment after the last such news. The editor holds the window.
//! 4. Esc with the focus in the list of files closes the window. Another program in front:
//!    the window goes behind it like any window, and follows again once Explorer is in front.

use super::{App, open::Arrival, timers, window::WM_EXPLORER};
use crate::files::{
    adapters::explorer::{
        Ask, Front, Hooks, News, Reader, in_file_list, modifiers_held, process_of, read_key,
    },
    follow::Follow,
};
use std::path::PathBuf;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{VK_ESCAPE, VK_SPACE},
        WindowsAndMessaging::{GetForegroundWindow, PostMessageW},
    },
};

/// News of the selection comes in bursts (a key, a click, a drag over files): it is read once
/// they stop for this long.
const SETTLE_MS: u32 = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Space,
    Follow,
}

#[derive(Default)]
pub(super) struct Explorer {
    follow: Follow,
    hooks: Option<Hooks>,
    reader: Option<Reader>,
    token: u64,
    asked: Option<(u64, Purpose)>,
    /// The Explorer window or the desktop in front; `None` = something else is.
    pub(super) front: Option<Front>,
}

impl App {
    /// By the setting: whether the island hears about the window in front at all.
    pub(super) fn sync_explorer(&mut self) {
        let on = self.settings.explorer_space;
        let service = self.service;
        let hooks = self
            .explorer
            .hooks
            .get_or_insert_with(|| Hooks::new(service, WM_EXPLORER));
        hooks.watch_foreground(on);
        if !on {
            hooks.watch_selection(None);
            self.explorer.front = None;
        } else {
            self.explorer.front = Front::of(unsafe { GetForegroundWindow() });
        }
        self.sync_explorer_keys();
    }

    pub(super) fn stop_explorer(&mut self) {
        self.explorer.hooks = None;
        self.explorer.front = None;
        self.sync_explorer_keys();
    }

    pub(super) fn follows_explorer(&self) -> bool {
        self.explorer.follow.following()
    }

    pub(super) fn on_explorer(&mut self, wparam: WPARAM) {
        match News::of(wparam) {
            News::Foreground => self.on_foreground(),
            News::Selection => {
                if let Some(hooks) = &self.explorer.hooks {
                    hooks.selection_seen();
                }
                self.settle_selection();
            }
            News::Answer => self.on_selection_read(),
        }
    }

    fn on_foreground(&mut self) {
        let foreground = unsafe { GetForegroundWindow() };
        let front = Front::of(foreground);
        self.explorer.front = front;
        self.sync_explorer_keys();
        let Some(window) = self.opened_window().filter(|_| self.follows_explorer()) else {
            return;
        };
        match front {
            Some(front) => {
                super::window::set_floating(window, true, None);
                self.watch_explorer_selection(Some(front));
                self.settle_selection();
            }
            // The window itself: it is in front anyway.
            None if foreground == window => {}
            None => {
                super::window::set_floating(window, false, Some(foreground));
                self.watch_explorer_selection(None);
            }
        }
    }

    fn watch_explorer_selection(&mut self, front: Option<Front>) {
        if let Some(hooks) = &mut self.explorer.hooks {
            hooks.watch_selection(front.map(|f| process_of(f.hwnd())));
        }
    }

    /// `WM_INPUT`: a copy of a key pressed while Explorer is in front.
    pub(super) fn on_explorer_key(&mut self, lparam: LPARAM) {
        let (Some(key), Some(front)) = (read_key(lparam), self.explorer.front) else {
            return;
        };
        // The copy may arrive after another window came forward.
        if unsafe { GetForegroundWindow() }.0 as isize != front.hwnd() {
            return;
        }
        if key.down && key.vk == VK_SPACE.0 && !modifiers_held() && in_file_list(front) {
            self.read_selection(Purpose::Space);
        } else if key.down
            && key.vk == VK_ESCAPE.0
            && self.follows_explorer()
            && in_file_list(front)
        {
            self.close_opened();
        } else if !key.down {
            self.settle_selection();
        }
    }

    fn settle_selection(&self) {
        if self.follows_explorer() && self.explorer.front.is_some() {
            self.set_timer(timers::FOLLOW, SETTLE_MS);
        }
    }

    pub(super) fn on_follow_timer(&mut self) {
        self.kill_timer(timers::FOLLOW);
        if self.follows_explorer() {
            self.read_selection(Purpose::Follow);
        }
    }

    fn read_selection(&mut self, purpose: Purpose) {
        let Some(front) = self.explorer.front else {
            return;
        };
        if self.explorer.reader.is_none() {
            let service = self.service.0 as isize;
            let notify = move || unsafe {
                let _ = PostMessageW(
                    Some(HWND(service as *mut _)),
                    WM_EXPLORER,
                    News::Answer.wparam(),
                    LPARAM(0),
                );
            };
            match Reader::start(notify) {
                Ok(reader) => self.explorer.reader = Some(reader),
                Err(failed) => {
                    crate::journal::warn(crate::lang::tr!(
                        "journal.explorer_failed",
                        error = failed.detail
                    ));
                    return;
                }
            }
        }
        let explorer = &mut self.explorer;
        explorer.token += 1;
        explorer.asked = Some((explorer.token, purpose));
        if let Some(reader) = &explorer.reader {
            reader.ask(Ask {
                token: explorer.token,
                front,
            });
        }
    }

    fn on_selection_read(&mut self) {
        let Some(answer) = self.explorer.reader.as_ref().and_then(Reader::latest) else {
            return;
        };
        let Some((token, purpose)) = self.explorer.asked else {
            return;
        };
        if answer.token != token {
            return;
        }
        self.explorer.asked = None;
        let editing = self.opened_editing();
        let follow = &mut self.explorer.follow;
        let show = match (purpose, &answer.item) {
            (Purpose::Space, Some(item)) => follow.space(item),
            (Purpose::Follow, item) => {
                follow.hold(editing);
                follow.selected(item.as_ref())
            }
            (Purpose::Space, None) => None,
        };
        if let Some(show) = show {
            self.show_file(show, Arrival::Explorer);
        }
    }

    /// `open.rs` put a window on screen (`path` = opened from Explorer, which it follows now)
    /// or closed it.
    pub(super) fn after_opened(&mut self, window: HWND, path: Option<PathBuf>) {
        let from_explorer = path.is_some();
        self.explorer.follow.showing(path);
        if from_explorer && let Some(front) = self.explorer.front {
            super::window::set_floating(window, true, None);
            self.watch_explorer_selection(Some(front));
        } else {
            self.watch_explorer_selection(None);
        }
    }
}
