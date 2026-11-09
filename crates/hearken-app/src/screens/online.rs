//! Connecting the player's online account, from Settings: the game asks
//! the server for a short code and speaks it, the player enters it on the
//! site, and the screen closes once the server hands over the account.
//! With an account already connected, the screen offers to disconnect it.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::online::{spell_code, Connect};
use crate::phase::{Phase, Phases};
use crate::sound::UiSound;
use crate::state::State;

/// What the screen is doing. The server's side is `ctx.online`'s
/// [`Connect`]; this is the player's: what the keys do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    /// Arrived connected to an account: Enter disconnects.
    Connected,
    /// Guests are allowed and nothing is asked yet: Up and Down choose,
    /// Enter picks.
    Choosing,
    /// A request is with the server: a code, a guest name or a new name.
    Asking,
    /// The server gave a code and waits for the player to enter it on
    /// the site: Enter repeats the code.
    Code,
    /// Asking for a code failed: Enter asks again.
    Failed,
}

impl Phase for Account {
    fn name(self) -> &'static str {
        match self {
            Account::Connected => "connected",
            Account::Choosing => "choosing",
            Account::Asking => "asking",
            Account::Code => "code",
            Account::Failed => "failed",
        }
    }
}

pub struct OnlineAccountScreen {
    phases: Phases<Account>,
    /// The choices on offer when guests are allowed, and the one heard.
    choices: Vec<Choice>,
    at: usize,
    /// The connect state last spoken, so each change is spoken once.
    heard: Connect,
}

/// What a player can do here when the server allows guests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Guest,
    NewName,
    Account,
    Disconnect,
}

impl Choice {
    fn label(self, site: &str) -> String {
        match self {
            Choice::Guest => "Play as a guest".into(),
            Choice::NewName => "New guest name".into(),
            Choice::Account => format!("Connect a {site} account"),
            Choice::Disconnect => "Stop playing online on this computer".into(),
        }
    }
}

impl OnlineAccountScreen {
    pub fn new() -> Self {
        OnlineAccountScreen {
            // Nothing is asked before `enter` says where the player stands.
            phases: Phases::new(Account::Choosing),
            choices: Vec::new(),
            at: 0,
            heard: Connect::Idle,
        }
    }

    /// Sends `ctx.online`'s request off and waits on it, speaking `line`.
    fn ask<G: Game>(&mut self, ctx: &mut Ctx<G>, line: String) {
        self.phases.set(ctx, Account::Asking);
        self.heard = Connect::Asking;
        ctx.say(line);
    }

    fn choice_line(&self, site: &str) -> String {
        let n = self.choices.len();
        format!(
            "{}, {} of {n}",
            self.choices[self.at].label(site),
            self.at + 1
        )
    }
}

impl Default for OnlineAccountScreen {
    fn default() -> Self {
        Self::new()
    }
}

/// A site's address as it is read out: no `https://`, and the slashes and
/// port colon in words, which screen readers otherwise say or skip as they
/// please.
fn spoken_url(url: &str) -> String {
    let bare = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let mut parts = bare.split('/');
    let host = parts.next().unwrap_or("");
    let host = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{name} port {port}")
        }
        _ => host.to_string(),
    };
    std::iter::once(host)
        .chain(parts.map(str::to_string))
        .collect::<Vec<_>>()
        .join(" slash ")
}

/// What a failed connection says, with what the keys do now.
fn failed_line(site: &str, e: &crate::online::OnlineError) -> String {
    match e {
        crate::online::OnlineError::RateLimited => {
            format!("{site} is busy. Wait a minute, then Enter asks again. Escape goes back.")
        }
        crate::online::OnlineError::CodeExpired => {
            "The code expired. Enter asks for a new one, Escape goes back.".into()
        }
        e => format!("{} Enter tries again, Escape goes back.", e.spoken(site)),
    }
}

fn code_line(site: &str, code: &str, url: &str, opened: bool) -> String {
    let place = if url.is_empty() {
        site.to_string()
    } else {
        spoken_url(url)
    };
    if opened {
        format!(
            "Code {}. {place} is open in your browser. Enter repeats the code.",
            spell_code(code)
        )
    } else {
        format!(
            "Code {}. Go to {place} and enter it. Enter repeats the code.",
            spell_code(code)
        )
    }
}

impl<G: Game> State<G> for OnlineAccountScreen {
    fn name(&self) -> &'static str {
        "online_account"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let site = ctx.online.site().to_string();
        if ctx.online.is_guest() {
            self.choices = vec![Choice::NewName, Choice::Account, Choice::Disconnect];
            let name = ctx.online.player_name().unwrap_or("a guest").to_string();
            ctx.say(format!(
                "Playing as guest {name}. {}",
                self.choice_line(&site)
            ));
        } else if ctx.online.is_connected() {
            self.phases.set(ctx, Account::Connected);
            let name = ctx
                .online
                .player_name()
                .unwrap_or("your account")
                .to_string();
            ctx.say(format!(
                "Connected to {site} as {name}. Enter disconnects this computer, Escape keeps it."
            ));
        } else if ctx.online.offers_guests() {
            self.choices = vec![Choice::Guest, Choice::Account];
            ctx.say(format!("Not playing online. {}", self.choice_line(&site)));
        } else {
            ctx.online.connect();
            self.ask(ctx, format!("Asking {site} for a code."));
        }
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        if !self.phases.is(Account::Connected) {
            ctx.online.cancel_connect();
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if matches!(self.phases.get(), Account::Connected | Account::Choosing) {
            return;
        }
        let state = ctx.online.connect_state().clone();
        if state == self.heard {
            return;
        }
        self.heard = state.clone();
        let site = ctx.online.site().to_string();
        match state {
            Connect::Waiting { code, url, opened } => {
                self.phases.set(ctx, Account::Code);
                ctx.say(code_line(&site, &code, &url, opened))
            }
            Connect::Connected { name } if ctx.online.is_guest() => {
                ctx.play_ui(UiSound::Select);
                ctx.pop_with(format!("Playing as guest {name}."));
            }
            Connect::Connected { name } => {
                ctx.play_ui(UiSound::Select);
                ctx.pop_with(format!("Connected to {site} as {name}."));
            }
            Connect::Renamed { name } => {
                self.phases.set(ctx, Account::Choosing);
                ctx.play_ui(UiSound::Select);
                ctx.say(format!("You are now {name}. Enter again for another."));
            }
            Connect::Failed(e) => {
                // With choices on offer the player picks again; asking
                // for a code, Enter asks again.
                let next = if self.choices.is_empty() {
                    Account::Failed
                } else {
                    Account::Choosing
                };
                self.phases.set(ctx, next);
                ctx.play_ui(UiSound::Error);
                ctx.say(failed_line(&site, &e));
            }
            Connect::Idle | Connect::Asking => {}
        }
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(key) = event.pressed() else {
            return;
        };
        let site = ctx.online.site().to_string();
        let choosing = self.phases.is(Account::Choosing) && !self.choices.is_empty();
        match key {
            Key::Escape => {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
            }
            Key::Up | Key::Down if choosing => {
                let n = self.choices.len();
                self.at = if key == Key::Up {
                    (self.at + n - 1) % n
                } else {
                    (self.at + 1) % n
                };
                ctx.play_ui(UiSound::Move);
                ctx.say(self.choice_line(&site));
            }
            Key::Return if event.is_repeat() && !self.choices.is_empty() => {}
            Key::Return if choosing => {
                match self.choices[self.at] {
                    Choice::Guest => {
                        ctx.online.play_as_guest();
                        self.ask(ctx, format!("Asking {site} for a guest name."));
                    }
                    Choice::NewName => {
                        ctx.online.new_guest_name();
                        self.ask(ctx, "Asking for a new name.".to_string());
                    }
                    Choice::Account => {
                        // Codes are what the rest of this screen speaks.
                        self.choices.clear();
                        ctx.online.connect();
                        self.ask(ctx, format!("Asking {site} for a code."));
                    }
                    Choice::Disconnect => {
                        ctx.online.disconnect();
                        ctx.play_ui(UiSound::Select);
                        ctx.pop_with("Stopped playing online on this computer.");
                    }
                }
            }
            Key::Return if !self.choices.is_empty() => {
                ctx.say(format!("Still asking {site}."));
            }
            // A held Enter repeats; only a fresh press disconnects.
            Key::Return if self.phases.is(Account::Connected) && event.is_repeat() => {}
            Key::Return if self.phases.is(Account::Connected) => {
                ctx.online.disconnect();
                ctx.play_ui(UiSound::Select);
                ctx.pop_with(format!("Disconnected from {site}."));
            }
            Key::Return => match ctx.online.connect_state().clone() {
                Connect::Waiting { code, url, opened } => {
                    ctx.say(code_line(&site, &code, &url, opened))
                }
                Connect::Failed(_) | Connect::Idle => {
                    ctx.online.connect();
                    self.ask(ctx, format!("Asking {site} for a code."));
                }
                Connect::Asking => ctx.say(format!("Still asking {site} for a code.")),
                Connect::Connected { .. } | Connect::Renamed { .. } => {}
            },
            _ => {}
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let site = ctx.online.site();
        if !self.choices.is_empty() && self.at < self.choices.len() {
            let mut lines: Vec<String> = Vec::new();
            if let Some(name) = ctx.online.player_name().filter(|_| ctx.online.is_guest()) {
                lines.push(format!("Playing as guest {name}."));
            }
            lines.extend(self.choices.iter().map(|c| c.label(site)));
            return lines;
        }
        match ctx.online.connect_state() {
            _ if self.phases.is(Account::Connected) => vec![format!(
                "Connected to {site} as {}.",
                ctx.online.player_name().unwrap_or("your account")
            )],
            Connect::Waiting { code, url, .. } => {
                let place = if url.is_empty() { site } else { url.as_str() };
                vec![format!("Code: {code}"), format!("Enter it at {place}")]
            }
            Connect::Asking | Connect::Idle => vec![format!("Asking {site} for a code.")],
            Connect::Connected { name } | Connect::Renamed { name } => {
                vec![format!("Connected as {name}.")]
            }
            Connect::Failed(e) => vec![failed_line(site, e)],
        }
    }

    fn phase(&self) -> Option<&'static str> {
        Some(self.phases.name())
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        let site = ctx.online.site();
        if self.phases.is(Account::Connected) {
            format!("Enter disconnects this computer from {site}. Escape goes back and stays connected.")
        } else if !self.choices.is_empty() {
            format!(
                "A guest plays with no account, under a name {site} picks. \
                 Connect an account to keep your scores on any computer. \
                 Up and Down choose, Enter picks, Escape goes back."
            )
        } else {
            format!(
                "Sign in on {site}, open its page for connecting a game, and enter the code. \
                 Enter repeats the code, Escape stops connecting."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::spoken_url;

    #[test]
    fn an_address_is_read_in_words() {
        assert_eq!(
            spoken_url("https://example.test/activate"),
            "example.test slash activate"
        );
        assert_eq!(
            spoken_url("http://127.0.0.1:8787/activate/"),
            "127.0.0.1 port 8787 slash activate"
        );
        assert_eq!(spoken_url("https://example.test"), "example.test");
    }
}
