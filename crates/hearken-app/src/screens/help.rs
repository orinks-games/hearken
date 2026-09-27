//! A game's help document read out a section at a time. The document is
//! compiled into the game, so help works before any data folder exists.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};

/// One `## ` heading and the prose under it, joined into one line.
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Split a Markdown document on `## ` headings. Anything before the first
/// one (the document's own title) is not a section. Other headings inside
/// a section keep their words and lose their `#` marks.
pub fn sections(doc: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in doc.lines() {
        match line.strip_prefix("## ") {
            Some(title) => out.push(Section {
                title: title.trim().to_string(),
                body: String::new(),
            }),
            None => {
                let Some(current) = out.last_mut() else {
                    continue;
                };
                let line = line.trim().trim_start_matches('#').trim_start();
                if line.is_empty() {
                    continue;
                }
                if !current.body.is_empty() {
                    current.body.push(' ');
                }
                current.body.push_str(line);
            }
        }
    }
    out
}

pub struct HelpScreen<G: Game> {
    menu: MenuCore<Self, G>,
    doc: &'static str,
}

impl<G: Game> HelpScreen<G> {
    pub fn new(doc: &'static str) -> Self {
        HelpScreen {
            menu: MenuCore::new("Help").with_intro_help(
                "Up and Down move between help topics, Enter reads one, Escape goes back.",
            ),
            doc,
        }
    }
}

impl<G: Game> Menu for HelpScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "help"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        sections(self.doc)
            .into_iter()
            .map(|section| {
                let body = section.body;
                MenuItem::new(section.title, move |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.say(body.clone())
                })
                .help("Press Enter to hear this topic.")
            })
            .collect()
    }
}
