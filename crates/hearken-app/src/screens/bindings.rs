//! Keyboard shortcuts, or controller buttons: one row per control, naming
//! the key or button it is on now. Enter on a row, then press the key or
//! button you want for it.
//!
//! The capture is one press. A key that is fixed, or already another
//! control's, is refused with the reason and the screen keeps listening;
//! Escape keeps what the row had. On the controller the right bumper held
//! during the press picks the second layer, and pressing the button the row
//! already has keeps it. Each change is saved as it lands, so a crash a
//! minute later loses nothing.
//!
//! Extracted from Freight Fate (commit c5d3ef8d),
//! `states/main_menu/shortcuts.rs`.

use crate::bindings::{Action, Chord, PadChord, Rebind};
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{ControllerButton, InputEvent, Key, Mods};
use crate::menu::{menu_input, Label, Menu, MenuCore, MenuItem};
use crate::text::sentence_start;

/// Which set of bindings a [`BindingsScreen`] moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingDevice {
    Keyboard,
    Controller,
}

pub struct BindingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
    device: BindingDevice,
    /// The control waiting for its new key or button.
    capturing: Option<G::Action>,
}

impl<G: Game> BindingsScreen<G> {
    pub fn new(device: BindingDevice) -> Self {
        let (title, help) = match device {
            BindingDevice::Keyboard => (
                "Keyboard shortcuts",
                "Up and Down pick a control, Enter then a key press moves it, Escape goes back.",
            ),
            BindingDevice::Controller => (
                "Controller buttons",
                "Up and Down pick a control, Enter then a button press moves it, Escape goes back.",
            ),
        };
        BindingsScreen {
            menu: MenuCore::new(title).with_intro_help(help),
            device,
            capturing: None,
        }
    }

    /// The control waiting for its new key or button, if any.
    pub fn capturing(&self) -> Option<G::Action> {
        self.capturing
    }

    fn current_name(&self, ctx: &Ctx<G>, action: G::Action) -> String {
        match self.device {
            BindingDevice::Keyboard => ctx.bindings.spoken(action),
            BindingDevice::Controller => ctx.bindings.pad_spoken(action),
        }
    }

    fn prompt(&self, ctx: &Ctx<G>, action: G::Action) -> String {
        let current = self.current_name(ctx, action);
        match self.device {
            BindingDevice::Keyboard => format!(
                "Press the new key for {}. Escape keeps {current}.",
                action.label()
            ),
            BindingDevice::Controller => format!(
                "Press the new button for {}, with the right bumper held for the second \
                 layer. Press {current} to keep it.",
                action.label()
            ),
        }
    }

    /// How a refusal ends: the way to keep what the control has.
    fn keep_hint(&self, current: &str) -> String {
        match self.device {
            BindingDevice::Keyboard => format!("Press another, or keep {current} with Escape."),
            BindingDevice::Controller => format!("Press another, or press {current} to keep it."),
        }
    }

    fn begin_capture(&mut self, ctx: &mut Ctx<G>, action: G::Action) {
        self.capturing = Some(action);
        let prompt = self.prompt(ctx, action);
        ctx.say(prompt);
    }

    fn keep(&mut self, ctx: &mut Ctx<G>, action: G::Action) {
        self.capturing = None;
        let current = self.current_name(ctx, action);
        ctx.say(format!("Kept {current}."));
    }

    fn finish(
        &mut self,
        ctx: &mut Ctx<G>,
        action: G::Action,
        result: Rebind<G::Action>,
        chosen: &str,
    ) {
        match result {
            Rebind::Done => {
                self.capturing = None;
                ctx.save_bindings();
                let now = self.current_name(ctx, action);
                ctx.say(format!("{} is now {now}.", action.label()));
            }
            Rebind::Unchanged => self.keep(ctx, action),
            Rebind::Taken(other) => {
                let hint = self.keep_hint(&self.current_name(ctx, action));
                ctx.say(format!(
                    "{} is already used for {}. {hint}",
                    sentence_start(chosen),
                    other.label()
                ));
            }
            Rebind::Reserved(reason) => {
                let hint = self.keep_hint(&self.current_name(ctx, action));
                ctx.say(format!("{}. {hint}", sentence_start(reason)));
            }
        }
    }

    fn capture_key(&mut self, ctx: &mut Ctx<G>, action: G::Action, key: Key, mods: Mods) {
        if matches!(
            key,
            Key::LCtrl | Key::RCtrl | Key::LShift | Key::RShift | Key::LAlt | Key::RAlt
        ) {
            return; // half a chord: wait for the key that goes with it
        }
        let chord = Chord { key, mods };
        let result = ctx.bindings.set_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn capture_button(&mut self, ctx: &mut Ctx<G>, action: G::Action, button: ControllerButton) {
        if button == ControllerButton::RightShoulder {
            return; // the second-layer key, held with the button that is the choice
        }
        let chord = PadChord {
            button,
            modified: ctx.controller.modifier(),
        };
        let result = ctx.bindings.set_pad_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn reset_all(&mut self, ctx: &mut Ctx<G>) {
        let line = match self.device {
            BindingDevice::Keyboard => {
                ctx.bindings.reset_keys();
                "Every keyboard shortcut is back to its default."
            }
            BindingDevice::Controller => {
                ctx.bindings.reset_pad();
                "Every controller button is back to its default."
            }
        };
        ctx.save_bindings();
        ctx.say(line);
    }
}

impl<G: Game> Menu for BindingsScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "bindings"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let device = self.device;
        let row_help = match device {
            BindingDevice::Keyboard => {
                "Enter, then press the key you want for this control. Escape keeps the one it has."
            }
            BindingDevice::Controller => {
                "Enter, then press the button you want for this control, with the right bumper \
                 held for the second layer."
            }
        };
        let mut items: Vec<MenuItem<Self, G>> = <G::Action as Action>::all()
            .iter()
            .copied()
            .filter(|action| match device {
                BindingDevice::Keyboard => !action.default_keys().is_empty(),
                BindingDevice::Controller => !action.default_pad().is_empty(),
            })
            .map(|action| {
                // The label is read live, so a row names its new key the
                // moment it moves.
                MenuItem::new(
                    Label::dynamic(move |s: &Self, ctx: &Ctx<G>| {
                        format!("{}: {}", action.label(), s.current_name(ctx, action))
                    }),
                    move |s: &mut Self, ctx: &mut Ctx<G>| s.begin_capture(ctx, action),
                )
                .help(row_help)
            })
            .collect();
        items.push(
            MenuItem::new(
                match device {
                    BindingDevice::Keyboard => "Reset every keyboard shortcut",
                    BindingDevice::Controller => "Reset every controller button",
                },
                |s: &mut Self, ctx: &mut Ctx<G>| s.reset_all(ctx),
            )
            .help("Puts every control on this screen back where it started."),
        );
        items
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(action) = self.capturing else {
            return menu_input(self, ctx, event);
        };
        if event.is_repeat() {
            return;
        }
        match (self.device, event) {
            (
                _,
                InputEvent::KeyDown {
                    key: Key::Escape, ..
                },
            ) => self.keep(ctx, action),
            (BindingDevice::Keyboard, InputEvent::KeyDown { key, mods, .. }) => {
                self.capture_key(ctx, action, *key, *mods)
            }
            (BindingDevice::Controller, InputEvent::ControllerButtonDown { button, .. }) => {
                self.capture_button(ctx, action, *button)
            }
            // A button on the keyboard screen, or a key on the controller
            // screen, is not a choice.
            _ => {}
        }
    }

    /// While listening, F1 repeats what the screen is waiting for.
    fn help(&self, ctx: &Ctx<G>) -> String {
        match self.capturing {
            Some(action) => self.prompt(ctx, action),
            None => self
                .current_help(ctx)
                .unwrap_or_else(|| self.menu().intro_help.clone()),
        }
    }
}
