//! Connecting the player's online account, from Settings: the game asks
//! the server for a short code and speaks it, the player enters it on the
//! site, and the screen closes once the server hands over the account.
//! With an account already connected, the screen offers to disconnect it.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::online::{spell_code, Connect};
use crate::sound::UiSound;
use crate::state::State;

pub struct OnlineAccountScreen {
    /// Whether the player arrived connected: then Enter disconnects.
    connected: bool,
    /// The connect state last spoken, so each change is spoken once.
    heard: Connect,
}

impl OnlineAccountScreen {
    pub fn new() -> Self {
        OnlineAccountScreen {
            connected: false,
            heard: Connect::Idle,
        }
    }
}

impl Default for OnlineAccountScreen {
    fn default() -> Self {
        Self::new()
    }
}

/// A site's address as it is read out: no `https://`.
fn spoken_url(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
}

fn code_line(site: &str, code: &str, url: &str) -> String {
    let place = if url.is_empty() {
        site
    } else {
        spoken_url(url)
    };
    format!(
        "Code {}. Go to {place} and enter it. Enter repeats the code.",
        spell_code(code)
    )
}

impl<G: Game> State<G> for OnlineAccountScreen {
    fn name(&self) -> &'static str {
        "online_account"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let site = ctx.online.site().to_string();
        if ctx.online.is_connected() {
            self.connected = true;
            let name = ctx
                .online
                .player_name()
                .unwrap_or("your account")
                .to_string();
            ctx.say(format!(
                "Connected to {site} as {name}. Enter disconnects this computer, Escape keeps it."
            ));
        } else {
            ctx.online.connect();
            self.heard = Connect::Asking;
            ctx.say(format!("Asking {site} for a code."));
        }
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        if !self.connected {
            ctx.online.cancel_connect();
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if self.connected {
            return;
        }
        let state = ctx.online.connect_state().clone();
        if state == self.heard {
            return;
        }
        self.heard = state.clone();
        let site = ctx.online.site().to_string();
        match state {
            Connect::Waiting { code, url } => ctx.say(code_line(&site, &code, &url)),
            Connect::Connected { name } => {
                ctx.play_ui(UiSound::Select);
                ctx.pop_with(format!("Connected to {site} as {name}"));
            }
            Connect::Failed(e) => {
                ctx.play_ui(UiSound::Error);
                let line = e.spoken(&site);
                if line.contains("Enter") {
                    ctx.say(line);
                } else {
                    ctx.say(format!("{line} Enter tries again, Escape goes back."));
                }
            }
            Connect::Idle | Connect::Asking => {}
        }
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(key) = event.pressed() else {
            return;
        };
        let site = ctx.online.site().to_string();
        match key {
            Key::Escape => {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
            }
            Key::Return if self.connected => {
                ctx.online.disconnect();
                ctx.play_ui(UiSound::Select);
                ctx.pop_with(format!("Disconnected from {site}"));
            }
            Key::Return => match ctx.online.connect_state().clone() {
                Connect::Waiting { code, url } => ctx.say(code_line(&site, &code, &url)),
                Connect::Failed(_) | Connect::Idle => {
                    ctx.online.connect();
                    self.heard = Connect::Asking;
                    ctx.say(format!("Asking {site} for a code."));
                }
                Connect::Asking => ctx.say(format!("Still asking {site} for a code.")),
                Connect::Connected { .. } => {}
            },
            _ => {}
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let site = ctx.online.site();
        match ctx.online.connect_state() {
            _ if self.connected => vec![format!(
                "Connected to {site} as {}.",
                ctx.online.player_name().unwrap_or("your account")
            )],
            Connect::Waiting { code, url } => {
                vec![format!("Code: {code}"), format!("Enter it at {url}")]
            }
            Connect::Asking | Connect::Idle => vec![format!("Asking {site} for a code.")],
            Connect::Connected { name } => vec![format!("Connected as {name}.")],
            Connect::Failed(e) => vec![e.spoken(site)],
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        let site = ctx.online.site();
        if self.connected {
            format!("Enter disconnects this computer from {site}. Escape goes back and stays connected.")
        } else {
            format!(
                "Sign in on {site}, open its page for connecting a game, and enter the code. \
                 Enter repeats the code, Escape stops connecting."
            )
        }
    }
}
