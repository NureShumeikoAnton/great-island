//! Action buttons: screenshot, screen recording, settings.

use super::{Control, ControlId, ControlState, Outcome};
use std::{cell::Cell, rc::Rc};

/// The island takes the shot itself (closes the menu, counts down the shutter, shows the card); the button only asks.
pub struct Screenshot;

impl Control for Screenshot {
    fn id(&self) -> ControlId {
        ControlId::Screenshot
    }

    fn state(&self) -> ControlState {
        ControlState::Off
    }

    fn press(&mut self, _now: u64) -> Outcome {
        Outcome::Screenshot
    }
}

/// Screen recording. The island records; the button asks it to start or stop and shows
/// the time from the shared cell: island ms the recording counts from (pauses excluded).
pub struct Recorder {
    since: Rc<Cell<Option<u64>>>,
}

impl Recorder {
    pub fn new(since: Rc<Cell<Option<u64>>>) -> Self {
        Self { since }
    }
}

impl Control for Recorder {
    fn id(&self) -> ControlId {
        ControlId::Record
    }

    fn state(&self) -> ControlState {
        match self.since.get() {
            Some(since) => ControlState::Running { since },
            None => ControlState::Off,
        }
    }

    fn press(&mut self, _now: u64) -> Outcome {
        Outcome::Record
    }
}

pub struct SettingsLink;

impl Control for SettingsLink {
    fn id(&self) -> ControlId {
        ControlId::Settings
    }

    fn state(&self) -> ControlState {
        ControlState::Off
    }

    fn press(&mut self, _now: u64) -> Outcome {
        Outcome::OpenSettings
    }
}
