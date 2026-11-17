//! Keep awake: blocks sleep for the island's thread (SetThreadExecutionState).
//! Lasts only while the process lives: quit the island and the PC sleeps normally again.

use crate::control::Switch;
use std::{cell::Cell, rc::Rc};
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

pub struct KeepAwake {
    on: bool,
    /// Also keep the screen on; shared with the island, which updates it when settings are saved.
    screen: Rc<Cell<bool>>,
    applied: Option<(bool, bool)>,
}

impl KeepAwake {
    pub fn new(screen: Rc<Cell<bool>>) -> Self {
        Self {
            on: false,
            screen,
            applied: None,
        }
    }

    fn apply(&mut self) -> bool {
        let wanted = (self.on, self.on && self.screen.get());
        if self.applied == Some(wanted) {
            return true;
        }
        let mut flags = ES_CONTINUOUS;
        if wanted.0 {
            flags |= ES_SYSTEM_REQUIRED;
        }
        if wanted.1 {
            flags |= ES_DISPLAY_REQUIRED;
        }
        // Zero = the system refused.
        let ok = unsafe { SetThreadExecutionState(flags) }.0 != 0;
        if ok {
            self.applied = Some(wanted);
        }
        ok
    }
}

impl Switch for KeepAwake {
    /// Also picks up a change of the screen setting.
    fn get(&mut self) -> bool {
        if self.on {
            self.apply();
        }
        self.on
    }

    fn set(&mut self, on: bool) -> bool {
        let was = self.on;
        self.on = on;
        if !self.apply() {
            self.on = was;
            return false;
        }
        true
    }
}
