//! What every screen is handed.

use hearken_audio::{Audio, Bus, Sound, Take, TakeState};
use hearken_l10n::Translator;
use hearken_speech::SpeechSink;
use std::sync::Arc;

use crate::bindings::KeyBindings;
use crate::controller::{ControllerManager, InputDevice};
use crate::game::Game;
use crate::held_keys::HeldKeys;
use crate::input::InputEvent;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::sound::{SoundConfig, UiSound, UiSounds, BUS_LABELS};
use crate::speech_delivery::Delivery;
use crate::state::{Reveal, StackOp, State};
use hearken_seal::Vault;

/// The `log` target every spoken line is written under, so a session log
/// reads as a transcript of the run.
pub const TRANSCRIPT_TARGET: &str = "hearken.transcript";

/// How many spoken lines `Ctx::transcript` keeps.
pub const TRANSCRIPT_CAP: usize = 2000;

/// The `log` target every screen-stack change is written under: a push, a
/// pop, what a pop revealed and how.
pub const SCREENS_TARGET: &str = "hearken.screens";

/// The `log` target every phase change (`Phases::set`) is written under.
pub const PHASE_TARGET: &str = "hearken.phase";

/// How many state changes `Ctx::changes` keeps.
pub const CHANGES_CAP: usize = 500;

pub struct Ctx<G: Game> {
    pub speech: Box<dyn SpeechSink>,
    /// Translates finished English lines before they are spoken.
    pub translator: Arc<Translator>,
    pub settings: Settings<G::Settings>,
    pub paths: Paths,
    /// The game's data folder, read and written through the seal: the
    /// place for records, saves and progress a player should not be able
    /// to edit. Sealed in a release built with `seal = true`, plain in
    /// every other build (see [`hearken_seal`]).
    pub vault: Vault,
    pub game: G::Data,
    /// Verified runs sent to the game's server, and the player's account
    /// there (see [`crate::online`]).
    #[cfg(feature = "online")]
    pub online: crate::online::Online,
    /// Play between players: rooms, seats and server-ordered turns, on the
    /// same server and token as `online` (see `hearken_rooms`). Polled by
    /// the app; what changed waits in `take_room_notices` for the screen.
    #[cfg(feature = "online")]
    pub rooms: hearken_rooms::Rooms,
    #[cfg(feature = "online")]
    room_notices: Vec<hearken_rooms::Notice>,
    /// The game's updates (see [`crate::updates`]): what the check at start
    /// found, and the way to check and install.
    #[cfg(feature = "update")]
    pub updates: crate::updates::Updates,
    /// The game's Discord status (see [`crate::discord`]). Dormant unless
    /// the game offers it and `run` started it.
    pub discord: crate::discord::Discord,
    /// Which keys are down now, for a screen that polls instead of reacting
    /// to presses.
    pub held: HeldKeys,
    /// The bound game controller. Headless contexts get one that never
    /// reaches a real subsystem; `run` swaps in SDL's.
    pub controller: ControllerManager,
    /// The game's controls, with the player's changes. Change them through
    /// `KeyBindings::set_chord` and friends, then `save_bindings`.
    pub bindings: KeyBindings<G::Action>,
    pub running: bool,
    /// Every line spoken this run, newest last. Event lines start `[event] `.
    pub transcript: Vec<String>,
    /// Lines spoken this run, counting those `transcript` has let go.
    pub said: u64,
    /// Lines for the player once the first screen has announced itself.
    pub notices: Vec<String>,
    /// Every state change this run, newest last, as the logs write them:
    /// `[screen] push pause over play`, `[phase] play: ready -> ending`.
    /// Nothing here is spoken; it is for tests, recordings and the agent
    /// server's inspector.
    pub changes: Vec<String>,
    /// State changes this run, counting those `changes` has let go.
    pub changed: u64,
    /// The screen on top of the stack, by `State::name`, as the app last
    /// set it; `"none"` before the first screen.
    pub(crate) screen: &'static str,
    /// The screen whose take is recording (`begin_take`), if any.
    pub(crate) take_owner: Option<&'static str>,
    pub frame: u64,
    /// Seconds since the run started, the sum of every frame's `dt`.
    pub clock: f64,
    /// The pool of positioned sounds, buses and live voices.
    pub audio: Audio,
    /// Which sound plays for each UI moment; the game's own copy of
    /// `sound().ui`, so a screen can point an earcon at nothing or a
    /// different sound at runtime.
    pub ui_sounds: UiSounds,
    /// The game's soundtrack and what the player has unlocked of it.
    pub soundtrack: crate::soundtrack::Soundtrack,
    sound: SoundConfig,
    pub(crate) delivery: Delivery<G::Category>,
    pending: Vec<StackOp<G>>,
    /// The game's own voices, if it installed any (`set_commentary`).
    pub(crate) commentary: Option<Box<dyn crate::commentary::Commentary<G>>>,
    /// Set while `queued_speech` runs: `say` queues instead of interrupting.
    hold_interrupt: bool,
}

impl<G: Game> Ctx<G> {
    pub fn new(
        speech: Box<dyn SpeechSink>,
        settings: Settings<G::Settings>,
        paths: Paths,
        game: G::Data,
    ) -> Self {
        let controller = ControllerManager::detached(settings.controller.enabled);
        let bindings = KeyBindings::from_saved(&settings.bindings);
        let sound = G::sound();
        let ui_sounds = sound.ui.clone();
        let delivery = Delivery::new(sound.pacer, G::verbosity());
        // Witnesses only guard the import of old plain files, so a game that
        // does not import leaves none.
        let (witnesses, checked) = if G::IMPORTS_PLAIN_SAVES {
            (paths.witnesses.clone(), paths.checked_witnesses.clone())
        } else {
            (Vec::new(), Vec::new())
        };
        let vault = Vault::new(&paths.data_dir)
            .with_witnesses(witnesses)
            .checking_witnesses(checked);
        #[cfg(feature = "online")]
        let online_config = online_config::<G>();
        #[cfg(feature = "online")]
        let rooms = hearken_rooms::Rooms::new(
            online_config
                .as_ref()
                .filter(|c| c.rooms)
                .map(|c| hearken_rooms::RoomsConfig::new(c.url.clone(), c.site.clone())),
        );
        #[cfg(feature = "online")]
        let online = crate::online::Online::new(online_config, &settings.online);
        let mut ctx = Ctx {
            #[cfg(feature = "online")]
            online,
            #[cfg(feature = "online")]
            rooms,
            #[cfg(feature = "online")]
            room_notices: Vec::new(),
            #[cfg(feature = "update")]
            updates: crate::updates::Updates::new(G::updates()),
            speech,
            translator: Arc::new(Translator::english()),
            discord: crate::discord::Discord::dormant(),
            settings,
            vault,
            paths,
            game,
            held: HeldKeys::default(),
            controller,
            bindings,
            running: true,
            transcript: Vec::new(),
            said: 0,
            notices: Vec::new(),
            changes: Vec::new(),
            changed: 0,
            screen: "none",
            take_owner: None,
            frame: 0,
            clock: 0.0,
            audio: Audio::silent(),
            ui_sounds,
            soundtrack: crate::soundtrack::Soundtrack::new(G::soundtrack()),
            sound,
            delivery,
            pending: Vec::new(),
            commentary: None,
            hold_interrupt: false,
        };
        // So saved bus volumes are in effect from the very first frame, not
        // just once the real backend has finished opening.
        ctx.install_audio(Audio::silent());
        ctx
    }

    /// Installs `audio` as the pool the game plays through: shuts down the
    /// pool it replaces (a harmless no-op the first time, since that one
    /// never played anything), then applies the game's spatial model, fades
    /// and duck buses, then the player's saved volumes.
    pub fn install_audio(&mut self, audio: Audio) {
        self.audio.shutdown();
        self.audio = audio;
        self.audio.spatial = self.sound.spatial;
        self.audio.fades = self.sound.fades;
        let duck_buses = self.sound.duck_buses.clone();
        self.audio.set_duck_buses(&duck_buses);
        self.apply_audio_settings();
    }

    /// Pushes the player's saved per-bus volumes (or this game's own default
    /// for a bus with nothing saved) to the audio pool.
    pub fn apply_audio_settings(&mut self) {
        for (bus, _) in self.buses() {
            let percent = self.volume_percent(bus);
            self.audio.set_bus_volume(bus, percent as f32 / 100.0);
        }
    }

    /// `bus`'s volume as a percent: the player's saved value (capped at
    /// 100), or this game's own default (`sound().default_volume`) when
    /// nothing is saved for it.
    pub fn volume_percent(&self, bus: Bus) -> u8 {
        self.settings
            .audio
            .saved_volume(bus)
            .unwrap_or_else(|| (self.sound.default_volume)(bus))
    }

    /// Every bus the player can set a volume for: the engine's own six, then
    /// the game's own.
    pub fn buses(&self) -> Vec<(Bus, &'static str)> {
        let mut buses: Vec<(Bus, &'static str)> = BUS_LABELS.to_vec();
        buses.extend(self.sound.buses.iter().copied());
        buses
    }

    /// Plays `sound` on the UI bus; `Sound::NONE` when the game has pointed
    /// it at nothing.
    pub fn play_ui(&mut self, sound: UiSound) -> Sound {
        match self.ui_sounds.get(sound) {
            Some(key) => self.audio.play(key, Bus::UI),
            None => Sound::NONE,
        }
    }

    /// The game's sound configuration, as `sound()` returned it at startup.
    pub fn sound_config(&self) -> &SoundConfig {
        &self.sound
    }

    /// Per-frame audio upkeep: re-mixes positioned sounds against the
    /// listener and the game's occlusion hook, and speaks a one-time notice
    /// (such as "no sound device") once the pool has one.
    pub(crate) fn update_audio(&mut self) {
        let listener = self.audio.listener().pos;
        let game = &self.game;
        self.audio.update(&|pos| G::occluded(game, listener, pos));
        if let Some(notice) = self.audio.take_notice() {
            self.say_queued(notice);
        }
    }

    /// Collects finished online requests, and saves the account when it
    /// changed; keeps `rooms` on the account's token and collects what
    /// changed in its rooms. Once a frame.
    #[cfg(feature = "online")]
    pub(crate) fn update_online(&mut self) {
        if self.online.update() {
            self.settings.online = self.online.saved();
            self.save_settings();
        }
        self.rooms
            .set_token(self.online.token().map(str::to_string));
        let notices = self.rooms.update();
        if self.rooms.was_signed_out() {
            self.online.signed_out_elsewhere();
        }
        self.room_notices.extend(notices);
        if self.room_notices.len() > MAX_ROOM_NOTICES {
            let drop = self.room_notices.len() - MAX_ROOM_NOTICES;
            self.room_notices.drain(..drop);
        }
    }

    /// What changed in the player's rooms since the last call: joins, turns,
    /// phases and the connection. The top screen takes them in `update`;
    /// what no screen takes is dropped after a while.
    #[cfg(feature = "online")]
    pub fn take_room_notices(&mut self) -> Vec<hearken_rooms::Notice> {
        std::mem::take(&mut self.room_notices)
    }

    /// Collects the check at start, and announces an update it found. Once
    /// a frame.
    #[cfg(feature = "update")]
    pub(crate) fn update_updates(&mut self, safe_to_prompt: bool) {
        let update = self.updates.poll_background(&self.settings.updates);
        if self.updates.announces() {
            if self.updates.prompts_at_start() {
                if safe_to_prompt && !self.has_pending() && self.updates.take_prompt() {
                    let found = self.updates.available().cloned();
                    self.push(crate::screens::UpdateCheckScreen::at_start(found));
                }
                return;
            }
            let Some(update) = update else {
                return;
            };
            let title = self.updates.title(&update);
            self.say_queued(format!(
                "{title} is available. To install it, choose Check for updates in Settings."
            ));
        }
    }

    /// Speak on the main channel now, cutting off what it was saying.
    pub fn say(&mut self, text: impl Into<String>) {
        self.speak(text.into(), true);
    }

    /// Speak on the main channel after what it is saying.
    pub fn say_queued(&mut self, text: impl Into<String>) {
        self.speak(text.into(), false);
    }

    /// Read a settings change back through the voice it changed, so a new
    /// speech rate is heard at that rate; on the main channel when no voice
    /// can be adjusted that way. `setting` is a key such as `"speech_rate"`.
    pub fn say_preview(&mut self, setting: &str, text: impl Into<String>) {
        let text = self.tr(&text.into());
        if text.trim().is_empty() {
            return;
        }
        if !self.speech.say_adjustment_preview(setting, &text, true) {
            self.speech.say(&text, true);
        }
        self.record(text);
    }

    /// Runs `f` with every `say` in it queued behind what is already being
    /// said instead of cutting it off, so a screen's own announcement can
    /// follow a line spoken first (a result, a notice) without silencing it.
    pub fn queued_speech<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let held = std::mem::replace(&mut self.hold_interrupt, true);
        let out = f(self);
        self.hold_interrupt = held;
        out
    }

    fn speak(&mut self, text: String, interrupt: bool) {
        if text.trim().is_empty() {
            return;
        }
        let text = self.tr(&text);
        if text.trim().is_empty() {
            return;
        }
        self.speech.say(&text, interrupt && !self.hold_interrupt);
        self.record(text);
    }

    /// Writes `line` to the transcript and the session log without
    /// speaking it, for speech that reached the player another way (the
    /// commentary's voice).
    pub fn transcribe(&mut self, line: impl Into<String>) {
        self.record(line.into());
    }

    pub(crate) fn record(&mut self, line: String) {
        log::info!(target: TRANSCRIPT_TARGET, "{line}");
        self.transcript.push(line);
        self.said += 1;
        if self.transcript.len() > TRANSCRIPT_CAP {
            let excess = self.transcript.len() - TRANSCRIPT_CAP;
            self.transcript.drain(..excess);
        }
    }

    /// The name of the screen on top of the stack: the one whose handlers
    /// run. `"none"` before the first screen.
    pub fn top_screen(&self) -> &'static str {
        self.screen
    }

    /// Notes a phase change on the top screen: logged under
    /// [`PHASE_TARGET`] and kept in `changes`. `Phases::set` calls this.
    pub(crate) fn note_phase(&mut self, from: &'static str, to: &'static str) {
        let line = format!("{}: {from} -> {to}", self.screen);
        log::info!(target: PHASE_TARGET, "{line}");
        self.note_change(format!("[phase] {line}"));
    }

    /// Notes a screen-stack change: logged under [`SCREENS_TARGET`] and kept
    /// in `changes`.
    pub(crate) fn note_screen(&mut self, line: String) {
        log::info!(target: SCREENS_TARGET, "{line}");
        self.note_change(format!("[screen] {line}"));
    }

    /// Starts a take of every clip started on `buses` from now on (see
    /// [`hearken_audio::take`]). The take belongs to the screen on top: it
    /// is held while another screen covers that one (a pause menu,
    /// Settings, the Soundtrack), so the held span is left out of it and a
    /// covering screen's own music leaves no trace; and it is dropped if
    /// that screen leaves the stack without ending it. Each change of its
    /// state is kept in `changes` as `[take] recording -> held`.
    pub fn begin_take(&mut self, buses: &[Bus]) {
        let from = self.audio.take_state();
        self.audio.begin_take(buses);
        self.take_owner = Some(self.screen);
        self.note_take(from, TakeState::Recording);
    }

    /// Ends the take and hands it back; `None` when none was recording.
    pub fn end_take(&mut self) -> Option<Take> {
        let from = self.audio.take_state();
        let take = self.audio.end_take();
        self.take_owner = None;
        if take.is_some() {
            self.note_take(from, TakeState::Ended);
        }
        take
    }

    /// Where the take recorder is: idle with no take.
    pub fn take_state(&self) -> TakeState {
        self.audio.take_state()
    }

    /// Holds the take while a screen other than its owner is on top, lets
    /// it run when its owner is back, and drops it once its owner has left
    /// the stack. The app calls it after every stack change, with the
    /// stack's screen names bottom to top.
    pub(crate) fn sync_take(&mut self, stack: &[&'static str]) {
        let Some(owner) = self.take_owner else {
            return;
        };
        let from = self.audio.take_state();
        if !stack.contains(&owner) {
            log::warn!(target: hearken_audio::take::TARGET, "{owner} left without ending its take; dropped");
            self.audio.end_take();
            self.take_owner = None;
            self.note_take(from, TakeState::Idle);
            return;
        }
        self.audio.hold_take(stack.last() != Some(&owner));
        let to = self.audio.take_state();
        if from != to {
            self.note_take(from, to);
        }
    }

    fn note_take(&mut self, from: TakeState, to: TakeState) {
        self.note_change(format!("[take] {} -> {}", from.name(), to.name()));
    }

    fn note_change(&mut self, line: String) {
        self.changes.push(line);
        self.changed += 1;
        if self.changes.len() > CHANGES_CAP {
            let excess = self.changes.len() - CHANGES_CAP;
            self.changes.drain(..excess);
        }
    }

    /// Silence both voices and the commentary, and drop the event backlog
    /// the pacer was projecting.
    pub fn stop_speech(&mut self) {
        self.speech.stop();
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            pacer.reset();
        }
        self.stop_commentary();
        // Nothing is talking now, so the duck lifts on the next frame.
        self.delivery.duck_until = 0.0;
    }

    /// Push the saved speech settings to the voices.
    pub fn apply_speech_settings(&mut self) {
        hearken_speech::apply_speech_settings(self.speech.as_mut(), &mut self.settings.speech);
    }

    /// Switches between the player's chosen built-in voice and automatic
    /// speech selection, announces the selection, and saves it.
    pub fn toggle_builtin_voice(&mut self) {
        if hearken_speech::builtin_voices().is_empty() {
            return;
        }
        let speech = &mut self.settings.speech;
        speech.self_voicing = !speech.self_voicing;
        let line = if speech.self_voicing {
            "Speech voice set to built-in."
        } else {
            "Speech voice set to automatic."
        };
        self.apply_speech_settings();
        self.save_settings();
        self.say(line);
    }

    /// Turns Discord status on or off, as the player's setting. Saved with
    /// the other settings.
    pub fn set_discord_status(&mut self, on: bool) {
        self.settings.discord.enabled = on;
        self.discord.set_enabled(on);
    }

    /// Write `settings.json`. A failure is logged: the game plays on with
    /// the settings it has.
    pub fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.paths) {
            log::error!("could not save settings: {e}");
        }
    }

    /// Keep the player's bindings in the settings and save them.
    pub fn save_bindings(&mut self) {
        self.bindings.store(&mut self.settings.bindings);
        self.save_settings();
    }

    /// The game action a key press or pad button means, by the player's
    /// bindings. `None` for anything else, and for a press no action has.
    pub fn action_for(&self, event: &InputEvent) -> Option<G::Action> {
        match event {
            InputEvent::KeyDown { key, mods, .. } => self.bindings.action_for(*key, *mods),
            InputEvent::ControllerButtonDown { button, .. } => self
                .bindings
                .pad_action_for(*button, self.controller.modifier()),
            _ => None,
        }
    }

    /// Whether `action`'s key or pad button is held down right now.
    pub fn action_held(&self, action: G::Action) -> bool {
        self.bindings.pressed(&self.held, action)
            || self.bindings.pad_chords(action).iter().any(|chord| {
                self.controller.is_down(chord.button)
                    && (!chord.modified || self.controller.modifier())
            })
    }

    /// `action`'s control as a prompt names it: the pad button while the
    /// player is on a controller and the action has one, else the key.
    pub fn control_name(&self, action: G::Action) -> String {
        if self.controller.device() == InputDevice::Controller
            && !self.bindings.pad_chords(action).is_empty()
        {
            self.bindings.pad_spoken(action)
        } else {
            self.bindings.spoken(action)
        }
    }

    pub fn push(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Push(Box::new(screen)));
    }

    /// Close the top screen. The screen revealed announces itself afresh.
    pub fn pop(&mut self) {
        self.pending.push(StackOp::Pop);
    }

    /// Close the top screen without the one revealed saying anything, for
    /// a screen that has already said everything the player needs.
    pub fn pop_quiet(&mut self) {
        self.pending.push(StackOp::PopReveal(Reveal::Quiet));
    }

    /// Close the top screen and hand `result` to the one revealed, which
    /// speaks it first and then its context instead of its whole
    /// announcement: "Via Ames added.", then the menu's current row.
    pub fn pop_with(&mut self, result: impl Into<String>) {
        self.pending
            .push(StackOp::PopReveal(Reveal::With(result.into())));
    }

    /// Close screens until the top one's `State::name` is `name`, which
    /// then announces itself afresh. When no screen on the stack has that
    /// name, nothing is closed and a warning is logged.
    pub fn pop_to(&mut self, name: &str) {
        self.pending
            .push(StackOp::PopTo(name.to_string(), Reveal::Announce));
    }

    /// `pop_to`, with the revealed screen saying nothing, as `pop_quiet`.
    pub fn pop_to_quiet(&mut self, name: &str) {
        self.pending
            .push(StackOp::PopTo(name.to_string(), Reveal::Quiet));
    }

    /// `pop_to`, handing `result` to the revealed screen, as `pop_with`.
    pub fn pop_to_with(&mut self, name: &str, result: impl Into<String>) {
        self.pending.push(StackOp::PopTo(
            name.to_string(),
            Reveal::With(result.into()),
        ));
    }

    pub fn replace(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Replace(Box::new(screen)));
    }

    pub fn reset_to(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Reset(Box::new(screen)));
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub(crate) fn take_pending(&mut self) -> Vec<StackOp<G>> {
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }
}

/// Room notices kept for a screen to take before the oldest are dropped.
#[cfg(feature = "online")]
const MAX_ROOM_NOTICES: usize = 256;

/// The game's server: `Game::online`, or in a developer build (not made by
/// `hearken release`), `<PREFIX>_ONLINE_URL` when it is set. The test
/// server offers guests and rooms whatever the game's own does, so a game
/// can try them before its server has them.
#[cfg(feature = "online")]
fn online_config<G: Game>() -> Option<crate::online::OnlineConfig> {
    let config = G::online();
    if hearken_replay::BuildId::is_release() || hearken_replay::BuildId::baked().is_some() {
        return config;
    }
    match std::env::var(format!("{}_ONLINE_URL", G::ENV_PREFIX)) {
        Ok(url) if !url.trim().is_empty() => {
            let url = url.trim().to_string();
            // Spoken as its host ("127.0.0.1"), the way a real site is.
            let host = url
                .split("://")
                .nth(1)
                .unwrap_or(&url)
                .split(['/', ':'])
                .next()
                .filter(|h| !h.is_empty())
                .unwrap_or("the test server")
                .to_string();
            let mut test = config.unwrap_or_else(|| crate::online::OnlineConfig::new("", host));
            test.url = url;
            test.guests = true;
            test.rooms = true;
            Some(test)
        }
        _ => config,
    }
}
