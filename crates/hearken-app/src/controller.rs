//! Game controllers: which pad is bound, what its buttons and sticks are
//! doing, held D-pad repeat, and hot-plug.
//!
//! SDL maps any pad it recognises onto the Xbox layout, and the manager
//! binds one pad at a time. It never touches SDL itself: the subsystem and
//! the pad sit behind [`PadSubsystem`] and [`PadDevice`]; [`sdl`] has the
//! real ones and [`fakes`] the test doubles.
//!
//! The app hands every controller event to
//! [`ControllerManager::process_event`], and only the ones it accepts reach
//! a screen, so a second pad, or a pad that delivers a press twice, can
//! never fire an action twice.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `controller.rs`, without
//! its analog pedal, wheel and clutch state or its rumble.

use std::collections::{HashMap, HashSet};

use crate::input::{ControllerAxis, ControllerButton, InputEvent};

pub mod fakes;
pub mod sdl;

/// What the app says when the bound pad goes away.
pub const CONTROLLER_DISCONNECTED: &str =
    "Controller disconnected. You can keep playing with the keyboard, or reconnect your controller.";

/// Deadzones: 10% for the sticks, 4% for the triggers.
pub const STICK_DEADZONE: f64 = 0.10;
pub const TRIGGER_DEADZONE: f64 = 0.04;
pub const AXIS_MAX: f64 = 32767.0;

/// Held D-pad left or right auto-repeats, for adjusting a setting.
pub const REPEAT_DELAY_S: f64 = 0.30; // wait before the first repeat
pub const REPEAT_FAST_S: f64 = 0.03; // fastest repeat once fully accelerated
pub const REPEAT_RAMP_S: f64 = 1.50; // time held over which it accelerates

/// The device the player used last, so a prompt can name the right control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDevice {
    Keyboard,
    Controller,
}

/// Rescale a normalized axis so it is 0 inside the deadzone and reaches
/// full range at the edge, avoiding a sudden jump just past the threshold.
pub fn deadzone(value: f64, dead: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude <= dead {
        return 0.0;
    }
    let scaled = (magnitude - dead) / (1.0 - dead);
    if value >= 0.0 {
        scaled
    } else {
        -scaled
    }
}

/// The held D-pad left/right auto-repeat clock, on its own so the timing is
/// testable without a manager.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RepeatTimer {
    button: Option<ControllerButton>,
    countdown: f64,
    held: f64,
}

impl RepeatTimer {
    pub fn button(&self) -> Option<ControllerButton> {
        self.button
    }

    /// A D-pad left/right press starts the clock; other buttons are ignored.
    pub fn press(&mut self, button: ControllerButton) {
        if matches!(
            button,
            ControllerButton::DPadLeft | ControllerButton::DPadRight
        ) {
            self.button = Some(button);
            self.countdown = REPEAT_DELAY_S;
            self.held = 0.0;
        }
    }

    pub fn release(&mut self, button: ControllerButton) {
        if self.button == Some(button) {
            self.button = None;
        }
    }

    pub fn clear(&mut self) {
        self.button = None;
    }

    /// Advance `dt`; how many repeats fire this frame (the interval shrinks
    /// from `REPEAT_DELAY_S` to `REPEAT_FAST_S` over `REPEAT_RAMP_S` held).
    pub fn tick(&mut self, dt: f64) -> u32 {
        if self.button.is_none() {
            return 0;
        }
        self.held += dt;
        self.countdown -= dt;
        let mut count = 0;
        while self.countdown <= 0.0 {
            count += 1;
            let fraction = (self.held / REPEAT_RAMP_S).min(1.0);
            let interval = REPEAT_DELAY_S - (REPEAT_DELAY_S - REPEAT_FAST_S) * fraction;
            self.countdown += interval;
        }
        count
    }
}

/// One opened game controller.
pub trait PadDevice {
    /// The instance id the pad opened with -- possibly stale after a
    /// hot-plug.
    fn instance_id(&self) -> u32;
    /// Whether the pad is still physically present. A stub answers `true`
    /// so a spurious add event does not drop a working pad.
    fn attached(&self) -> bool {
        true
    }
}

/// The controller subsystem.
pub trait PadSubsystem {
    fn count(&self) -> u32;
    fn is_controller(&self, index: u32) -> bool;
    fn name_for_index(&self, index: u32) -> Option<String>;
    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>>;
}

/// Brings the subsystem up on demand; `None` when it is unavailable.
pub type PadSubsystemFactory = Box<dyn FnMut() -> Option<Box<dyn PadSubsystem>>>;

/// Owns the bound controller and decides which of its events a screen
/// sees.
pub struct ControllerManager {
    device: Option<Box<dyn PadDevice>>,
    enabled: bool,
    last_used: InputDevice,
    instance_id: Option<u32>,
    name: String,
    /// Latched until the app reads it with `take_disconnect`.
    disconnected: bool,
    /// A freshly opened pad reports a device id that a hot-plug can leave
    /// stale (the events then arrive under a different id). The opened id
    /// is provisional and the id of the first real event is adopted, so the
    /// binding always matches the live event stream.
    id_pending: bool,
    /// Buttons down now, so a duplicate press (some pads and drivers
    /// deliver a button twice) fires only once, on the genuine transition.
    buttons_down: HashSet<ControllerButton>,
    /// The last foreign instance id a dropped event was logged for, so a
    /// stream of them logs once rather than every frame.
    logged_mismatch_id: Option<u32>,
    /// The right bumper is held: the second layer of pad bindings.
    modifier: bool,
    /// Each axis past its deadzone: -1.0 to 1.0, triggers 0.0 to 1.0.
    axes: HashMap<ControllerAxis, f64>,
    repeat: RepeatTimer,
    /// Brought up only once controllers are on, so a player with them off
    /// never has a pad enumerated or bound.
    subsystem: Option<Box<dyn PadSubsystem>>,
    factory: Option<PadSubsystemFactory>,
}

impl ControllerManager {
    /// A manager over a subsystem factory (`None`: no subsystem on this
    /// machine, keyboard only).
    pub fn new(enabled: bool, factory: Option<PadSubsystemFactory>) -> Self {
        let mut manager = ControllerManager {
            device: None,
            enabled,
            last_used: InputDevice::Keyboard,
            instance_id: None,
            name: String::new(),
            disconnected: false,
            id_pending: false,
            buttons_down: HashSet::new(),
            logged_mismatch_id: None,
            modifier: false,
            axes: HashMap::new(),
            repeat: RepeatTimer::default(),
            subsystem: None,
            factory,
        };
        if enabled {
            manager.init_subsystem();
        } else {
            log::debug!("Controller support off at startup; subsystem not initialized");
        }
        manager
    }

    /// A manager that never reaches a subsystem (headless runs and tests).
    pub fn detached(enabled: bool) -> Self {
        Self::new(enabled, None)
    }

    // -- device lifecycle -----------------------------------------------------

    /// Bring the subsystem up and bind the first pad. Idempotent and
    /// headless-safe: a failure leaves the subsystem absent (keyboard only).
    fn init_subsystem(&mut self) {
        if self.subsystem.is_some() {
            return;
        }
        let Some(factory) = self.factory.as_mut() else {
            return;
        };
        match factory() {
            Some(subsystem) => {
                self.subsystem = Some(subsystem);
                log::debug!("Controller subsystem initialized");
                self.open_first();
            }
            None => log::info!("Controller subsystem unavailable; keyboard only"),
        }
    }

    /// Release the pad, then the subsystem, so no controller handles remain
    /// while support is off.
    fn teardown_subsystem(&mut self) {
        self.close_controller();
        if self.subsystem.take().is_some() {
            log::debug!("Controller subsystem torn down");
        }
    }

    pub fn subsystem_up(&self) -> bool {
        self.subsystem.is_some()
    }

    pub fn connected(&self) -> bool {
        self.device.is_some()
    }

    /// True when a controller is connected and controller input is on.
    pub fn active(&self) -> bool {
        self.enabled && self.device.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The bound instance id, `None` when nothing is bound.
    pub fn instance_id(&self) -> Option<u32> {
        self.instance_id
    }

    /// Whether the bound id is still provisional (see `id_pending`).
    pub fn id_pending(&self) -> bool {
        self.id_pending
    }

    /// Test seam: mark the bound id latched or provisional.
    pub fn set_id_pending(&mut self, pending: bool) {
        self.id_pending = pending;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            self.init_subsystem();
        } else {
            self.teardown_subsystem();
            self.release_all();
            self.last_used = InputDevice::Keyboard;
        }
    }

    /// Bind `device` (what opening a pad does, and the test seam).
    pub fn bind_device(&mut self, device: Box<dyn PadDevice>, name: &str) {
        self.instance_id = Some(device.instance_id());
        // Provisional: the first real event's instance id is authoritative
        // (see id_pending), which self-heals a stale id left by a hot-plug.
        self.id_pending = true;
        self.name = if name.is_empty() {
            "controller".to_string()
        } else {
            name.to_string()
        };
        log::info!(
            "Controller connected: {} (instance {:?})",
            self.name,
            self.instance_id
        );
        self.device = Some(device);
    }

    /// Bind the controller at `index`; true on success.
    fn open_index(&mut self, index: u32) -> bool {
        let (device, name) = {
            let Some(subsystem) = self.subsystem.as_ref() else {
                return false;
            };
            if !subsystem.is_controller(index) {
                return false;
            }
            let Some(device) = subsystem.open(index) else {
                log::debug!("Could not open controller at slot {index}");
                return false;
            };
            (device, subsystem.name_for_index(index).unwrap_or_default())
        };
        self.bind_device(device, &name);
        true
    }

    fn open_first(&mut self) {
        let count = match self.subsystem.as_ref() {
            Some(subsystem) => subsystem.count(),
            None => return,
        };
        for index in 0..count {
            if self.open_index(index) {
                return;
            }
        }
    }

    fn close_controller(&mut self) {
        self.device = None;
        self.instance_id = None;
        self.id_pending = false;
        self.buttons_down.clear();
    }

    fn is_attached(&self) -> bool {
        self.device.as_ref().is_some_and(|device| device.attached())
    }

    /// Drop the current pad and bind whatever is connected now.
    ///
    /// The first real event's instance id (see `id_pending`) corrects a
    /// stale id left by a hot-plug, so the SDL subsystem is never cycled:
    /// quitting and re-initializing it while the event loop is live
    /// re-registers SDL's controller event watch and makes every controller
    /// event arrive twice.
    pub fn reopen(&mut self) {
        self.close_controller();
        self.open_first();
    }

    /// Whether the bound pad went away since the last call.
    pub fn take_disconnect(&mut self) -> bool {
        std::mem::take(&mut self.disconnected)
    }

    /// Forget every button, stick position and repeat. The app calls this
    /// when the window loses focus: SDL drops the releases while the window
    /// is in the background, and a button remembered as down would make the
    /// next real press look like a duplicate.
    pub fn release_all(&mut self) {
        self.modifier = false;
        self.repeat.clear();
        self.buttons_down.clear();
        self.axes.clear();
    }

    // -- reads ------------------------------------------------------------------

    /// Record that the keyboard was just used, so prompts name keys.
    pub fn note_keyboard(&mut self) {
        self.last_used = InputDevice::Keyboard;
    }

    /// The controller while one is active and was used last, else the
    /// keyboard.
    pub fn device(&self) -> InputDevice {
        if self.active() && self.last_used == InputDevice::Controller {
            InputDevice::Controller
        } else {
            InputDevice::Keyboard
        }
    }

    /// Whether the right bumper is held: the second layer.
    pub fn modifier(&self) -> bool {
        self.active() && self.modifier
    }

    /// Whether `button` is held on the bound pad.
    pub fn is_down(&self, button: ControllerButton) -> bool {
        self.active() && self.buttons_down.contains(&button)
    }

    /// Where `axis` is, past its deadzone; 0.0 while no pad is active.
    pub fn axis(&self, axis: ControllerAxis) -> f64 {
        if self.active() {
            self.axes.get(&axis).copied().unwrap_or(0.0)
        } else {
            0.0
        }
    }

    // -- event handling -------------------------------------------------------

    /// Update the manager from a controller event. True for an event the
    /// top screen should see: a press or release from the bound pad, or the
    /// bound pad going away. Axis, add and foreign-id events return false,
    /// so a stray event -- a duplicate from a pad that enumerates twice --
    /// never reaches a game action.
    pub fn process_event(&mut self, event: &InputEvent) -> bool {
        let instance_id = match event {
            InputEvent::ControllerAdded { device_index } => {
                log::debug!("Controller device added (slot {device_index})");
                // Re-open when nothing is bound, or the bound pad has quietly
                // detached (a missed removal) -- otherwise a stale binding
                // would block the reconnect.
                if self.connected() && self.is_attached() {
                    log::debug!(
                        "Ignoring add; a pad is already bound (instance {:?})",
                        self.instance_id
                    );
                    return false;
                }
                self.reopen();
                return false;
            }
            InputEvent::ControllerRemoved { instance_id } => {
                log::debug!("Controller device removed (instance {instance_id})");
                if self.instance_id.is_some() && Some(*instance_id) == self.instance_id {
                    self.close_controller();
                    self.release_all();
                    self.disconnected = true;
                    self.logged_mismatch_id = None;
                    log::info!("Controller disconnected");
                    return true;
                }
                return false;
            }
            InputEvent::ControllerButtonDown { instance_id, .. }
            | InputEvent::ControllerButtonUp { instance_id, .. }
            | InputEvent::ControllerAxis { instance_id, .. } => *instance_id,
            _ => return false,
        };
        if !self.active() {
            return false;
        }
        if Some(instance_id) != self.instance_id {
            if self.id_pending {
                // First real event after (re)open: its id is authoritative.
                log::info!(
                    "Controller bound to instance {instance_id} (opened as {:?})",
                    self.instance_id
                );
                self.instance_id = Some(instance_id);
                self.id_pending = false;
                self.logged_mismatch_id = None;
            } else {
                // A genuinely foreign id: a second pad, or a duplicate from
                // a pad that enumerates twice. Drop it, logging once.
                if Some(instance_id) != self.logged_mismatch_id {
                    self.logged_mismatch_id = Some(instance_id);
                    log::debug!(
                        "Dropping controller event from instance {instance_id} (bound to {:?})",
                        self.instance_id
                    );
                }
                return false;
            }
        } else {
            // Matched the bound id, so it is live: a duplicate under a
            // different id is now a duplicate.
            self.id_pending = false;
        }
        match event {
            InputEvent::ControllerAxis { axis, value, .. } => {
                self.on_axis(*axis, *value);
                false
            }
            InputEvent::ControllerButtonDown { button, .. } => {
                if !self.buttons_down.insert(*button) {
                    log::debug!(
                        "Ignoring duplicate button-down {button:?} (instance {instance_id})"
                    );
                    return false;
                }
                self.last_used = InputDevice::Controller;
                if *button == ControllerButton::RightShoulder {
                    self.modifier = true;
                }
                self.repeat.press(*button);
                true
            }
            InputEvent::ControllerButtonUp { button, .. } => {
                if !self.buttons_down.remove(button) {
                    return false; // a stray or duplicate release
                }
                if *button == ControllerButton::RightShoulder {
                    self.modifier = false;
                }
                self.repeat.release(*button);
                true
            }
            _ => false,
        }
    }

    fn on_axis(&mut self, axis: ControllerAxis, raw: i16) {
        let value = (f64::from(raw) / AXIS_MAX).clamp(-1.0, 1.0);
        let value = match axis {
            ControllerAxis::TriggerLeft | ControllerAxis::TriggerRight => {
                deadzone(value.max(0.0), TRIGGER_DEADZONE)
            }
            _ => deadzone(value, STICK_DEADZONE),
        };
        if value != 0.0 {
            self.last_used = InputDevice::Controller;
        }
        self.axes.insert(axis, value);
    }

    /// The frame's repeats of a held D-pad left or right, as presses marked
    /// `repeat`.
    pub fn tick(&mut self, dt: f64) -> Vec<InputEvent> {
        if !self.active() {
            return Vec::new();
        }
        let Some(button) = self.repeat.button() else {
            return Vec::new();
        };
        let count = self.repeat.tick(dt);
        let instance_id = self.instance_id.unwrap_or(0);
        (0..count)
            .map(|_| InputEvent::ControllerButtonDown {
                button,
                instance_id,
                repeat: true,
            })
            .collect()
    }

    /// Release the pad and the subsystem at the end of the run, and the SDL
    /// handle the factory holds, before the window begins its exit.
    pub fn shutdown(&mut self) {
        self.teardown_subsystem();
        self.factory = None;
    }
}
