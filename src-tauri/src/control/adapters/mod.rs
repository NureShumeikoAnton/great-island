//! Windows adapters for the control-center ports. All system work is here;
//! buttons (`Toggle`) see only the `Switch` trait.

mod audio;
mod awake;

pub use audio::{DeviceMute, Flow};
pub use awake::KeepAwake;

/// Called from a system thread when the state may have changed from outside;
/// the island rereads the buttons on its own thread.
pub type Notify = std::sync::Arc<dyn Fn() + Send + Sync>;
