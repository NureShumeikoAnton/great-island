//! Recording on Windows: capture, scaling, sound, encoder, files. The island assembles them.
//! Also what the pre-start panel needs from Windows: the microphones and a meter.

mod encoder;
mod engine;
mod fade;
mod files;
mod microphones;
mod scale;
mod sound;

pub use engine::{Ending, Engine, Event, Setup, rescue};
pub use files::{TempShelf, default_dir, remove, usable_dir};
pub use microphones::{Hearing, Meter, microphones};
pub use sound::{Note, microphone_ready};
