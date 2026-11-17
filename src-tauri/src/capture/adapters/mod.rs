//! The Windows side a screenshot and a recording share: the clipboard, file names and the shell,
//! the screen. What only one of them needs stays in its own `adapters/`.

mod clipboard;
mod files;
mod screen;

pub use clipboard::SystemClipboard;
pub use files::{free_name, known_folder, open, show_in_folder};
pub use screen::{hide_from_capture, now, screen_rect, wait_for_hiding, whole_monitor};
