//! Accessible text entry: characters are echoed as they are typed, and a
//! review cursor lets the player check what they typed without retyping
//! it.
//!
//! Left and Right step the cursor one character and speak the character
//! moved over, the way a screen reader reads caret movement in a text
//! field. Home and End jump to either end. Typing and Backspace act at the
//! cursor, so a player who arrows back to fix a letter edits in place. Up
//! and Down read the whole text, as they do in a one-line field under a
//! screen reader. Enter confirms; Escape cancels.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `states/text_entry.rs`.
//! Freight Fate read the whole text on F2; F2 stops speech everywhere here.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::sound::UiSound;
use crate::state::State;
use crate::text::{end_sentence, spoken_char};

/// The longest text a field takes unless the game says otherwise.
pub const MAX_LEN: usize = 24;

/// How the field works, spoken on arrival and on F1.
pub const INSTRUCTIONS: &str = "Type, then press Enter. Left and Right review what you have \
     typed, Home and End jump to the start or end, and Up reads it all. Escape cancels.";

/// What the screen does with the text on Enter. The screen stays open
/// unless this closes it, so a game can refuse a text and say why.
pub type OnConfirm<G> = Box<dyn FnMut(&mut Ctx<G>, &str)>;

pub struct TextEntryScreen<G: Game> {
    heading: String,
    label: String,
    /// The typed text, as characters, so the cursor counts characters.
    chars: Vec<char>,
    /// A gap from 0 to `chars.len()`, the way a text field's caret works.
    cursor: usize,
    max_len: usize,
    /// How the field works, spoken on arrival and on F1.
    instructions: String,
    on_confirm: OnConfirm<G>,
}

impl<G: Game> TextEntryScreen<G> {
    pub fn new(
        heading: &str,
        label: &str,
        on_confirm: impl FnMut(&mut Ctx<G>, &str) + 'static,
    ) -> Self {
        TextEntryScreen {
            heading: heading.to_string(),
            label: label.to_string(),
            chars: Vec::new(),
            cursor: 0,
            max_len: MAX_LEN,
            instructions: INSTRUCTIONS.to_string(),
            on_confirm: Box::new(on_confirm),
        }
    }

    /// A field with no label of its own, for a heading that already says
    /// what to type ("Name your route."): arrival speaks the heading, any
    /// starting text, then the instructions. `new` with an empty label is
    /// the same.
    pub fn unlabelled(heading: &str, on_confirm: impl FnMut(&mut Ctx<G>, &str) + 'static) -> Self {
        TextEntryScreen::new(heading, "", on_confirm)
    }

    /// Speak `instructions` on arrival and on F1 in place of
    /// [`INSTRUCTIONS`], such as a shorter line for a player who has heard
    /// the full one, or one naming what the field accepts. Empty speaks
    /// none on arrival.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    /// Take up to `max_len` characters.
    pub fn max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }

    /// Start with `text` in the field, the cursor at its end.
    pub fn with_text(mut self, text: &str) -> Self {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn whole(&self) -> String {
        if self.chars.is_empty() {
            "Empty.".to_string()
        } else {
            self.text()
        }
    }

    fn insert(&mut self, ctx: &mut Ctx<G>, ch: char) {
        if self.chars.len() >= self.max_len {
            ctx.play_ui(UiSound::Error);
            ctx.say("The field is full.");
            return;
        }
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
        ctx.play_ui(UiSound::Tick);
        ctx.say(spoken_char(ch));
    }

    fn backspace(&mut self, ctx: &mut Ctx<G>) {
        if self.cursor == 0 {
            ctx.play_ui(UiSound::Error);
            return;
        }
        let removed = self.chars.remove(self.cursor - 1);
        self.cursor -= 1;
        let rest = self.whole();
        ctx.say(format!("Deleted {}. {rest}", spoken_char(removed)));
    }

    /// Step the cursor one character left or right and speak the character
    /// that sat between the old and new positions: the one moved over.
    fn move_cursor(&mut self, ctx: &mut Ctx<G>, delta: i64) {
        let next = self.cursor as i64 + delta;
        if next < 0 || next > self.chars.len() as i64 {
            ctx.play_ui(UiSound::Error);
            return;
        }
        let next = next as usize;
        let ch = self.chars[self.cursor.min(next)];
        self.cursor = next;
        ctx.say(spoken_char(ch));
    }

    fn jump_cursor(&mut self, ctx: &mut Ctx<G>, target: usize, label: &str) {
        if self.cursor == target {
            ctx.play_ui(UiSound::Error);
            return;
        }
        self.cursor = target;
        // A jump that moves the cursor means the field is not empty.
        let edge = if target == 0 {
            self.chars[0]
        } else {
            self.chars[self.chars.len() - 1]
        };
        ctx.say(format!("{label}. {}", spoken_char(edge)));
    }
}

/// Whether `ch` can be typed into a field: printable characters and the
/// space, not control characters or other whitespace.
pub fn is_printable(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    !(ch.is_control() || ch.is_whitespace())
}

impl<G: Game> State<G> for TextEntryScreen<G> {
    fn name(&self) -> &'static str {
        "text_entry"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let field = match (self.label.is_empty(), self.chars.is_empty()) {
            (true, true) => String::new(),
            (true, false) => self.text(),
            (false, true) => self.label.clone(),
            (false, false) => format!("{}: {}", self.label, self.text()),
        };
        let parts = [self.heading.as_str(), field.as_str(), &self.instructions];
        let line = parts
            .iter()
            .filter(|p| !p.trim().is_empty())
            .map(|p| end_sentence(p))
            .collect::<Vec<_>>()
            .join(" ");
        ctx.say(line);
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(key) = event.pressed() else {
            return;
        };
        match key {
            Key::Escape => {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
            }
            Key::Return | Key::KpEnter => {
                let text = self.text();
                (self.on_confirm)(ctx, &text);
            }
            Key::Backspace => self.backspace(ctx),
            Key::Left => self.move_cursor(ctx, -1),
            Key::Right => self.move_cursor(ctx, 1),
            Key::Home => self.jump_cursor(ctx, 0, "Start"),
            Key::End => {
                let end = self.chars.len();
                self.jump_cursor(ctx, end, "End");
            }
            Key::Up | Key::Down => {
                let whole = self.whole();
                ctx.say(whole);
            }
            _ => {
                if let Some((_, _, Some(ch))) = event.key_down() {
                    if is_printable(ch) {
                        self.insert(ctx, ch);
                    }
                }
            }
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        let before: String = self.chars[..self.cursor].iter().collect();
        let after: String = self.chars[self.cursor..].iter().collect();
        let field = if self.label.is_empty() {
            format!("{before}|{after}")
        } else {
            format!("{}: {before}|{after}", self.label)
        };
        vec![
            self.heading.clone(),
            String::new(),
            field,
            self.instructions.clone(),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        if self.instructions.trim().is_empty() {
            INSTRUCTIONS.to_string()
        } else {
            self.instructions.clone()
        }
    }
}
