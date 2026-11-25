//! The chat list, and the list of blocked and muted players.
//!
//! The chat list is one menu for a table and for the game's channel:
//! "Say something", then the lines newest first, then (at a table)
//! "Players". Enter on a line opens its speaker: mute, block, or report
//! that line. New lines join the top without moving the player's place.
//!
//! A channel's list joins the channel while it is open and leaves it on
//! the way out. Where that stands is the channel link, an explicit state
//! machine logged as `[chat] link: joining -> in`:
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | Out | Not in the channel | The list opens: Joining |
//! | Joining | The server was asked for the channel | It answers: In. It refuses: Out, with the reason |
//! | In | Watched, in focus, polled | The list closes: Out. The server drops the seat (away too long): Joining |

use hearken_rooms::{Answer, Mute, MuteMode, Notice, RoomId, Ticket};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::lobby::Channel;
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::screens::confirm::ConfirmScreen;
use crate::screens::players::{PlayerScreen, PlayersScreen};
use crate::sound::UiSound;

/// What the chat list shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Table(RoomId),
    Channel(Channel),
}

/// Where a channel's list stands with the server.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Link {
    /// A table: nothing to join.
    Table,
    Out,
    Joining(Ticket),
    In(RoomId),
}

impl Link {
    fn name(&self) -> &'static str {
        match self {
            Link::Table => "table",
            Link::Out => "out",
            Link::Joining(_) => "joining",
            Link::In(_) => "in",
        }
    }
}

/// The chat of a table or of the game's channel.
pub struct ChatListScreen<G: Game> {
    target: Target,
    link: Link,
    menu: MenuCore<Self, G>,
    /// The room in focus before the list opened, put back on the way out.
    focus_before: Option<RoomId>,
    /// How many lines the rows were built from, so new ones keep the place.
    shown: usize,
}

impl<G: Game> ChatListScreen<G> {
    /// The chat of the table `room`.
    pub fn table(room: RoomId) -> Self {
        ChatListScreen {
            target: Target::Table(room),
            link: Link::Table,
            menu: MenuCore::new("Chat"),
            focus_before: None,
            shown: 0,
        }
    }

    /// The game's channel: joined while the list is open.
    pub fn channel(channel: Channel) -> Self {
        ChatListScreen {
            menu: MenuCore::new(channel.label),
            target: Target::Channel(channel),
            link: Link::Out,
            focus_before: None,
            shown: 0,
        }
    }

    fn room(&self) -> Option<RoomId> {
        match (&self.target, &self.link) {
            (Target::Table(id), _) => Some(id.clone()),
            (Target::Channel(_), Link::In(id)) => Some(id.clone()),
            _ => None,
        }
    }

    fn set_link(&mut self, ctx: &mut Ctx<G>, to: Link) {
        let line = format!("[chat] link: {} -> {}", self.link.name(), to.name());
        log::info!("{line}");
        ctx.note_change(line);
        self.link = to;
    }

    fn join(&mut self, ctx: &mut Ctx<G>) {
        let Target::Channel(channel) = self.target.clone() else {
            return;
        };
        match ctx.rooms.channel(channel.kind, channel.name) {
            Ok(ticket) => self.set_link(ctx, Link::Joining(ticket)),
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
                self.set_link(ctx, Link::Out);
            }
        }
    }

    /// Marks every line here as seen, for the Play online row and notices.
    fn seen(&self, ctx: &mut Ctx<G>) {
        if !matches!(self.target, Target::Channel(_)) {
            return;
        }
        if let Some(id) = self.room() {
            let last = ctx.chat_lines(&id).last().map_or(0, |(e, _)| e.seq);
            ctx.lobby.channel_seen = ctx.lobby.channel_seen.max(last);
        }
        if let Some(seq) = ctx
            .rooms
            .lobby()
            .and_then(|l| match &self.target {
                Target::Channel(c) => l.channel(c.kind, c.name),
                Target::Table(_) => None,
            })
            .and_then(|c| c.last.as_ref())
            .map(|l| l.seq)
        {
            ctx.lobby.channel_seen = ctx.lobby.channel_seen.max(seq);
        }
    }
}

impl<G: Game> Menu for ChatListScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "chat"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.focus_before = ctx.rooms.focus().cloned();
        if let Target::Table(id) = &self.target {
            ctx.rooms.set_focus(Some(id.clone()));
        } else {
            self.join(ctx);
        }
        self.seen(ctx);
        crate::menu::menu_enter(self, ctx);
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        if let (Target::Channel(_), Link::In(id)) = (&self.target, &self.link) {
            let id = id.clone();
            ctx.rooms.watch(&id, false);
            let _ = ctx.rooms.leave(&id);
            ctx.rooms.forget(&id);
            self.set_link(ctx, Link::Out);
        }
        ctx.rooms.set_focus(self.focus_before.take());
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let mut items = Vec::new();
        let Some(id) = self.room() else {
            self.shown = 0;
            return vec![MenuItem::inert(format!("Asking {}.", ctx.online.site()))];
        };
        items.push(
            MenuItem::new("Say something", |_: &mut Self, ctx: &mut Ctx<G>| {
                ctx.open_chat()
            })
            .help("Type a line, or choose a ready line with Up and Down."),
        );
        let lines = ctx.chat_lines(&id);
        self.shown = lines.len();
        let you = ctx.rooms.room(&id).and_then(|r| r.info.you);
        for (event, line) in lines.into_iter().rev() {
            let mine = event.seat.is_some() && event.seat == you;
            let speaker = event.handle.clone().filter(|_| !mine);
            let room = id.clone();
            let name = event.name.clone().unwrap_or_default();
            let seq = event.seq;
            let mut item =
                MenuItem::new(line, move |_: &mut Self, ctx: &mut Ctx<G>| match &speaker {
                    Some(handle) => ctx.push(PlayerScreen::<G>::for_line(
                        room.clone(),
                        &name,
                        handle,
                        seq,
                    )),
                    None => {
                        ctx.play_ui(UiSound::Error);
                    }
                });
            if !mine {
                item = item.help("Enter mutes, blocks or reports this player.");
            }
            items.push(item);
        }
        if self.shown == 0 {
            items.push(MenuItem::inert("Nothing said yet."));
        }
        if matches!(self.target, Target::Table(_)) {
            let room = id.clone();
            items.push(
                MenuItem::new("Players", move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(PlayersScreen::<G>::new(room.clone()));
                })
                .help("Mute, block or report a player at this table."),
            );
        }
        items
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if let Link::Joining(ticket) = self.link.clone() {
            if let Some(answer) = ctx.rooms.result(ticket) {
                match answer {
                    Ok(Answer::Room(id)) => {
                        ctx.rooms.watch(&id, true);
                        ctx.rooms.set_focus(Some(id.clone()));
                        self.set_link(ctx, Link::In(id));
                        self.refresh(ctx, false);
                        self.seen(ctx);
                        self.speak_current(ctx);
                    }
                    Ok(_) => self.set_link(ctx, Link::Out),
                    Err(e) => {
                        ctx.play_ui(UiSound::Error);
                        let site = ctx.online.site().to_string();
                        ctx.say(e.spoken(&site));
                        self.set_link(ctx, Link::Out);
                        self.refresh(ctx, true);
                    }
                }
            }
        }
        let Some(id) = self.room() else {
            return;
        };
        let mut fresh = false;
        let mut dropped = false;
        let notices = ctx.take_room_notices();
        for notice in &notices {
            match notice {
                Notice::Events(room, _) if *room == id => fresh = true,
                Notice::Dropped(room, _) if *room == id => dropped = true,
                _ => {}
            }
        }
        if dropped && matches!(self.target, Target::Channel(_)) {
            // The seat went while away: ask for it again.
            self.join(ctx);
            return;
        }
        if fresh {
            let before = self.shown;
            let at = self.menu.index;
            self.refresh(ctx, true);
            let added = self.shown.saturating_sub(before);
            // New lines join the top: the row under the cursor stays put.
            if at > 0 && added > 0 {
                self.menu.index = (at + added).min(self.menu.items.len().saturating_sub(1));
            }
            self.seen(ctx);
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            "Say something sends a line. The lines are newest first; Enter on one mutes, blocks \
             or reports the player."
                .to_string()
        })
    }
}

/// What the blocked and muted list waits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Loading(Ticket),
    Setting(Ticket),
}

/// Settings, Online: the players blocked or muted, and the way back.
pub struct MutesScreen<G: Game> {
    menu: MenuCore<Self, G>,
    step: Step,
    mutes: Option<Vec<Mute>>,
    /// The player being set free, for the line once it is done.
    freeing: Option<(String, MuteMode)>,
}

impl<G: Game> MutesScreen<G> {
    pub fn new() -> Self {
        MutesScreen {
            menu: MenuCore::new("Blocked and muted players"),
            step: Step::Idle,
            mutes: None,
            freeing: None,
        }
    }

    fn load(&mut self, ctx: &mut Ctx<G>) {
        match ctx.rooms.mutes() {
            Ok(ticket) => self.step = Step::Loading(ticket),
            Err(e) => {
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
    }
}

impl<G: Game> Default for MutesScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu for MutesScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "mutes"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.load(ctx);
        crate::menu::menu_enter(self, ctx);
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let Some(mutes) = self.mutes.clone() else {
            return vec![MenuItem::inert(format!("Asking {}.", ctx.online.site()))];
        };
        if mutes.is_empty() {
            return vec![MenuItem::inert("Nobody blocked or muted.")];
        }
        let mut sorted = mutes;
        // Blocks first: they hide the most.
        sorted.sort_by_key(|m| (m.mode != MuteMode::Block, m.name.clone()));
        sorted
            .into_iter()
            .map(|m| {
                let name = hearken_rooms::clean_text(m.name.as_deref().unwrap_or("A player"), 64);
                let (label, ask) = match m.mode {
                    MuteMode::Block => (format!("{name}, blocked"), format!("Unblock {name}?")),
                    _ => (format!("{name}, muted"), format!("Unmute {name}?")),
                };
                let handle = m.handle.clone();
                let mode = m.mode;
                MenuItem::new(label, move |s: &mut Self, ctx: &mut Ctx<G>| {
                    if s.step != Step::Idle {
                        ctx.say("Still asking.");
                        return;
                    }
                    s.freeing = Some((name.clone(), mode));
                    let handle = handle.clone();
                    ctx.push(ConfirmScreen::<G>::new(ask.clone(), move |ctx| {
                        ctx.pop_with(format!("free {handle}"));
                    }));
                })
            })
            .collect()
    }

    fn resume_with(&mut self, ctx: &mut Ctx<G>, result: &str) {
        if let Some(handle) = result.strip_prefix("free ") {
            match ctx.rooms.set_mute(handle, MuteMode::None) {
                Ok(ticket) => self.step = Step::Setting(ticket),
                Err(e) => {
                    ctx.play_ui(UiSound::Error);
                    let site = ctx.online.site().to_string();
                    ctx.say(e.spoken(&site));
                }
            }
            return;
        }
        self.freeing = None;
        self.announce(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let ticket = match self.step {
            Step::Idle => return,
            Step::Loading(t) | Step::Setting(t) => t,
        };
        let Some(answer) = ctx.rooms.result(ticket) else {
            return;
        };
        let step = self.step;
        self.step = Step::Idle;
        match (step, answer) {
            (Step::Loading(_), Ok(Answer::Mutes(mutes))) => {
                let first = self.mutes.is_none();
                self.mutes = Some(mutes);
                self.refresh(ctx, true);
                if first {
                    ctx.queued_speech(|ctx| self.speak_current(ctx));
                }
            }
            (Step::Setting(_), Ok(_)) => {
                ctx.play_ui(UiSound::Select);
                if let Some((name, mode)) = self.freeing.take() {
                    ctx.say(match mode {
                        MuteMode::Block => format!("{name} unblocked."),
                        _ => format!("{name} unmuted."),
                    });
                }
                self.load(ctx);
            }
            (_, Ok(_)) => {}
            (_, Err(e)) => {
                ctx.play_ui(UiSound::Error);
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "Players you blocked or muted. Enter lets one reach you again. Blocked players stay \
         hidden from you, and you from them, until you unblock them."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use hearken_rooms::transport::{reply, Scripted};
    use serde_json::json;

    use super::*;
    use crate::input::{InputEvent, Key};
    use crate::online::OnlineConfig;
    use crate::state::State;
    use crate::testing::Headless;
    use crate::{NoAction, NoCategory};

    struct Demo;

    impl Game for Demo {
        const TITLE: &'static str = "Demo";
        const DATA_DIR: &'static str = "hearken-chat-screens";
        const ENV_PREFIX: &'static str = "HEARKEN_CHAT_SCREENS";
        type Event = ();
        type Data = ();
        type Settings = ();
        type Action = NoAction;
        type Category = NoCategory;

        fn online() -> Option<OnlineConfig> {
            Some(
                OnlineConfig::new("https://example.test/api/games/demo", "example.test")
                    .with_rooms(),
            )
        }
    }

    struct Blank;

    impl State<Demo> for Blank {
        fn name(&self) -> &'static str {
            "blank"
        }
        fn help(&self, _ctx: &Ctx<Demo>) -> String {
            String::new()
        }
    }

    const ID: &str = "0123456789abcdef01234567";

    fn room() -> serde_json::Value {
        json!({
            "id": ID, "kind": "pebbles", "code": "M4K7T8", "phase": "playing", "open": false,
            "seats": { "min": 2, "max": 2 }, "host": 0,
            "members": [
                { "seat": 0, "name": "Ada", "guest": false, "here": true, "left": false },
                { "seat": 1, "name": "Bo", "guest": false, "here": true, "left": false,
                  "handle": "bbbbbbbbbbbbbbbb" },
            ],
            "you": 0, "seed": "7", "context": {}, "seq": 3,
            "snapshot": null, "result": null, "updated_at": 1000, "turn_started_at": null,
        })
    }

    fn start(screen: impl State<Demo> + 'static) -> (Headless<Demo>, Arc<Scripted>) {
        let mut h = Headless::<Demo>::new((), Blank);
        let script = Scripted::new();
        h.app.ctx.rooms.set_transport(script.clone());
        h.app.ctx.online.hold_account("tok", "Ada");
        h.frame(0.0);
        h.clear();
        h.app.ctx.push(screen);
        (h, script)
    }

    fn wait_for(h: &mut Headless<Demo>, text: &str) {
        for _ in 0..400 {
            if h.spoken().iter().any(|l| l.contains(text)) {
                return;
            }
            h.frame(0.016);
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never heard {text:?}; heard {:?}", h.spoken());
    }

    fn press(h: &mut Headless<Demo>, key: Key) {
        h.send(InputEvent::key(key));
    }

    #[test]
    fn the_blocked_and_muted_list_lists_blocks_first_and_unblocks_after_asking() {
        let (mut h, script) = start(MutesScreen::<Demo>::new());
        script.push_for(
            "rooms/mutes",
            reply(
                200,
                json!({ "ok": true, "mutes": [
                { "handle": "cccccccccccccccc", "mode": "mute", "name": "Cy" },
                { "handle": "bbbbbbbbbbbbbbbb", "mode": "block", "name": "Bo" },
            ] }),
            ),
        );
        h.frame(0.0);
        wait_for(&mut h, "Bo, blocked");
        let lines = h.app.top_lines();
        assert!(lines.iter().any(|l| l.contains("Cy, muted")), "{lines:?}");
        h.clear();

        press(&mut h, Key::Return);
        wait_for(&mut h, "Unblock Bo?");
        script.push_for(
            "rooms/mutes",
            reply(
                200,
                json!({ "ok": true, "mutes": [
            { "handle": "cccccccccccccccc", "mode": "mute", "name": "Cy" },
        ] }),
            ),
        );
        script.push_for(
            "rooms/mutes",
            reply(
                200,
                json!({ "ok": true, "mutes": [
            { "handle": "cccccccccccccccc", "mode": "mute", "name": "Cy" },
        ] }),
            ),
        );
        press(&mut h, Key::Return); // Yes
        wait_for(&mut h, "Bo unblocked.");
        assert_eq!(h.app.top_name(), "mutes");
        let freed = script.requests.lock().unwrap().iter().any(|r| {
            r.0 == "POST" && r.2 == json!({ "handle": "bbbbbbbbbbbbbbbb", "mode": "none" })
        });
        assert!(freed);
    }

    #[test]
    fn an_empty_list_says_so() {
        let (mut h, script) = start(MutesScreen::<Demo>::new());
        script.push_for(
            "rooms/mutes",
            reply(200, json!({ "ok": true, "mutes": [] })),
        );
        h.frame(0.0);
        wait_for(&mut h, "Nobody blocked or muted.");
    }

    #[test]
    fn a_tables_chat_lists_lines_newest_first_and_a_line_opens_its_speaker() {
        let mut h = Headless::<Demo>::new((), Blank);
        let script = Scripted::new();
        h.app.ctx.rooms.set_transport(script.clone());
        h.app.ctx.online.hold_account("tok", "Ada");
        h.frame(0.0);
        h.app
            .ctx
            .rooms
            .adopt(serde_json::from_value(room()).unwrap());
        h.app.ctx.rooms.watch(&RoomId(ID.into()), true);
        h.app.ctx.rooms.set_focus(Some(RoomId(ID.into())));
        script.push_for(
            "?after=0",
            reply(
                200,
                json!({ "ok": true, "room": room(),
            "events": [
                { "seq": 1, "type": "chat", "at": 1, "seat": 1, "name": "Bo",
                  "handle": "bbbbbbbbbbbbbbbb", "text": "hi there" },
                { "seq": 2, "type": "chat", "at": 2, "seat": 0, "name": "Ada", "text": "hello" },
                { "seq": 3, "type": "chat", "at": 3, "seat": 1, "name": "Bo",
                  "handle": "bbbbbbbbbbbbbbbb", "text": "good luck" },
            ], "read_to": 3, "more": false }),
            ),
        );
        for _ in 0..200 {
            h.frame(0.016);
            if !h.app.ctx.chat_lines(&RoomId(ID.into())).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        h.clear();
        h.app
            .ctx
            .push(ChatListScreen::<Demo>::table(RoomId(ID.into())));
        h.frame(0.0);
        let lines = h.app.top_lines();
        let at = |t: &str| {
            lines
                .iter()
                .position(|l| l.contains(t))
                .unwrap_or(usize::MAX)
        };
        assert_eq!(at("Say something"), 1, "{lines:?}");
        assert!(at("good luck") < at("hello"), "{lines:?}");
        assert!(at("hello") < at("hi there"), "{lines:?}");
        assert!(at("Players") < usize::MAX, "{lines:?}");

        // Down to Bo's newest line, Enter opens Bo.
        press(&mut h, Key::Down);
        press(&mut h, Key::Return);
        h.frame(0.0);
        assert_eq!(h.app.top_name(), "player");

        // Blocking Bo takes his lines already read out of the list.
        script.push_for(
            "rooms/mutes",
            reply(200, json!({ "ok": true, "mutes": [] })),
        );
        script.push_for(
            "rooms/mutes",
            reply(
                200,
                json!({ "ok": true, "handle": "bbbbbbbbbbbbbbbb", "mode": "block", "name": "Bo" }),
            ),
        );
        for _ in 0..20 {
            h.frame(0.016);
            std::thread::sleep(Duration::from_millis(5));
        }
        press(&mut h, Key::Down);
        press(&mut h, Key::Return);
        h.frame(0.0);
        assert_eq!(h.app.top_name(), "confirm");
        press(&mut h, Key::Return);
        wait_for(&mut h, "Bo blocked.");
        assert_eq!(h.app.top_name(), "chat");
        let lines = h.app.top_lines();
        assert!(lines.iter().any(|l| l.contains("hello")), "{lines:?}");
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("good luck") || l.contains("hi there")),
            "{lines:?}"
        );
    }

    /// A game whose chat waits during play, with ready lines and rules.
    struct Quiet;

    impl Game for Quiet {
        const TITLE: &'static str = "Quiet";
        const DATA_DIR: &'static str = "hearken-chat-quiet";
        const ENV_PREFIX: &'static str = "HEARKEN_CHAT_QUIET";
        type Event = ();
        type Data = ();
        type Settings = ();
        type Action = NoAction;
        type Category = NoCategory;

        fn online() -> Option<OnlineConfig> {
            Some(
                OnlineConfig::new("https://example.test/api/games/quiet", "example.test")
                    .with_rooms(),
            )
        }

        fn chat() -> Option<crate::chat::ChatConfig> {
            Some(crate::chat::ChatConfig {
                ready: |_| {
                    vec![crate::chat::ReadyLine {
                        id: "gl",
                        text: "Good luck.".into(),
                    }]
                },
                rules: Some("Chat rules: be kind."),
                ..Default::default()
            })
        }

        fn lobby() -> Option<crate::lobby::LobbyConfig<Self>> {
            let mut lobby = crate::lobby::LobbyConfig::new(|_| Vec::new());
            lobby.hold = |_, top| top == "playing";
            Some(lobby)
        }
    }

    struct QuietBlank;

    impl State<Quiet> for QuietBlank {
        fn name(&self) -> &'static str {
            "blank"
        }
        fn help(&self, _ctx: &Ctx<Quiet>) -> String {
            String::new()
        }
    }

    struct Playing;

    impl State<Quiet> for Playing {
        fn name(&self) -> &'static str {
            "playing"
        }
        fn help(&self, _ctx: &Ctx<Quiet>) -> String {
            String::new()
        }
    }

    fn quiet_wait(
        h: &mut Headless<Quiet>,
        what: &str,
        mut done: impl FnMut(&Headless<Quiet>) -> bool,
    ) {
        for _ in 0..400 {
            if done(h) {
                return;
            }
            h.frame(0.016);
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never saw {what}; heard {:?}", h.spoken());
    }

    #[test]
    fn chat_waits_during_play_then_ready_lines_and_the_rules_come_with_the_say_key() {
        let mut h = Headless::<Quiet>::new((), QuietBlank);
        let script = Scripted::new();
        h.app.ctx.rooms.set_transport(script.clone());
        h.app.ctx.online.hold_account("tok", "Ada");
        h.frame(0.0);
        let id = RoomId(ID.into());
        h.app
            .ctx
            .rooms
            .adopt(serde_json::from_value(room()).unwrap());
        h.app.ctx.rooms.watch(&id, true);
        h.app.ctx.rooms.set_focus(Some(id.clone()));
        h.app.ctx.push(Playing);
        h.frame(0.0);
        assert_eq!(h.app.ctx.quiet(), crate::chat::Quiet::Held);
        script.push_for("?after=0", reply(200, json!({ "ok": true, "room": room(),
            "events": [{ "seq": 1, "type": "chat", "at": 1, "seat": 1, "name": "Bo",
                "handle": "bbbbbbbbbbbbbbbb", "text": "nice move" }], "read_to": 1, "more": false })));
        quiet_wait(&mut h, "the line read", |h| {
            !h.app.ctx.chat_lines(&id).is_empty()
        });
        for _ in 0..5 {
            h.frame(0.016);
        }
        assert!(
            !h.spoken().iter().any(|l| l.contains("nice move")),
            "{:?}",
            h.spoken()
        );

        // The say key does nothing while play is on.
        h.send(InputEvent::key(Key::T));
        h.frame(0.0);
        assert_eq!(h.app.top_name(), "playing");

        // Play stops: the held line comes.
        h.app.ctx.pop_quiet();
        quiet_wait(&mut h, "the held line", |h| {
            h.spoken().iter().any(|l| l.contains("nice move"))
        });
        h.clear();

        // A ready line goes as its id, with no rules to agree to.
        h.send(InputEvent::key(Key::T));
        h.frame(0.0);
        assert_eq!(h.app.top_name(), "text_entry");
        press_quiet(&mut h, Key::Down);
        quiet_wait(&mut h, "the ready line", |h| {
            h.spoken().iter().any(|l| l.contains("Good luck."))
        });
        script.push_for("/events", reply(200, json!({ "ok": true, "seq": 2 })));
        press_quiet(&mut h, Key::Return);
        h.frame(0.0);
        let posted = |script: &Scripted| -> Vec<serde_json::Value> {
            script
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.0 == "POST" && r.1.ends_with("/events"))
                .map(|r| r.2.clone())
                .collect()
        };
        quiet_wait(&mut h, "the ready line sent", |_| {
            !posted(&script).is_empty()
        });
        assert_eq!(posted(&script)[0], json!({ "type": "chat", "quick": "gl" }));
        assert_ne!(h.app.top_name(), "text_entry");

        // The first typed line asks for the rules first.
        h.clear();
        h.send(InputEvent::key(Key::T));
        h.frame(0.0);
        for c in "hi".chars() {
            h.send(InputEvent::typed(c));
        }
        press_quiet(&mut h, Key::Return);
        quiet_wait(&mut h, "the rules", |h| {
            h.spoken()
                .iter()
                .any(|l| l.contains("Chat rules: be kind. Enter agrees."))
        });
        script.push_for("/events", reply(200, json!({ "ok": true, "seq": 3 })));
        press_quiet(&mut h, Key::Return); // Agree
        quiet_wait(&mut h, "the typed line sent", |_| {
            posted(&script).len() == 2
        });
        assert_eq!(posted(&script)[1]["text"], json!("hi"));
        assert!(h.app.ctx.chat_rules_agreed());
        assert_eq!(h.app.top_name(), "blank");
    }

    fn press_quiet(h: &mut Headless<Quiet>, key: Key) {
        h.send(InputEvent::key(key));
    }
}
