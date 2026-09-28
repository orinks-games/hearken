//! The accessible menu: a list of rows navigated by ear. Every move speaks
//! the row it lands on, Enter runs the row's action, Left and Right adjust a
//! row that has an adjuster and speak the new value, and F1 (handled by the
//! app) reads the row's help.
//!
//! A screen embeds a [`MenuCore`] and implements [`Menu`]; every `Menu` is a
//! [`State`] through the blanket impl at the bottom of this file.

use hearken_audio::Bus;

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::sound::UiSound;
use crate::state::State;
use crate::text::end_sentence;

/// A label computed while the menu is open. Rows whose text depends on
/// state that changes under them must be read through this, never off a
/// string captured when the menu was built.
pub type DynamicLabel<S, G> = Box<dyn Fn(&S, &Ctx<G>) -> String>;
/// What activating a row does.
pub type MenuAction<S, G> = Box<dyn FnMut(&mut S, &mut Ctx<G>)>;
/// What Left and Right do to a row, given -1 or +1.
pub type MenuAdjust<S, G> = Box<dyn FnMut(&mut S, &mut Ctx<G>, i32)>;

/// A row's label or help: fixed text, or text computed on demand.
pub enum Label<S, G: Game> {
    Text(String),
    Dynamic(DynamicLabel<S, G>),
}

impl<S, G: Game> Label<S, G> {
    pub fn dynamic(f: impl Fn(&S, &Ctx<G>) -> String + 'static) -> Self {
        Label::Dynamic(Box::new(f))
    }

    pub fn text(&self, state: &S, ctx: &Ctx<G>) -> String {
        match self {
            Label::Text(text) => text.clone(),
            Label::Dynamic(f) => f(state, ctx),
        }
    }
}

impl<S, G: Game> From<&str> for Label<S, G> {
    fn from(text: &str) -> Self {
        Label::Text(text.to_string())
    }
}

impl<S, G: Game> From<String> for Label<S, G> {
    fn from(text: String) -> Self {
        Label::Text(text)
    }
}

/// One row of a menu.
pub struct MenuItem<S, G: Game> {
    label: Label<S, G>,
    help: Option<Label<S, G>>,
    action: MenuAction<S, G>,
    /// Set when Left and Right change this row instead of doing nothing.
    pub adjust: Option<MenuAdjust<S, G>>,
    /// The row's own select sound: outer `None` plays the menu's `UiSound::Select`
    /// default; `Some(None)` plays nothing; `Some(Some(key))` plays `key`.
    select: Option<Option<&'static str>>,
    /// Whether Left and Right play `UiSound::Move` on this row. `silent_adjust`
    /// turns it off for a row that plays its own sound.
    adjust_sound: bool,
}

impl<S: 'static, G: Game> MenuItem<S, G> {
    pub fn new(
        label: impl Into<Label<S, G>>,
        action: impl FnMut(&mut S, &mut Ctx<G>) + 'static,
    ) -> Self {
        MenuItem {
            label: label.into(),
            help: None,
            action: Box::new(action),
            adjust: None,
            select: None,
            adjust_sound: true,
        }
    }

    /// A row that does nothing when activated: a heading or a readout.
    /// Activating it plays nothing, not the menu's Select earcon; a later
    /// `.adjustable(...)` leaves that silent select alone.
    pub fn inert(label: impl Into<Label<S, G>>) -> Self {
        MenuItem::new(label, |_: &mut S, _: &mut Ctx<G>| {}).select_sound(None)
    }

    pub fn help(mut self, help: impl Into<Label<S, G>>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Give the row a Left/Right adjuster.
    pub fn adjustable(mut self, f: impl FnMut(&mut S, &mut Ctx<G>, i32) + 'static) -> Self {
        self.adjust = Some(Box::new(f));
        self
    }

    /// Override the row's select sound: `Some(key)` plays `key` instead of
    /// the menu's default, `None` silences it.
    pub fn select_sound(mut self, key: Option<&'static str>) -> Self {
        self.select = Some(key);
        self
    }

    /// Left and Right play nothing for this row; it plays its own sound.
    pub fn silent_adjust(mut self) -> Self {
        self.adjust_sound = false;
        self
    }

    pub fn text(&self, state: &S, ctx: &Ctx<G>) -> String {
        self.label.text(state, ctx)
    }

    pub fn help_text(&self, state: &S, ctx: &Ctx<G>) -> Option<String> {
        self.help.as_ref().map(|h| h.text(state, ctx))
    }
}

/// The reusable half of a menu screen: the title, the rows, the cursor.
pub struct MenuCore<S, G: Game> {
    pub title: String,
    pub intro_help: String,
    pub items: Vec<MenuItem<S, G>>,
    pub index: usize,
    /// Bumped whenever the rows are replaced. `activate` and `adjust` lend
    /// the rows to a callback and compare this afterwards to tell a
    /// callback that installed its own rows from one that did not. Anything
    /// that assigns `items` directly must bump it; `set_items` does both.
    pub revision: u64,
}

pub const DEFAULT_INTRO_HELP: &str =
    "Up and Down move, Enter chooses, Escape goes back, F2 stops speech.";

impl<S, G: Game> MenuCore<S, G> {
    pub fn new(title: impl Into<String>) -> Self {
        MenuCore {
            title: title.into(),
            intro_help: DEFAULT_INTRO_HELP.to_string(),
            items: Vec::new(),
            index: 0,
            revision: 0,
        }
    }

    /// Replace the rows and mark them changed.
    pub fn set_items(&mut self, items: Vec<MenuItem<S, G>>) {
        self.items = items;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn with_intro_help(mut self, help: impl Into<String>) -> Self {
        self.intro_help = help.into();
        self
    }
}

impl<S, G: Game> Default for MenuCore<S, G> {
    fn default() -> Self {
        MenuCore::new("Menu")
    }
}

/// A vertically navigated, fully spoken menu. Implement `name`, the two
/// accessors and `build_items`; everything else has a default the screen
/// may override.
pub trait Menu: Sized + 'static {
    /// The game this menu belongs to. An associated type rather than a
    /// parameter, so a built-in screen that is not a menu can still be a
    /// `State` of any game without clashing with the blanket impl below.
    type Game: Game;

    fn name(&self) -> &'static str;
    fn menu(&self) -> &MenuCore<Self, Self::Game>;
    fn menu_mut(&mut self) -> &mut MenuCore<Self, Self::Game>;
    fn build_items(&mut self, ctx: &mut Ctx<Self::Game>) -> Vec<MenuItem<Self, Self::Game>>;

    // -- State hooks ----------------------------------------------------------

    fn enter(&mut self, ctx: &mut Ctx<Self::Game>) {
        menu_enter(self, ctx);
    }

    fn exit(&mut self, _ctx: &mut Ctx<Self::Game>) {}

    /// Revealed again: rebuild the rows (a setting may have changed) and
    /// re-announce, keeping the cursor.
    fn resume(&mut self, ctx: &mut Ctx<Self::Game>) {
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn handle_input(&mut self, ctx: &mut Ctx<Self::Game>, event: &InputEvent) {
        menu_input(self, ctx, event)
    }

    fn on_event(&mut self, _ctx: &mut Ctx<Self::Game>, _event: &<Self::Game as Game>::Event) {}

    fn update(&mut self, _ctx: &mut Ctx<Self::Game>, _dt: f64) {}

    fn lines(&self, ctx: &Ctx<Self::Game>) -> Vec<String> {
        let core = self.menu();
        let mut out = vec![core.title.clone()];
        for (i, item) in core.items.iter().enumerate() {
            let marker = if i == core.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out
    }

    /// F1: the focused row's help, or the menu's own help when the row has
    /// none.
    fn help(&self, ctx: &Ctx<Self::Game>) -> String {
        self.current_help(ctx)
            .unwrap_or_else(|| self.menu().intro_help.clone())
    }

    /// What Escape does. Closing the screen is the default, so a menu at
    /// the root of the stack must override this: popping the only screen
    /// ends the run.
    fn on_escape(&mut self, ctx: &mut Ctx<Self::Game>) {
        ctx.play_ui(UiSound::Back);
        ctx.pop()
    }

    // -- menu behaviour --------------------------------------------------------

    /// Rebuild the rows from `build_items`, keeping the cursor when
    /// `keep_index` is set.
    fn refresh(&mut self, ctx: &mut Ctx<Self::Game>, keep_index: bool) {
        let items = self.build_items(ctx);
        let core = self.menu_mut();
        core.set_items(items);
        if !keep_index || core.index >= core.items.len() {
            core.index = 0;
        }
    }

    /// The title as a sentence, then the row under the cursor queued
    /// behind it: `"Title."`, `"Row. 1 of 4."`.
    fn announce(&mut self, ctx: &mut Ctx<Self::Game>) {
        let title = end_sentence(&self.menu().title);
        ctx.say(title);
        let current = self.current_text(ctx);
        ctx.say_queued(current);
    }

    /// The row under the cursor as a sentence, then where it is when the
    /// player has menu positions on: `"Beta. 2 of 3."`.
    fn current_text(&self, ctx: &Ctx<Self::Game>) -> String {
        let core = self.menu();
        let Some(item) = core.items.get(core.index) else {
            return "No options available.".to_string();
        };
        let label = end_sentence(&item.text(self, ctx));
        if ctx.settings.menus.announce_position {
            format!("{label} {} of {}.", core.index + 1, core.items.len())
        } else {
            label
        }
    }

    fn current_help(&self, ctx: &Ctx<Self::Game>) -> Option<String> {
        let core = self.menu();
        core.items.get(core.index)?.help_text(self, ctx)
    }

    fn speak_current(&mut self, ctx: &mut Ctx<Self::Game>) {
        let text = self.current_text(ctx);
        ctx.say(text);
    }

    /// How `adjust` reads a changed row back. A screen that must read some
    /// rows through another voice (a speech-rate preview) overrides this.
    fn speak_adjusted(&mut self, ctx: &mut Ctx<Self::Game>) {
        self.speak_current(ctx)
    }

    /// Move the cursor by `delta`, wrapping at both ends.
    fn move_by(&mut self, ctx: &mut Ctx<Self::Game>, delta: i32) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        let here = self.menu().index as i32;
        self.menu_mut().index = (here + delta).rem_euclid(n as i32) as usize;
        ctx.play_ui(UiSound::Move);
        self.speak_current(ctx);
    }

    fn jump(&mut self, ctx: &mut Ctx<Self::Game>, index: usize) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        self.menu_mut().index = index.min(n - 1);
        ctx.play_ui(UiSound::Move);
        self.speak_current(ctx);
    }

    /// The next row after the cursor whose label starts with `c`, wrapping.
    /// Silent when nothing matches.
    fn letter_jump(&mut self, ctx: &mut Ctx<Self::Game>, c: char) {
        let n = self.menu().items.len();
        let start = self.menu().index;
        let wanted = c.to_ascii_lowercase();
        for offset in 1..=n {
            let i = (start + offset) % n;
            let text = self.menu().items[i].text(self, ctx);
            if text
                .chars()
                .next()
                .is_some_and(|f| f.to_ascii_lowercase() == wanted)
            {
                self.menu_mut().index = i;
                ctx.play_ui(UiSound::Move);
                self.speak_current(ctx);
                return;
            }
        }
    }

    fn activate(&mut self, ctx: &mut Ctx<Self::Game>) {
        let i = self.menu().index;
        if i >= self.menu().items.len() {
            return;
        }
        // The action needs `&mut self`, so the rows are lent to it and handed
        // back afterwards -- unless the action installed rows of its own.
        let revision = self.menu().revision;
        let mut items = std::mem::take(&mut self.menu_mut().items);
        match items[i].select {
            Some(Some(key)) => {
                ctx.audio.play(key, Bus::UI);
            }
            Some(None) => {}
            None => {
                ctx.play_ui(UiSound::Select);
            }
        }
        (items[i].action)(self, ctx);
        if self.menu().revision == revision {
            self.menu_mut().items = items;
        }
    }

    /// Left and Right: change the row in place and read it back. A row with
    /// no adjuster does and says nothing.
    fn adjust(&mut self, ctx: &mut Ctx<Self::Game>, delta: i32) {
        let i = self.menu().index;
        let Some(item) = self.menu().items.get(i) else {
            return;
        };
        if item.adjust.is_none() {
            return;
        }
        if item.adjust_sound {
            ctx.play_ui(UiSound::Move);
        }
        let revision = self.menu().revision;
        let mut items = std::mem::take(&mut self.menu_mut().items);
        if let Some(adjust) = items[i].adjust.as_mut() {
            adjust(self, ctx, delta);
        }
        if self.menu().revision == revision {
            self.menu_mut().items = items;
        }
        self.speak_adjusted(ctx);
    }
}

/// `Menu::enter`'s default body: `UiSound::Open`, then the rows are built
/// and the title and first row are announced. A screen that must act before
/// or after arriving (the pause screen holding the game's sound) overrides
/// `enter` and calls this itself.
pub fn menu_enter<S: Menu>(state: &mut S, ctx: &mut Ctx<S::Game>) {
    ctx.play_ui(UiSound::Open);
    state.refresh(ctx, false);
    state.announce(ctx);
}

/// The navigation every menu shares, from the keyboard or a controller. A
/// screen with extra keys overrides `handle_input` and calls this for
/// everything it does not claim.
pub fn menu_input<S: Menu>(state: &mut S, ctx: &mut Ctx<S::Game>, event: &InputEvent) {
    let Some(key) = event.pressed() else {
        return;
    };
    let text = event.key_down().and_then(|(_, _, text)| text);
    match key {
        Key::Down => state.move_by(ctx, 1),
        Key::Up => state.move_by(ctx, -1),
        Key::Home => state.jump(ctx, 0),
        Key::End => {
            let last = state.menu().items.len().saturating_sub(1);
            state.jump(ctx, last);
        }
        Key::Return | Key::KpEnter | Key::Space => state.activate(ctx),
        Key::Left => state.adjust(ctx, -1),
        Key::Right => state.adjust(ctx, 1),
        Key::Escape => state.on_escape(ctx),
        other => {
            let typed = text
                .filter(|c| c.is_alphanumeric())
                .or_else(|| letter_of(other));
            if let Some(c) = typed {
                state.letter_jump(ctx, c);
            }
        }
    }
}

/// The character a letter or digit key stands for, so first-letter jumps
/// work when the shell reported no typed text.
fn letter_of(key: Key) -> Option<char> {
    ('a'..='z')
        .chain('0'..='9')
        .find(|&c| Key::from_char(c) == key)
}

impl<M: Menu> State<M::Game> for M {
    fn name(&self) -> &'static str {
        Menu::name(self)
    }
    fn enter(&mut self, ctx: &mut Ctx<M::Game>) {
        Menu::enter(self, ctx)
    }
    fn exit(&mut self, ctx: &mut Ctx<M::Game>) {
        Menu::exit(self, ctx)
    }
    fn resume(&mut self, ctx: &mut Ctx<M::Game>) {
        Menu::resume(self, ctx)
    }
    fn handle_input(&mut self, ctx: &mut Ctx<M::Game>, event: &InputEvent) {
        Menu::handle_input(self, ctx, event)
    }
    fn on_event(&mut self, ctx: &mut Ctx<M::Game>, event: &<M::Game as Game>::Event) {
        Menu::on_event(self, ctx, event)
    }
    fn update(&mut self, ctx: &mut Ctx<M::Game>, dt: f64) {
        Menu::update(self, ctx, dt)
    }
    fn lines(&self, ctx: &Ctx<M::Game>) -> Vec<String> {
        Menu::lines(self, ctx)
    }
    fn help(&self, ctx: &Ctx<M::Game>) -> String {
        Menu::help(self, ctx)
    }
}
