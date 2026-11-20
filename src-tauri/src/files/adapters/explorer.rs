//! Explorer as Space and following need it: which window is in front (an Explorer window or
//! the desktop), whether the focus is in its list of files, what is selected there, and
//! Windows' news: the foreground changed, a key went by, the selection may have moved.
//!
//! The keys come as raw input (`RIDEV_INPUTSINK`): a copy of each key, taken only while
//! Explorer is in front. Nothing is held back from Explorer and no hook sits in the input
//! chain, so a busy island can't slow anyone's typing.

use crate::{fault::Failed, files::follow::Item};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver, Sender},
    },
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, IServiceProvider,
            },
            SystemServices::{SFGAO_FOLDER, SFGAO_STREAM},
            Variant::VARIANT,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            Input::{
                GetRawInputData, HRAWINPUT,
                KeyboardAndMouse::{
                    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
                },
                RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE,
                RIM_TYPEKEYBOARD, RegisterRawInputDevices,
            },
            Shell::{
                CSIDL_DESKTOP, IFolderView2, IShellBrowser, IShellItem, IShellItemArray,
                IShellWindows, IWebBrowserApp, SID_STopLevelBrowser, SIGDN_FILESYSPATH,
                SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
            },
            WindowsAndMessaging::{
                EVENT_OBJECT_FOCUS, EVENT_OBJECT_SELECTIONWITHIN, EVENT_SYSTEM_FOREGROUND,
                FindWindowExW, GUITHREADINFO, GetClassNameW, GetGUIThreadInfo, GetParent,
                GetWindowThreadProcessId, PostMessageW, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{Interface, PCWSTR, w},
};

/// Not in the windows crate: raw input's key-up flag.
const RI_KEY_BREAK: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Explorer(isize),
    Desktop(isize),
}

impl Front {
    pub fn hwnd(self) -> isize {
        match self {
            Self::Explorer(h) | Self::Desktop(h) => h,
        }
    }

    pub fn of(hwnd: HWND) -> Option<Self> {
        let class = class_name(hwnd);
        match class.as_str() {
            "CabinetWClass" | "ExploreWClass" => Some(Self::Explorer(hwnd.0 as isize)),
            "Progman" => Some(Self::Desktop(hwnd.0 as isize)),
            // The desktop's icons sometimes live in a WorkerW instead of Progman.
            "WorkerW" if child(hwnd, w!("SHELLDLL_DefView")).is_some() => {
                Some(Self::Desktop(hwnd.0 as isize))
            }
            _ => None,
        }
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut text = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut text) };
    String::from_utf16_lossy(&text[..n.max(0) as usize])
}

fn child(parent: HWND, class: PCWSTR) -> Option<HWND> {
    unsafe { FindWindowExW(Some(parent), None, class, None) }.ok()
}

pub fn process_of(hwnd: isize) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid)) };
    pid
}

/// The keyboard focus of `front` is in its list of files, not in a name being edited, the
/// address bar or the search box.
pub fn in_file_list(front: Front) -> bool {
    let hwnd = HWND(front.hwnd() as *mut _);
    unsafe {
        let thread = GetWindowThreadProcessId(hwnd, None);
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if thread == 0
            || GetGUIThreadInfo(thread, &mut info).is_err()
            || info.hwndFocus.is_invalid()
        {
            return false;
        }
        let focus = class_name(info.hwndFocus);
        let parent = GetParent(info.hwndFocus)
            .map(class_name)
            .unwrap_or_default();
        matches!(focus.as_str(), "DirectUIHWND" | "SysListView32") && parent == "SHELLDLL_DefView"
    }
}

pub struct Ask {
    pub token: u64,
    pub front: Front,
}

pub struct Answer {
    pub token: u64,
    pub item: Option<Item>,
}

/// Reads the selection on a thread of its own: a call into a busy Explorer waits for it, and
/// on the island's thread COM would pump the island's messages meanwhile. Only the latest
/// question is answered.
pub struct Reader {
    asks: Sender<Ask>,
    answers: Receiver<Answer>,
}

impl Reader {
    pub fn start(notify: impl Fn() + Send + 'static) -> Result<Self, Failed> {
        let (asks, questions) = mpsc::channel::<Ask>();
        let (replies, answers) = mpsc::channel();
        std::thread::Builder::new()
            .name("explorer".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                while let Ok(mut ask) = questions.recv() {
                    while let Ok(later) = questions.try_recv() {
                        ask = later;
                    }
                    let item = selected(ask.front);
                    if replies
                        .send(Answer {
                            token: ask.token,
                            item,
                        })
                        .is_err()
                    {
                        break;
                    }
                    notify();
                }
            })?;
        Ok(Self { asks, answers })
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    pub fn latest(&self) -> Option<Answer> {
        self.answers.try_iter().last()
    }
}

/// The selected item of the active tab (or the desktop); of several, the one Explorer's
/// cursor is on. `None` for nothing selected or an item that isn't a file on disk.
fn selected(front: Front) -> Option<Item> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let browser = match front {
            Front::Desktop(_) => desktop(&windows)?,
            Front::Explorer(hwnd) => explorer(&windows, hwnd)?,
        };
        let view: IFolderView2 = browser.QueryActiveShellView().ok()?.cast().ok()?;
        let selection: IShellItemArray = view.GetSelection(false).ok()?;
        let count = selection.GetCount().ok()?;
        let items: Vec<Item> = (0..count)
            .filter_map(|i| selection.GetItemAt(i).ok())
            .filter_map(|item| item_of(&item))
            .collect();
        let focused = view
            .GetFocusedItem()
            .ok()
            .and_then(|i| view.GetItem::<IShellItem>(i).ok())
            .and_then(|item| item_of(&item));
        match focused {
            Some(item) if items.contains(&item) => Some(item),
            _ => items.into_iter().next(),
        }
    }
}

unsafe fn desktop(windows: &IShellWindows) -> Option<IShellBrowser> {
    let mut hwnd = 0;
    unsafe {
        let found = windows
            .FindWindowSW(
                &VARIANT::from(CSIDL_DESKTOP as i32),
                &VARIANT::default(),
                SWC_DESKTOP,
                &mut hwnd,
                SWFO_NEEDDISPATCH,
            )
            .ok()?;
        found
            .cast::<IServiceProvider>()
            .ok()?
            .QueryService(&SID_STopLevelBrowser)
            .ok()
    }
}

/// Windows 11 has a browser per tab, all under one window: the active tab's window is the
/// window's first `ShellTabWindowClass` child, and its browser says it is that one.
unsafe fn explorer(windows: &IShellWindows, hwnd: isize) -> Option<IShellBrowser> {
    let tab = child(HWND(hwnd as *mut _), w!("ShellTabWindowClass"));
    let mut fallback = None;
    unsafe {
        for i in 0..windows.Count().ok()? {
            let Some(app) = windows
                .Item(&VARIANT::from(i))
                .ok()
                .and_then(|item| item.cast::<IWebBrowserApp>().ok())
            else {
                continue;
            };
            if app.HWND().map(|h| h.0) != Ok(hwnd) {
                continue;
            }
            let Some(browser) = app
                .cast::<IServiceProvider>()
                .ok()
                .and_then(|p| p.QueryService::<IShellBrowser>(&SID_STopLevelBrowser).ok())
            else {
                continue;
            };
            match tab {
                Some(tab) if browser.GetWindow().ok() == Some(tab) => return Some(browser),
                Some(_) => fallback = fallback.or(Some(browser)),
                None => return Some(browser),
            }
        }
    }
    fallback
}

fn item_of(item: &IShellItem) -> Option<Item> {
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        let attributes = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).ok()?;
        // A zip is a folder to Explorer and a stream on the disk: a file here.
        let folder = attributes.contains(SFGAO_FOLDER) && !attributes.contains(SFGAO_STREAM);
        Some(Item {
            path: path?,
            folder,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum News {
    Foreground,
    Selection,
    Answer,
}

impl News {
    pub fn of(wparam: WPARAM) -> Self {
        match wparam.0 {
            1 => Self::Selection,
            2 => Self::Answer,
            _ => Self::Foreground,
        }
    }

    pub fn wparam(self) -> WPARAM {
        WPARAM(match self {
            Self::Foreground => 0,
            Self::Selection => 1,
            Self::Answer => 2,
        })
    }
}

static TARGET: AtomicIsize = AtomicIsize::new(0);
static MESSAGE: AtomicU32 = AtomicU32::new(0);
/// A selection of thousands of files is thousands of events: one message waits at a time.
static SELECTION_POSTED: AtomicBool = AtomicBool::new(false);

fn post(news: News) {
    let hwnd = TARGET.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                MESSAGE.load(Ordering::Relaxed),
                news.wparam(),
                LPARAM(0),
            );
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        post(News::Foreground);
    } else if !SELECTION_POSTED.swap(true, Ordering::Relaxed) {
        post(News::Selection);
    }
}

/// Windows' news about the foreground (any program) and, while following, about the focus
/// and the selection in Explorer's process. The callbacks come on the thread that set them
/// (out of context: nothing of ours runs in Explorer) and post `message` to `target`.
#[derive(Default)]
pub struct Hooks {
    foreground: Option<HWINEVENTHOOK>,
    selection: Option<(u32, HWINEVENTHOOK)>,
}

impl Hooks {
    pub fn new(target: HWND, message: u32) -> Self {
        TARGET.store(target.0 as isize, Ordering::Relaxed);
        MESSAGE.store(message, Ordering::Relaxed);
        Self::default()
    }

    pub fn watch_foreground(&mut self, on: bool) {
        match (on, self.foreground) {
            (true, None) => {
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(on_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                self.foreground = (!hook.is_invalid()).then_some(hook);
            }
            (false, Some(hook)) => {
                unsafe {
                    let _ = UnhookWinEvent(hook);
                }
                self.foreground = None;
            }
            _ => {}
        }
    }

    /// `Some(pid)`: Explorer's process to listen to; `None`: stop.
    pub fn watch_selection(&mut self, process: Option<u32>) {
        if self.selection.map(|(pid, _)| pid) == process {
            return;
        }
        if let Some((_, hook)) = self.selection.take() {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
        let Some(pid) = process.filter(|&pid| pid != 0) else {
            return;
        };
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_FOCUS,
                EVENT_OBJECT_SELECTIONWITHIN,
                None,
                Some(on_event),
                pid,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if !hook.is_invalid() {
            self.selection = Some((pid, hook));
        }
    }

    /// The posted selection message was taken: the next event may post again.
    pub fn selection_seen(&self) {
        SELECTION_POSTED.store(false, Ordering::Relaxed);
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        self.watch_foreground(false);
        self.watch_selection(None);
    }
}

/// Copies of every key go to `target` as `WM_INPUT` (`None` = they stop).
pub fn listen_keys(target: Option<HWND>) -> Result<(), Failed> {
    let device = RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: 6,
        dwFlags: if target.is_some() {
            RIDEV_INPUTSINK
        } else {
            RIDEV_REMOVE
        },
        hwndTarget: target.unwrap_or_default(),
    };
    unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32)? };
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub vk: u16,
    pub down: bool,
}

pub fn read_key(lparam: LPARAM) -> Option<Key> {
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some((&raw mut raw).cast()),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return None;
    }
    let keyboard = unsafe { raw.data.keyboard };
    Some(Key {
        vk: keyboard.VKey,
        down: keyboard.Flags & RI_KEY_BREAK == 0,
    })
}

/// Ctrl, Alt, Shift or Win is down: Space with them is someone else's.
pub fn modifiers_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}
