//! The Windows side a screenshot and a recording share: the clipboard, file names, the screen.
//! What only one of them needs stays in its own `adapters/`; the shell side of files (known
//! folders, opening, Explorer) is shared wider, in `crate::files`.

mod clipboard;
mod files;
pub mod graphics;
mod screen;

pub use crate::files::adapters::{known_folder, open, show_in_folder};
pub use clipboard::SystemClipboard;
pub use files::free_name;
pub use screen::{hide_from_capture, now, screen_rect, wait_for_hiding, whole_monitor};
