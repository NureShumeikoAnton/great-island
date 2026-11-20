//! The Windows side of files: the shell every feature shares (Windows' known folders,
//! opening a file, showing files in Explorer), photos through WIC, the Recycle Bin, the
//! "Open with" registration and Explorer's selection for Space.

pub mod association;
mod bin;
pub mod explorer;
mod photo;
mod shell;

pub use bin::recycle;
pub use photo::{Disk, read as read_photo};
pub use shell::{known_folder, open, show_in_folder, show_together};
