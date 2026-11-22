//! Screenshot ports on Windows: the folder. The screen grab and the clipboard are the shared
//! ones from `capture`. Only they know the system; the island assembles them.

mod folder;

pub use folder::{Folder, default_dir};

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
