//! A yes-or-no question, or a choice between a few answers: the question
//! is spoken first, so the player hears the consequence before the
//! options, then the option under the cursor with its position.
//!
//! Each option runs its own closure. An option that changes the stack
//! itself (`ctx.pop_with("Route deleted.")`, `ctx.pop_to(...)`) decides
//! where the player lands; one that does not closes the confirm screen, and
//! the screen beneath announces itself. Escape picks the cancel option, the
//! last one unless [`ConfirmScreen::cancel`] says otherwise. An option
//! marked [`ConfirmOption::ends_game`] quits the run without closing the
//! screen first, so the screen beneath never speaks.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::sound::UiSound;

/// What choosing an option does, before the screen closes.
pub type OnChoose<G> = Box<dyn FnMut(&mut Ctx<G>)>;

/// One answer to the question.
pub struct ConfirmOption<G: Game> {
    label: String,
    help: Option<String>,
    on_select: OnChoose<G>,
    ends_game: bool,
}

impl<G: Game> ConfirmOption<G> {
    /// An answer that runs `on_select`, then closes the screen unless
    /// `on_select` changed the stack itself.
    pub fn new(label: impl Into<String>, on_select: impl FnMut(&mut Ctx<G>) + 'static) -> Self {
        ConfirmOption {
            label: label.into(),
            help: None,
            on_select: Box::new(on_select),
            ends_game: false,
        }
    }

    /// An answer that only closes the screen: "No", "Keep it".
    pub fn close(label: impl Into<String>) -> Self {
        ConfirmOption::new(label, |_ctx: &mut Ctx<G>| {})
    }

    /// Spoken on F1 while this option is under the cursor.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Choosing this answer runs its closure, then ends the run without
    /// closing the screen, so nothing beneath speaks after it. A closure
    /// that wants a last word ("Goodbye.") says it itself.
    pub fn ends_game(mut self) -> Self {
        self.ends_game = true;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

pub struct ConfirmScreen<G: Game> {
    menu: MenuCore<Self, G>,
    options: Vec<ConfirmOption<G>>,
    cancel: usize,
}

impl<G: Game> ConfirmScreen<G> {
    /// "Yes", which runs `on_yes`, and "No", which closes the screen and is
    /// what Escape picks. The cursor starts on Yes.
    pub fn new(question: impl Into<String>, on_yes: impl FnMut(&mut Ctx<G>) + 'static) -> Self {
        ConfirmScreen::with_options(
            question,
            vec![
                ConfirmOption::new("Yes", on_yes),
                ConfirmOption::close("No"),
            ],
        )
    }

    /// The question and its answers, in order. Escape picks the last.
    pub fn with_options(question: impl Into<String>, options: Vec<ConfirmOption<G>>) -> Self {
        let mut screen = ConfirmScreen {
            menu: MenuCore::new(question),
            cancel: options.len().saturating_sub(1),
            options,
        };
        screen.update_help();
        screen
    }

    /// Rename the first option ("Yes" from `new`).
    pub fn yes_label(mut self, label: impl Into<String>) -> Self {
        if let Some(option) = self.options.first_mut() {
            option.label = label.into();
        }
        self.update_help();
        self
    }

    /// Rename the cancel option ("No" from `new`).
    pub fn no_label(mut self, label: impl Into<String>) -> Self {
        if let Some(option) = self.options.get_mut(self.cancel) {
            option.label = label.into();
        }
        self.update_help();
        self
    }

    /// Which option Escape picks, by position. Out of range is ignored.
    pub fn cancel(mut self, index: usize) -> Self {
        if index < self.options.len() {
            self.cancel = index;
        }
        self.update_help();
        self
    }

    /// Start the cursor on the option at `index`, such as the cancel
    /// option for a question whose Yes cannot be undone.
    pub fn focus(mut self, index: usize) -> Self {
        if index < self.options.len() {
            self.menu.index = index;
        }
        self
    }

    /// Start the cursor on the cancel option.
    pub fn focus_cancel(self) -> Self {
        let cancel = self.cancel;
        self.focus(cancel)
    }

    fn update_help(&mut self) {
        let cancel = self
            .options
            .get(self.cancel)
            .map(|o| o.label.clone())
            .unwrap_or_default();
        self.menu.intro_help = format!("Up and Down move, Enter chooses, Escape chooses {cancel}.");
    }

    fn choose(&mut self, ctx: &mut Ctx<G>, index: usize) {
        let Some(option) = self.options.get_mut(index) else {
            return;
        };
        let before = ctx.pending_len();
        (option.on_select)(ctx);
        if option.ends_game {
            ctx.quit();
        } else if ctx.pending_len() == before && ctx.running {
            ctx.pop();
        }
    }
}

impl<G: Game> Menu for ConfirmScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "confirm"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        self.options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let item = MenuItem::new(option.label.as_str(), move |s: &mut Self, ctx| {
                    s.choose(ctx, i)
                });
                match &option.help {
                    Some(help) => item.help(help.as_str()),
                    None => item,
                }
            })
            .collect()
    }

    /// `menu_enter`, keeping the cursor where `focus` put it.
    fn enter(&mut self, ctx: &mut Ctx<G>) {
        ctx.play_ui(UiSound::Open);
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    /// Picks the cancel option, as if it were chosen.
    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.play_ui(UiSound::Back);
        let cancel = self.cancel;
        self.choose(ctx, cancel);
    }
}
