//! The presence service with a fake Discord client and a stepped clock: no
//! real pipe, nothing tied to the wall clock. Ported from Freight Fate's
//! tests, which these behaviours came from.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hearken_discord::{
    check_reply, format_activity, ActivityPayload, Clock, DiscordPresence, DiscordPresenceOptions,
    Presence, RpcClient, RpcFactory, MAX_FIELD_LEN,
};

#[derive(Default)]
struct FakeState {
    connect_error: Option<String>,
    update_error: Option<String>,
    connects: usize,
    app_ids: Vec<String>,
    updates: Vec<ActivityPayload>,
    cleared: usize,
    closed: usize,
}

#[derive(Clone, Default)]
struct FakeRpc(Arc<Mutex<FakeState>>);

impl FakeRpc {
    fn with_connect_error(msg: &str) -> Self {
        let me = Self::default();
        me.0.lock().unwrap().connect_error = Some(msg.to_string());
        me
    }

    fn with_update_error(msg: &str) -> Self {
        let me = Self::default();
        me.0.lock().unwrap().update_error = Some(msg.to_string());
        me
    }

    fn connects(&self) -> usize {
        self.0.lock().unwrap().connects
    }

    fn updates(&self) -> Vec<ActivityPayload> {
        self.0.lock().unwrap().updates.clone()
    }

    fn cleared(&self) -> usize {
        self.0.lock().unwrap().cleared
    }

    fn closed(&self) -> usize {
        self.0.lock().unwrap().closed
    }

    fn factory(&self) -> RpcFactory {
        let me = self.clone();
        Arc::new(move |app_id: &str| {
            me.0.lock().unwrap().app_ids.push(app_id.to_string());
            Ok(Box::new(me.clone()) as Box<dyn RpcClient>)
        })
    }
}

impl RpcClient for FakeRpc {
    fn connect(&mut self) -> Result<(), String> {
        let mut st = self.0.lock().unwrap();
        st.connects += 1;
        match &st.connect_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn update(&mut self, payload: &ActivityPayload) -> Result<(), String> {
        let mut st = self.0.lock().unwrap();
        if let Some(e) = &st.update_error {
            return Err(e.clone());
        }
        st.updates.push(payload.clone());
        Ok(())
    }

    fn clear(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().cleared += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().closed += 1;
        Ok(())
    }
}

#[derive(Clone, Default)]
struct ManualClock(Arc<Mutex<f64>>);

impl ManualClock {
    fn advance(&self, s: f64) {
        *self.0.lock().unwrap() += s;
    }

    fn clock(&self) -> Clock {
        let me = self.clone();
        Arc::new(move || *me.0.lock().unwrap())
    }
}

fn options(rpc: &FakeRpc, clock: &ManualClock) -> DiscordPresenceOptions {
    DiscordPresenceOptions {
        clock: clock.clock(),
        rpc_factory: Some(rpc.factory()),
        session_start: Some(1234.0),
        threaded: false,
        ..DiscordPresenceOptions::new("test-app-id")
    }
}

fn make(rpc: &FakeRpc, clock: &ManualClock) -> DiscordPresence {
    DiscordPresence::new(options(rpc, clock))
}

fn threaded(factory: RpcFactory) -> DiscordPresence {
    DiscordPresence::new(DiscordPresenceOptions {
        min_interval_s: 0.0,
        rpc_factory: Some(factory),
        ..DiscordPresenceOptions::new("test-app-id")
    })
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !done() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}

// -- formatting ---------------------------------------------------------------

#[test]
fn format_maps_both_lines_and_the_start() {
    let payload = format_activity(
        &Presence::new("In the lobby").detail("Playing Belfry"),
        Some(1234.0),
    );
    assert_eq!(payload.details, "In the lobby");
    assert_eq!(payload.state.as_deref(), Some("Playing Belfry"));
    assert_eq!(payload.start, Some(1234));
}

#[test]
fn format_leaves_out_an_empty_second_line() {
    let payload = format_activity(&Presence::new("In the lobby"), None);
    assert!(payload.state.is_none());
    assert!(payload.start.is_none());
}

#[test]
fn format_clamps_to_discords_limit() {
    let payload = format_activity(
        &Presence::new("A".repeat(500)).detail("x".repeat(500)),
        None,
    );
    assert!(payload.details.chars().count() <= MAX_FIELD_LEN);
    assert!(payload.state.unwrap().chars().count() <= MAX_FIELD_LEN);
    assert!(payload.details.ends_with('…'));
}

// -- the game's application ID ------------------------------------------------

#[test]
fn the_games_own_app_id_reaches_discord() {
    let rpc = FakeRpc::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        clock: ManualClock::default().clock(),
        rpc_factory: Some(rpc.factory()),
        threaded: false,
        ..DiscordPresenceOptions::new(" 1234567890 ")
    });
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.shutdown();
    assert_eq!(
        rpc.0.lock().unwrap().app_ids,
        vec!["1234567890".to_string()]
    );
}

#[test]
fn no_app_id_leaves_it_dormant() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        app_id: String::new(),
        ..options(&rpc, &clock)
    });
    assert!(!p.available() && !p.enabled());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.shutdown();
    assert_eq!(rpc.connects(), 0);
}

#[test]
fn no_elapsed_counter_when_the_game_turns_it_off() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        show_elapsed: false,
        ..options(&rpc, &clock)
    });
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    assert_eq!(rpc.updates()[0].start, None);
    p.shutdown();
}

// -- off, absent, or Discord closed -------------------------------------------

#[test]
fn disabled_never_touches_discord() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        enabled: false,
        ..options(&rpc, &clock)
    });
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.shutdown();
    assert!(!p.enabled());
    assert_eq!(rpc.connects(), 0);
}

#[test]
fn no_client_leaves_it_dormant() {
    let p = DiscordPresence::new(DiscordPresenceOptions {
        rpc_factory: None,
        threaded: false,
        ..DiscordPresenceOptions::new("test-app-id")
    });
    assert!(!p.enabled());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.shutdown();
}

#[test]
fn discord_closed_is_retried_after_the_backoff() {
    let rpc = FakeRpc::with_connect_error("Discord not running");
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    assert_eq!(rpc.connects(), 1);
    assert!(rpc.updates().is_empty());
    assert!(!p.connected());

    p.update(Some(Presence::new("Playing Belfry")));
    assert_eq!(rpc.connects(), 1, "hammered the pipe inside the backoff");

    clock.advance(31.0);
    p.update(Some(Presence::new("Playing Tightrope")));
    assert_eq!(rpc.connects(), 2);
    p.shutdown();
}

#[test]
fn a_failed_update_drops_the_connection() {
    let rpc = FakeRpc::with_update_error("pipe closed");
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    assert!(!p.connected());
    p.shutdown();
}

// -- throttling and de-duplication --------------------------------------------

#[test]
fn the_same_presence_is_sent_once() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    for _ in 0..3 {
        p.update(Some(Presence::new("Playing Belfry").detail("Round 2")));
    }
    assert_eq!(rpc.updates().len(), 1);
    p.shutdown();
}

#[test]
fn none_leaves_the_status_alone() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("Playing Belfry")));
    clock.advance(20.0);
    p.update(None);
    assert_eq!(rpc.updates().len(), 1);
    assert_eq!(rpc.cleared(), 0);
    p.shutdown();
}

#[test]
fn changes_are_throttled_then_flushed() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    assert_eq!(rpc.updates().len(), 1, "the first presence goes at once");

    clock.advance(5.0);
    p.update(Some(Presence::new("Playing Belfry")));
    assert_eq!(rpc.updates().len(), 1, "sent inside the throttle window");

    clock.advance(11.0);
    p.update(Some(Presence::new("Playing Tightrope")));
    assert_eq!(rpc.updates().len(), 2);
    assert_eq!(rpc.updates()[1].details, "Playing Tightrope");
    p.shutdown();
}

// -- shutdown and the switch --------------------------------------------------

#[test]
fn shutdown_clears_and_closes() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.shutdown();
    assert_eq!((rpc.cleared(), rpc.closed()), (1, 1));
    assert!(!p.connected());
}

#[test]
fn shutdown_is_safe_twice_and_without_start() {
    let p = make(&FakeRpc::default(), &ManualClock::default());
    p.shutdown();
    p.shutdown();
}

#[test]
fn the_switch_takes_it_down_and_puts_it_back() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = make(&rpc, &clock);
    p.start();
    p.update(Some(Presence::new("Playing Belfry")));

    p.set_enabled(false);
    assert!(!p.enabled());
    assert_eq!(rpc.cleared(), 1);
    p.update(Some(Presence::new("In the lobby")));
    assert_eq!(rpc.updates().len(), 1, "sent while switched off");

    p.set_enabled(true);
    assert_eq!(rpc.updates().len(), 2, "switching on did not re-show it");
    assert_eq!(rpc.updates()[1].details, "Playing Belfry");

    clock.advance(20.0);
    p.update(Some(Presence::new("In the lobby")));
    assert_eq!(rpc.updates().len(), 3);
    p.shutdown();
}

// -- walking away -------------------------------------------------------------

#[test]
fn an_unchanged_presence_comes_down_once_and_a_change_puts_it_back() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        min_interval_s: 0.0,
        idle_clear_s: 100.0,
        ..options(&rpc, &clock)
    });
    p.start();
    p.update(Some(Presence::new("Paused")));

    clock.advance(60.0);
    p.update(Some(Presence::new("Paused")));
    p.tick();
    assert_eq!(rpc.cleared(), 0, "cleared before the idle window closed");

    clock.advance(60.0);
    p.tick();
    assert_eq!(rpc.cleared(), 1, "an idle presence should come down");

    clock.advance(500.0);
    p.tick();
    p.tick();
    assert_eq!(rpc.cleared(), 1, "the clear repeated while still idle");
    assert_eq!(rpc.updates().len(), 1);

    p.update(Some(Presence::new("Playing Belfry")));
    assert_eq!(rpc.updates().len(), 2, "the presence did not come back");
    p.shutdown();
}

#[test]
fn switching_back_on_restarts_the_idle_clock() {
    let rpc = FakeRpc::default();
    let clock = ManualClock::default();
    let p = DiscordPresence::new(DiscordPresenceOptions {
        idle_clear_s: 100.0,
        ..options(&rpc, &clock)
    });
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    p.set_enabled(false);
    clock.advance(1_000.0);
    p.set_enabled(true);
    let cleared = rpc.cleared();
    clock.advance(1.0);
    p.tick();
    assert_eq!(rpc.cleared(), cleared, "hidden by the old idle age");
    p.shutdown();
}

// -- the worker thread --------------------------------------------------------

#[test]
fn the_threaded_service_sends_and_shuts_down() {
    let rpc = FakeRpc::default();
    let p = threaded(rpc.factory());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    wait_until(|| !rpc.updates().is_empty());
    assert!(p.connected());
    p.shutdown();
    assert!(!rpc.updates().is_empty());
    assert_eq!((rpc.cleared(), rpc.closed()), (1, 1));
}

/// A handshake that blocks until the test lets it go: Discord's handshake
/// is a pipe read with no timeout, and Discord stops answering them for a
/// while after quick relaunches.
#[derive(Clone, Default)]
struct BlockingRpc {
    inner: FakeRpc,
    entered: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
}

impl BlockingRpc {
    fn factory(&self) -> RpcFactory {
        let me = self.clone();
        Arc::new(move |_: &str| Ok(Box::new(me.clone()) as Box<dyn RpcClient>))
    }

    fn await_handshake(&self) {
        wait_until(|| self.entered.load(Ordering::SeqCst));
        assert!(
            self.entered.load(Ordering::SeqCst),
            "never reached the handshake"
        );
    }

    fn release_and_settle(&self) {
        self.released.store(true, Ordering::SeqCst);
        wait_until(|| self.inner.connects() > 0);
        thread::sleep(Duration::from_millis(100));
    }
}

impl RpcClient for BlockingRpc {
    fn connect(&mut self) -> Result<(), String> {
        self.entered.store(true, Ordering::SeqCst);
        while !self.released.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
        self.inner.connect()
    }

    fn update(&mut self, payload: &ActivityPayload) -> Result<(), String> {
        self.inner.update(payload)
    }

    fn clear(&mut self) -> Result<(), String> {
        self.inner.clear()
    }

    fn close(&mut self) -> Result<(), String> {
        self.inner.close()
    }
}

#[test]
fn quitting_never_waits_on_a_handshake_and_a_late_one_shows_nothing() {
    let rpc = BlockingRpc::default();
    let p = threaded(rpc.factory());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    rpc.await_handshake();

    let began = Instant::now();
    p.shutdown();
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "quit waited {:?}",
        began.elapsed()
    );

    rpc.release_and_settle();
    assert!(rpc.inner.updates().is_empty(), "shown after quitting");
    assert!(!p.connected());
}

#[test]
fn switching_off_mid_handshake_is_quick_and_a_late_one_shows_nothing() {
    let rpc = BlockingRpc::default();
    let p = threaded(rpc.factory());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    rpc.await_handshake();

    let began = Instant::now();
    p.set_enabled(false);
    assert!(began.elapsed() < Duration::from_millis(500));

    rpc.release_and_settle();
    assert!(rpc.inner.updates().is_empty(), "shown after switching off");
    assert!(!p.connected() && !p.enabled());
}

// -- Discord's replies ----------------------------------------------------------

#[test]
fn a_refusal_is_told_from_an_acknowledgement() {
    let ack = serde_json::json!({"cmd": "SET_ACTIVITY", "evt": null, "data": {}});
    assert!(check_reply(1, &ack).is_ok());
    let refused = serde_json::json!({
        "cmd": "SET_ACTIVITY", "evt": "ERROR",
        "data": {"code": 4000, "message": "rate limited"},
    });
    assert!(check_reply(1, &refused)
        .unwrap_err()
        .contains("rate limited"));
    assert!(check_reply(2, &serde_json::json!({"code": 1000})).is_err());
}

/// The real client with no Discord running (as on any test machine): it
/// fails quietly, and quitting is quick.
#[cfg(feature = "ipc")]
#[test]
fn the_real_client_is_silent_without_discord() {
    let p = DiscordPresence::new(DiscordPresenceOptions::new("1234567890"));
    assert!(p.available());
    p.start();
    p.update(Some(Presence::new("In the lobby")));
    thread::sleep(Duration::from_millis(100));
    let began = Instant::now();
    p.shutdown();
    assert!(began.elapsed() < Duration::from_millis(500));
}
