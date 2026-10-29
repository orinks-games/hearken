//! Discord status: which presence the engine sends, the Settings switch, and
//! what is saved. A fake Discord client stands in for the pipe.

use std::sync::{Arc, Mutex};

use hearken_app::screens::SettingsScreen;
use hearken_app::{
    Ctx, Discord, DiscordSettings, Game, Headless, Key, NoAction, NoCategory, Presence, Settings,
    State,
};
use hearken_discord::{
    ActivityPayload, DiscordPresence, DiscordPresenceOptions, RpcClient, RpcFactory,
};

struct PresenceGame;

#[derive(Default)]
struct Data {
    fallback: Option<&'static str>,
}

impl Game for PresenceGame {
    const TITLE: &'static str = "Presence Game";
    const DATA_DIR: &'static str = "HearkenPresenceGame";
    const ENV_PREFIX: &'static str = "HEARKEN_PRESENCE_GAME";
    type Event = ();
    type Data = Data;
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn presence(ctx: &Ctx<Self>) -> Option<Presence> {
        ctx.game.fallback.map(Presence::new)
    }
}

/// A screen that reports `presence`, or nothing.
struct Screen {
    name: &'static str,
    presence: Option<&'static str>,
}

impl State<PresenceGame> for Screen {
    fn name(&self) -> &'static str {
        self.name
    }
    fn help(&self, _ctx: &Ctx<PresenceGame>) -> String {
        String::new()
    }
    fn presence(&self, _ctx: &Ctx<PresenceGame>) -> Option<Presence> {
        self.presence.map(|p| Presence::new(p).detail("Round 1"))
    }
}

fn screen(name: &'static str, presence: Option<&'static str>) -> Screen {
    Screen { name, presence }
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Vec<String>>>);

impl Fake {
    fn sent(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn factory(&self) -> RpcFactory {
        let me = self.clone();
        Arc::new(move |_: &str| Ok(Box::new(me.clone()) as Box<dyn RpcClient>))
    }
}

impl RpcClient for Fake {
    fn connect(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn update(&mut self, payload: &ActivityPayload) -> Result<(), String> {
        let line = match &payload.state {
            Some(state) => format!("{} / {state}", payload.details),
            None => payload.details.clone(),
        };
        self.0.lock().unwrap().push(line);
        Ok(())
    }
    fn clear(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().push("(cleared)".into());
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// A headless game whose Discord goes to `fake`, unthrottled and
/// synchronous.
fn with_discord(data: Data, first: Screen) -> (Headless<PresenceGame>, Fake) {
    let fake = Fake::default();
    let mut h = Headless::new(data, first);
    let service = DiscordPresence::new(DiscordPresenceOptions {
        min_interval_s: 0.0,
        rpc_factory: Some(fake.factory()),
        threaded: false,
        ..DiscordPresenceOptions::new("1234")
    });
    service.start();
    h.app.ctx.discord = Discord::with_service(service);
    (h, fake)
}

#[test]
fn the_top_screen_with_a_presence_is_shown() {
    let (mut h, fake) = with_discord(Data::default(), screen("lobby", Some("In the lobby")));
    h.frame(0.016);
    assert_eq!(fake.sent(), ["In the lobby / Round 1"]);

    h.app.ctx.push(screen("play", Some("Playing")));
    h.frame(0.016);
    assert_eq!(fake.sent().last().unwrap(), "Playing / Round 1");
}

#[test]
fn a_screen_with_none_leaves_it_to_the_one_below() {
    let (mut h, fake) = with_discord(Data::default(), screen("play", Some("Playing")));
    h.frame(0.016);
    h.app.ctx.push(screen("help", None));
    h.frame(0.016);
    h.frame(0.016);
    assert_eq!(
        fake.sent(),
        ["Playing / Round 1"],
        "help over play changed it"
    );
}

#[test]
fn the_games_fallback_covers_screens_with_none() {
    let data = Data {
        fallback: Some("Somewhere in the game"),
    };
    let (mut h, fake) = with_discord(data, screen("menu", None));
    h.frame(0.016);
    assert_eq!(fake.sent(), ["Somewhere in the game"]);
}

#[test]
fn nothing_is_asked_or_sent_while_dormant() {
    // The headless default: no Discord at all, so no Settings row either.
    let mut h = Headless::new(
        Data {
            fallback: Some("x"),
        },
        screen("root", Some("In the lobby")),
    );
    h.frame(0.016);
    assert!(!h.app.ctx.discord.is_offered());
    h.app.ctx.push(SettingsScreen::<PresenceGame>::new());
    h.frame(0.0);
    let lines = h.app.top_lines().join("\n");
    assert!(!lines.contains("Discord"), "{lines}");
}

#[test]
fn settings_switches_it_off_and_on() {
    let (mut h, fake) = with_discord(Data::default(), screen("lobby", Some("In the lobby")));
    h.frame(0.016);
    h.app.ctx.push(SettingsScreen::<PresenceGame>::new());
    h.frame(0.0);
    let lines = h.app.top_lines();
    let at = lines
        .iter()
        .position(|l| l.contains("Discord status: on"))
        .expect("a Discord status row");
    // Row 0 is the title; the cursor starts on the first row.
    for _ in 1..at {
        h.press(Key::Down);
    }
    h.clear();
    h.press(Key::Return);
    assert!(!h.app.ctx.settings.discord.enabled);
    assert!(!h.app.ctx.discord.is_on());
    assert!(
        h.spoken()[0].starts_with("Discord status: off"),
        "{:?}",
        h.spoken()
    );
    assert_eq!(fake.sent().last().unwrap(), "(cleared)");

    h.press(Key::Return);
    assert!(h.app.ctx.discord.is_on());
    assert_eq!(fake.sent().last().unwrap(), "In the lobby / Round 1");
}

#[test]
fn the_setting_is_saved_only_once_changed() {
    let mut settings = Settings::<()>::default();
    let json = serde_json::to_string(&settings).unwrap();
    assert!(!json.contains("discord"), "{json}");
    settings.discord = DiscordSettings { enabled: false };
    let json = serde_json::to_string(&settings).unwrap();
    let back: Settings<()> = serde_json::from_str(&json).unwrap();
    assert!(!back.discord.enabled);
    let old: Settings<()> = serde_json::from_str("{}").unwrap();
    assert!(old.discord.enabled, "on for a file from before the setting");
}
