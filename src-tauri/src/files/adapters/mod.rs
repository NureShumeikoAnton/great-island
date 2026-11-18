//! The shell side of files every feature shares: Windows' known folders, opening a file,
//! showing files in Explorer.

mod shell;

pub use shell::{known_folder, open, show_in_folder, show_together};
