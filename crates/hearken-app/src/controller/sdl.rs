//! The SDL side of the controller seam: [`PadDevice`] and [`PadSubsystem`]
//! over `sdl2::controller`, and the factory the app hands the manager.
//! Converting SDL's buttons and axes is `sdl_shell`'s job.

use sdl2::controller::GameController;
use sdl2::GameControllerSubsystem;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};

/// An opened pad. Dropping it closes the device.
pub struct SdlPad {
    controller: GameController,
}

impl PadDevice for SdlPad {
    fn instance_id(&self) -> u32 {
        self.controller.instance_id()
    }

    fn attached(&self) -> bool {
        self.controller.attached()
    }
}

/// The game controller subsystem. Dropping it quits the subsystem.
pub struct SdlPadSubsystem {
    subsystem: GameControllerSubsystem,
}

impl PadSubsystem for SdlPadSubsystem {
    fn count(&self) -> u32 {
        self.subsystem.num_joysticks().unwrap_or(0)
    }

    fn is_controller(&self, index: u32) -> bool {
        self.subsystem.is_game_controller(index)
    }

    fn name_for_index(&self, index: u32) -> Option<String> {
        self.subsystem.name_for_index(index).ok()
    }

    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>> {
        match self.subsystem.open(index) {
            Ok(controller) => Some(Box::new(SdlPad { controller })),
            Err(e) => {
                log::debug!("Could not open controller at slot {index}: {e}");
                None
            }
        }
    }
}

/// A factory that brings SDL's game controller subsystem up on demand.
/// Every failure (no SDL, dummy drivers) is "no subsystem": keyboard only.
pub fn sdl_factory(sdl: sdl2::Sdl) -> PadSubsystemFactory {
    Box::new(move || match sdl.game_controller() {
        Ok(subsystem) => Some(Box::new(SdlPadSubsystem { subsystem }) as Box<dyn PadSubsystem>),
        Err(e) => {
            log::info!("Controller subsystem unavailable; keyboard only: {e}");
            None
        }
    })
}
