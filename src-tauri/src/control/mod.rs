//! Control center: a row of round buttons opened by swiping down on the island.
//!
//! A button implements [`Control`]; a [`Switch`] port does the system work (swapped out in tests);
//! [`ControlCenter`] is the ordered set; `adapters/` hold the Windows ports and are the only code
//! that knows the system. Button logic is pure: time comes in as island milliseconds. What the
//! island must do after a press is returned as [`Outcome`]. A change from outside (muted by a key)
//! is noticed in [`ControlCenter::refresh`]: a button's own press is already accounted for,
//! so an unexpected change is foreign.

mod actions;
pub mod adapters;
mod toggle;

use crate::fault::Failed;

pub use actions::{Recorder, Scanner, Screenshot, SettingsLink};
pub use toggle::{Flag, Switch, Toggle, Unavailable};

use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlId {
    Sound,
    Microphone,
    DoNotDisturb,
    KeepAwake,
    Screenshot,
    Scan,
    Record,
    Settings,
}

impl ControlId {
    /// Holding it does something else than a click: Screenshot opens the area selection.
    pub fn holds(self) -> bool {
        self == Self::Screenshot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlState {
    /// Normal position. One-shot actions are always Off.
    Off,
    On,
    /// Island ms the process started at: screen recording.
    Running {
        since: u64,
    },
}

impl ControlState {
    pub fn is_active(self) -> bool {
        self != Self::Off
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    /// The system did not let the switch flip: the button stays as it was.
    Refused,
    Screenshot,
    Scan,
    OpenSettings,
    Record,
}

pub trait Control {
    fn id(&self) -> ControlId;
    fn state(&self) -> ControlState;
    fn press(&mut self, now: u64) -> Outcome;
    /// Why the last press was `Refused`, when the port knows.
    fn refusal(&mut self) -> Option<Failed> {
        None
    }
    /// Rereads the state from the system (it may have changed outside the island). True = it changed.
    fn refresh(&mut self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: ControlId,
    pub state: ControlState,
}

pub struct Ports {
    pub sound: Box<dyn Switch>,
    pub microphone: Box<dyn Switch>,
    pub quiet: Box<dyn Switch>,
    pub awake: Box<dyn Switch>,
    pub recording: Rc<Cell<Option<u64>>>,
}

impl Ports {
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self {
            sound: Box::new(Flag::default()),
            microphone: Box::new(Flag::default()),
            quiet: Box::new(Flag::default()),
            awake: Box::new(Flag::default()),
            recording: Rc::default(),
        }
    }
}

pub struct ControlCenter {
    controls: Vec<Box<dyn Control>>,
}

impl ControlCenter {
    pub fn new(controls: Vec<Box<dyn Control>>) -> Self {
        Self { controls }
    }

    pub fn standard(ports: Ports) -> Self {
        Self::new(vec![
            Box::new(Toggle::new(ControlId::Sound, ports.sound)),
            Box::new(Toggle::new(ControlId::Microphone, ports.microphone)),
            Box::new(Toggle::new(ControlId::DoNotDisturb, ports.quiet)),
            Box::new(Toggle::new(ControlId::KeepAwake, ports.awake)),
            Box::new(Screenshot),
            Box::new(Scanner),
            Box::new(Recorder::new(ports.recording)),
            Box::new(SettingsLink),
        ])
    }

    pub fn is_on(&self, id: ControlId) -> bool {
        self.state(id).is_some_and(ControlState::is_active)
    }

    pub fn items(&self) -> Vec<Item> {
        self.controls
            .iter()
            .map(|c| Item {
                id: c.id(),
                state: c.state(),
            })
            .collect()
    }

    pub fn state(&self, id: ControlId) -> Option<ControlState> {
        self.controls
            .iter()
            .find(|c| c.id() == id)
            .map(|c| c.state())
    }

    pub fn press(&mut self, id: ControlId, now: u64) -> Outcome {
        match self.controls.iter_mut().find(|c| c.id() == id) {
            Some(control) => control.press(now),
            None => Outcome::Stay,
        }
    }

    pub fn refusal(&mut self, id: ControlId) -> Option<Failed> {
        self.controls
            .iter_mut()
            .find(|c| c.id() == id)
            .and_then(|c| c.refusal())
    }

    pub fn refresh(&mut self) -> Vec<ControlId> {
        self.controls
            .iter_mut()
            .filter_map(|c| c.refresh().then(|| c.id()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_order_is_the_layout_order() {
        let ids: Vec<ControlId> = ControlCenter::standard(Ports::in_memory())
            .items()
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(
            ids,
            [
                ControlId::Sound,
                ControlId::Microphone,
                ControlId::DoNotDisturb,
                ControlId::KeepAwake,
                ControlId::Screenshot,
                ControlId::Scan,
                ControlId::Record,
                ControlId::Settings,
            ]
        );
    }

    #[test]
    fn toggles_flip_and_stay_open() {
        let mut center = ControlCenter::standard(Ports::in_memory());
        assert_eq!(center.state(ControlId::Sound), Some(ControlState::Off));
        assert_eq!(center.press(ControlId::Sound, 0), Outcome::Stay);
        assert_eq!(center.state(ControlId::Sound), Some(ControlState::On));
        center.press(ControlId::Sound, 0);
        assert_eq!(center.state(ControlId::Sound), Some(ControlState::Off));
    }

    #[test]
    fn settings_ask_the_island_to_open_them() {
        let mut center = ControlCenter::standard(Ports::in_memory());
        assert_eq!(center.press(ControlId::Settings, 0), Outcome::OpenSettings);
        assert_eq!(center.state(ControlId::Settings), Some(ControlState::Off));
    }

    #[test]
    fn screenshot_asks_the_island_to_take_it() {
        let mut center = ControlCenter::standard(Ports::in_memory());
        assert_eq!(center.press(ControlId::Screenshot, 0), Outcome::Screenshot);
        assert_eq!(center.state(ControlId::Screenshot), Some(ControlState::Off));
        assert_eq!(center.press(ControlId::Scan, 0), Outcome::Scan);
    }

    #[test]
    fn record_asks_the_island_and_shows_its_time() {
        let ports = Ports::in_memory();
        let since = ports.recording.clone();
        let mut center = ControlCenter::standard(ports);
        assert_eq!(center.press(ControlId::Record, 5_000), Outcome::Record);
        assert_eq!(center.state(ControlId::Record), Some(ControlState::Off));
        since.set(Some(5_000));
        assert_eq!(
            center.state(ControlId::Record),
            Some(ControlState::Running { since: 5_000 })
        );
    }

    struct Shared(std::rc::Rc<std::cell::Cell<bool>>);

    impl Switch for Shared {
        fn get(&mut self) -> bool {
            self.0.get()
        }

        fn set(&mut self, on: bool) -> bool {
            self.0.set(on);
            true
        }
    }

    #[test]
    fn refresh_reports_only_outside_changes() {
        let system = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut center = ControlCenter::standard(Ports {
            sound: Box::new(Shared(system.clone())),
            ..Ports::in_memory()
        });
        // The button's own press is already accounted for: not news on reread.
        center.press(ControlId::Sound, 0);
        assert!(center.refresh().is_empty());
        system.set(false);
        assert_eq!(center.refresh(), [ControlId::Sound]);
        assert!(!center.is_on(ControlId::Sound));
    }

    #[test]
    fn unknown_button_does_nothing() {
        let mut center = ControlCenter::new(Vec::new());
        assert_eq!(center.press(ControlId::Sound, 0), Outcome::Stay);
        assert_eq!(center.state(ControlId::Sound), None);
        assert!(center.refresh().is_empty());
    }
}
