//! The Windows side of downloads: the Downloads folder watched on a thread of its own.

mod watch;

pub use watch::{Arrival, Event, Watch, folder};
