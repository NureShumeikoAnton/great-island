//! The Windows side of the devices: paired Bluetooth devices and the battery Windows reads from
//! them (`bluetooth`), the default sound endpoints (`endpoints`), both tied together on a
//! thread of their own (`watch`).

mod bluetooth;
mod endpoints;
mod watch;

pub use watch::Watch;
