//! The SDL window: a focus target for the keyboard. Its title is the
//! game's and never changes, since a screen reader can announce a title
//! change.

use std::time::{Duration, Instant};

use sdl2::controller::{Axis, Button};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::Color;
use sdl2::render::WindowCanvas;
use sdl2::EventPump;

use crate::input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
use crate::shell::{Shell, FRAME_SECONDS};

pub struct SdlShell {
    _sdl: sdl2::Sdl,
    canvas: WindowCanvas,
    pump: EventPump,
    last: Instant,
}

impl SdlShell {
    pub fn new(title: &str) -> Result<Self, String> {
        let sdl = sdl2::init()?;
        let video = sdl.video()?;
        let window = video
            .window(title, 640, 360)
            .position_centered()
            .build()
            .map_err(|e| e.to_string())?;
        let canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
        let pump = sdl.event_pump()?;
        Ok(SdlShell {
            _sdl: sdl,
            canvas,
            pump,
            last: Instant::now(),
        })
    }
}

impl Shell for SdlShell {
    fn poll(&mut self) -> Vec<InputEvent> {
        translate_events(self.pump.poll_iter().collect())
    }

    fn render(&mut self, _lines: &[String]) {
        self.canvas.set_draw_color(Color::RGB(12, 24, 16));
        self.canvas.clear();
        self.canvas.present();
    }

    fn frame(&mut self) -> f64 {
        let target = Duration::from_secs_f64(FRAME_SECONDS);
        let spent = self.last.elapsed();
        if spent < target {
            std::thread::sleep(target - spent);
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        dt.min(0.25)
    }
}

/// SDL events to game events, pairing each `KeyDown` with the `TextInput`
/// that follows it.
pub fn translate_events(raw: Vec<Event>) -> Vec<InputEvent> {
    let mut out = Vec::with_capacity(raw.len());
    let mut pending_text: Option<usize> = None; // index in `out` of the last KeyDown
    for event in raw {
        match event {
            Event::KeyDown {
                keycode,
                keymod,
                repeat,
                ..
            } => {
                let key = keycode.map(key_from_keycode).unwrap_or(Key::Other(0));
                out.push(InputEvent::KeyDown {
                    key,
                    mods: mods_from(keymod),
                    text: None,
                    repeat,
                });
                pending_text = Some(out.len() - 1);
                continue;
            }
            Event::TextInput { text, .. } => {
                if let Some(index) = pending_text.take() {
                    if let Some(InputEvent::KeyDown { text: slot, .. }) = out.get_mut(index) {
                        *slot = text.chars().next();
                    }
                }
                continue;
            }
            Event::KeyUp {
                keycode, keymod, ..
            } => {
                let key = keycode.map(key_from_keycode).unwrap_or(Key::Other(0));
                out.push(InputEvent::KeyUp {
                    key,
                    mods: mods_from(keymod),
                });
            }
            Event::ControllerButtonDown { which, button, .. } => {
                out.push(InputEvent::ControllerButtonDown {
                    button: button_from_sdl(button),
                    instance_id: which,
                    repeat: false,
                });
            }
            Event::ControllerButtonUp { which, button, .. } => {
                out.push(InputEvent::ControllerButtonUp {
                    button: button_from_sdl(button),
                    instance_id: which,
                });
            }
            Event::ControllerAxisMotion {
                which, axis, value, ..
            } => {
                out.push(InputEvent::ControllerAxis {
                    axis: axis_from_sdl(axis),
                    value,
                    instance_id: which,
                });
            }
            Event::ControllerDeviceAdded { which, .. } => {
                out.push(InputEvent::ControllerAdded {
                    device_index: which,
                });
            }
            Event::ControllerDeviceRemoved { which, .. } => {
                out.push(InputEvent::ControllerRemoved { instance_id: which });
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => out.push(InputEvent::WindowFocusGained),
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => out.push(InputEvent::WindowFocusLost),
            Event::Quit { .. } => out.push(InputEvent::Quit),
            _ => {}
        }
        pending_text = None;
    }
    out
}

/// `event.mod & KMOD_*`.
pub fn mods_from(keymod: Mod) -> Mods {
    Mods {
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl: keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
    }
}

/// SDL keycode to the game's key set.
pub fn key_from_keycode(keycode: Keycode) -> Key {
    match keycode {
        Keycode::A => Key::A,
        Keycode::B => Key::B,
        Keycode::C => Key::C,
        Keycode::D => Key::D,
        Keycode::E => Key::E,
        Keycode::F => Key::F,
        Keycode::G => Key::G,
        Keycode::H => Key::H,
        Keycode::I => Key::I,
        Keycode::J => Key::J,
        Keycode::K => Key::K,
        Keycode::L => Key::L,
        Keycode::M => Key::M,
        Keycode::N => Key::N,
        Keycode::O => Key::O,
        Keycode::P => Key::P,
        Keycode::Q => Key::Q,
        Keycode::R => Key::R,
        Keycode::S => Key::S,
        Keycode::T => Key::T,
        Keycode::U => Key::U,
        Keycode::V => Key::V,
        Keycode::W => Key::W,
        Keycode::X => Key::X,
        Keycode::Y => Key::Y,
        Keycode::Z => Key::Z,
        Keycode::NUM_0 => Key::Num0,
        Keycode::NUM_1 => Key::Num1,
        Keycode::NUM_2 => Key::Num2,
        Keycode::NUM_3 => Key::Num3,
        Keycode::NUM_4 => Key::Num4,
        Keycode::NUM_5 => Key::Num5,
        Keycode::NUM_6 => Key::Num6,
        Keycode::NUM_7 => Key::Num7,
        Keycode::NUM_8 => Key::Num8,
        Keycode::NUM_9 => Key::Num9,
        Keycode::KP_0 => Key::Kp0,
        Keycode::KP_1 => Key::Kp1,
        Keycode::KP_2 => Key::Kp2,
        Keycode::KP_3 => Key::Kp3,
        Keycode::KP_4 => Key::Kp4,
        Keycode::KP_5 => Key::Kp5,
        Keycode::KP_6 => Key::Kp6,
        Keycode::KP_7 => Key::Kp7,
        Keycode::KP_8 => Key::Kp8,
        Keycode::KP_9 => Key::Kp9,
        Keycode::KP_ENTER => Key::KpEnter,
        Keycode::KP_PLUS => Key::KpPlus,
        Keycode::KP_MINUS => Key::KpMinus,
        Keycode::F1 => Key::F1,
        Keycode::F2 => Key::F2,
        Keycode::F3 => Key::F3,
        Keycode::F4 => Key::F4,
        Keycode::F5 => Key::F5,
        Keycode::F6 => Key::F6,
        Keycode::F7 => Key::F7,
        Keycode::F8 => Key::F8,
        Keycode::F9 => Key::F9,
        Keycode::F10 => Key::F10,
        Keycode::F11 => Key::F11,
        Keycode::F12 => Key::F12,
        Keycode::RETURN => Key::Return,
        Keycode::ESCAPE => Key::Escape,
        Keycode::SPACE => Key::Space,
        Keycode::TAB => Key::Tab,
        Keycode::BACKSPACE => Key::Backspace,
        Keycode::UP => Key::Up,
        Keycode::DOWN => Key::Down,
        Keycode::LEFT => Key::Left,
        Keycode::RIGHT => Key::Right,
        Keycode::HOME => Key::Home,
        Keycode::END => Key::End,
        Keycode::PAGEUP => Key::PageUp,
        Keycode::PAGEDOWN => Key::PageDown,
        Keycode::INSERT => Key::Insert,
        Keycode::DELETE => Key::Delete,
        Keycode::COMMA => Key::Comma,
        Keycode::PERIOD => Key::Period,
        Keycode::SLASH => Key::Slash,
        Keycode::BACKSLASH => Key::Backslash,
        Keycode::BACKQUOTE => Key::Backquote,
        Keycode::EQUALS => Key::Equals,
        Keycode::PLUS => Key::Plus,
        Keycode::MINUS => Key::Minus,
        Keycode::SEMICOLON => Key::Semicolon,
        Keycode::QUOTE => Key::Quote,
        Keycode::LEFTBRACKET => Key::LeftBracket,
        Keycode::RIGHTBRACKET => Key::RightBracket,
        Keycode::LCTRL => Key::LCtrl,
        Keycode::RCTRL => Key::RCtrl,
        Keycode::LSHIFT => Key::LShift,
        Keycode::RSHIFT => Key::RShift,
        Keycode::LALT => Key::LAlt,
        Keycode::RALT => Key::RAlt,
        other => Key::Other(other.into_i32()),
    }
}

pub fn button_from_sdl(button: Button) -> ControllerButton {
    match button {
        Button::A => ControllerButton::A,
        Button::B => ControllerButton::B,
        Button::X => ControllerButton::X,
        Button::Y => ControllerButton::Y,
        Button::Back => ControllerButton::Back,
        Button::Guide => ControllerButton::Guide,
        Button::Start => ControllerButton::Start,
        Button::LeftStick => ControllerButton::LeftStick,
        Button::RightStick => ControllerButton::RightStick,
        Button::LeftShoulder => ControllerButton::LeftShoulder,
        Button::RightShoulder => ControllerButton::RightShoulder,
        Button::DPadUp => ControllerButton::DPadUp,
        Button::DPadDown => ControllerButton::DPadDown,
        Button::DPadLeft => ControllerButton::DPadLeft,
        Button::DPadRight => ControllerButton::DPadRight,
        Button::Misc1 => ControllerButton::Misc1,
        Button::Paddle1 => ControllerButton::Paddle1,
        Button::Paddle2 => ControllerButton::Paddle2,
        Button::Paddle3 => ControllerButton::Paddle3,
        Button::Paddle4 => ControllerButton::Paddle4,
        Button::Touchpad => ControllerButton::Touchpad,
    }
}

pub fn axis_from_sdl(axis: Axis) -> ControllerAxis {
    match axis {
        Axis::LeftX => ControllerAxis::LeftX,
        Axis::LeftY => ControllerAxis::LeftY,
        Axis::RightX => ControllerAxis::RightX,
        Axis::RightY => ControllerAxis::RightY,
        Axis::TriggerLeft => ControllerAxis::TriggerLeft,
        Axis::TriggerRight => ControllerAxis::TriggerRight,
    }
}
