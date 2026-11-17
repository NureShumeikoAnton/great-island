//! Screenshot ports on Windows: screen capture and the folder. The clipboard is the shared one
//! from `capture`. Only they know the system; the island assembles them.

mod folder;
mod screen;

pub use folder::{Folder, default_dir};
pub use screen::capture;

use super::Clipboard;
use crate::{
    capture::{Image, adapters::SystemClipboard},
    fault::Failed,
};

impl Clipboard for SystemClipboard {
    fn put(&mut self, image: &Image) -> Result<(), Failed> {
        self.put_image(image)
    }
}
