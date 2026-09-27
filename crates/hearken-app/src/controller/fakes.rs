//! Test doubles for the controller seam: a pad whose presence the test
//! controls, and a subsystem that hands out pads and counts how often it
//! was brought up, torn down and asked to open one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};

/// A pad with a fixed instance id.
pub struct FakePad {
    pub instance_id: u32,
    /// Set false to unplug it without a removal event.
    pub attached: Rc<Cell<bool>>,
}

impl FakePad {
    pub fn new(instance_id: u32) -> Self {
        FakePad {
            instance_id,
            attached: Rc::new(Cell::new(true)),
        }
    }
}

impl PadDevice for FakePad {
    fn instance_id(&self) -> u32 {
        self.instance_id
    }

    fn attached(&self) -> bool {
        self.attached.get()
    }
}

/// What a [`FakeSubsystem`] and its factory saw.
#[derive(Debug, Default)]
pub struct FakeSdlLog {
    /// How many times the factory brought a subsystem up.
    pub inits: u32,
    /// How many subsystems were dropped.
    pub quits: u32,
    /// Every `open(index)` call.
    pub opens: Vec<u32>,
}

/// A subsystem with `pads` recognised controllers (name, instance id).
pub struct FakeSubsystem {
    pub pads: Vec<(String, u32)>,
    pub log: Rc<RefCell<FakeSdlLog>>,
}

impl PadSubsystem for FakeSubsystem {
    fn count(&self) -> u32 {
        self.pads.len() as u32
    }

    fn is_controller(&self, index: u32) -> bool {
        (index as usize) < self.pads.len()
    }

    fn name_for_index(&self, index: u32) -> Option<String> {
        self.pads.get(index as usize).map(|(name, _)| name.clone())
    }

    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>> {
        self.log.borrow_mut().opens.push(index);
        let (_, instance_id) = self.pads.get(index as usize)?;
        Some(Box::new(FakePad::new(*instance_id)))
    }
}

impl Drop for FakeSubsystem {
    fn drop(&mut self) {
        self.log.borrow_mut().quits += 1;
    }
}

/// A factory handing out [`FakeSubsystem`]s with `pads`, recording into
/// `log`.
pub fn fake_factory(pads: Vec<(String, u32)>, log: Rc<RefCell<FakeSdlLog>>) -> PadSubsystemFactory {
    Box::new(move || {
        log.borrow_mut().inits += 1;
        Some(Box::new(FakeSubsystem {
            pads: pads.clone(),
            log: Rc::clone(&log),
        }))
    })
}
