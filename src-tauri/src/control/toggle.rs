//! Toggles: a button holds the position and delegates the system change to a port.

use super::{Control, ControlId, ControlState, Outcome};
use crate::fault::Failed;

pub trait Switch {
    /// Current position. May also pick up outside changes (default device switched, a setting changed).
    fn get(&mut self) -> bool;
    /// Sets the position. False = the system refused.
    fn set(&mut self, on: bool) -> bool;
    /// Why the last `set` was refused, when the port knows.
    fn refusal(&mut self) -> Option<Failed> {
        None
    }
}

impl Switch for Box<dyn Switch> {
    fn get(&mut self) -> bool {
        (**self).get()
    }

    fn set(&mut self, on: bool) -> bool {
        (**self).set(on)
    }

    fn refusal(&mut self) -> Option<Failed> {
        (**self).refusal()
    }
}

#[derive(Default)]
pub struct Flag {
    on: bool,
}

impl Flag {
    pub fn new(on: bool) -> Self {
        Self { on }
    }
}

impl Switch for Flag {
    fn get(&mut self) -> bool {
        self.on
    }

    fn set(&mut self, on: bool) -> bool {
        self.on = on;
        true
    }
}

/// System side unavailable (could not connect to audio): the button stays in its normal position and doesn't toggle.
pub struct Unavailable;

impl Switch for Unavailable {
    fn get(&mut self) -> bool {
        false
    }

    fn set(&mut self, _on: bool) -> bool {
        false
    }
}

pub struct Toggle<S: Switch> {
    id: ControlId,
    switch: S,
    on: bool,
}

impl<S: Switch> Toggle<S> {
    pub fn new(id: ControlId, mut switch: S) -> Self {
        let on = switch.get();
        Self { id, switch, on }
    }
}

impl<S: Switch> Control for Toggle<S> {
    fn id(&self) -> ControlId {
        self.id
    }

    fn state(&self) -> ControlState {
        if self.on {
            ControlState::On
        } else {
            ControlState::Off
        }
    }

    fn press(&mut self, _now: u64) -> Outcome {
        if self.switch.set(!self.on) {
            self.on = !self.on;
            Outcome::Stay
        } else {
            Outcome::Refused
        }
    }

    fn refusal(&mut self) -> Option<Failed> {
        self.switch.refusal()
    }

    fn refresh(&mut self) -> bool {
        let on = self.switch.get();
        let changed = on != self.on;
        self.on = on;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct Fake {
        on: Rc<Cell<bool>>,
        allowed: bool,
    }

    impl Switch for Fake {
        fn get(&mut self) -> bool {
            self.on.get()
        }

        fn set(&mut self, on: bool) -> bool {
            if self.allowed {
                self.on.set(on);
            }
            self.allowed
        }
    }

    #[test]
    fn starts_from_the_system_position() {
        let on = Rc::new(Cell::new(true));
        let toggle = Toggle::new(ControlId::Sound, Fake { on, allowed: true });
        assert_eq!(toggle.state(), ControlState::On);
    }

    #[test]
    fn refused_switch_keeps_the_position() {
        let on = Rc::new(Cell::new(false));
        let mut toggle = Toggle::new(ControlId::Microphone, Fake { on, allowed: false });
        assert_eq!(toggle.press(0), Outcome::Refused);
        assert_eq!(toggle.state(), ControlState::Off);
    }

    #[test]
    fn refresh_notices_outside_changes() {
        let on = Rc::new(Cell::new(false));
        let mut toggle = Toggle::new(
            ControlId::Sound,
            Fake {
                on: on.clone(),
                allowed: true,
            },
        );
        assert!(!toggle.refresh());
        on.set(true);
        assert!(toggle.refresh());
        assert_eq!(toggle.state(), ControlState::On);
    }
}
