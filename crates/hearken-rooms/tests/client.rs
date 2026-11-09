//! The rooms client against a scripted server: a room made and joined,
//! polls absorbed into events and notices, a match stepped by the turns
//! the server ordered, and the connection going offline and back.

#[path = "../../hearken-replay/examples/pebbles.rs"]
#[allow(dead_code)]
mod pebbles;

use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_replay::Seat;
use hearken_rooms::transport::{reply, Scripted};
use hearken_rooms::{
    Answer, Connection, Match, Notice, Phase, RoomId, Rooms, RoomsConfig, RoomsError, Sync,
    POLL_ACTIVE, POLL_QUIET, QUIET_AFTER,
};
use pebbles::{Pebbles, Setup, Take};
use serde_json::{json, Value};

const ID: &str = "0123456789abcdef01234567";

fn room_id() -> RoomId {
    RoomId(ID.to_string())
}

/// A room as the server describes it, in `phase`, with `seq` events.
fn room(phase: &str, seq: u64, members: &[(u8, &str)], extra: Value) -> Value {
    let mut v = json!({
        "id": ID,
        "kind": "pebbles",
        "code": "M4K7T8",
        "phase": phase,
        "open": false,
        "seats": { "min": 2, "max": 2 },
        "host": 0,
        "members": members.iter().map(|(seat, name)| json!({
            "seat": seat, "name": name, "guest": false, "here": true, "left": false
        })).collect::<Vec<_>>(),
        "you": 0,
        "seed": null,
        "context": { "pile": 10 },
        "seq": seq,
        "snapshot": null,
        "result": null,
        "updated_at": 1000,
        "turn_started_at": null,
    });
    if let Value::Object(more) = extra {
        for (k, val) in more {
            v[k] = val;
        }
    }
    v
}

fn ok_room(room: Value) -> Result<hearken_rooms::Reply, String> {
    reply(200, json!({ "ok": true, "room": room }))
}

fn ok_events(room: Value, events: Vec<Value>, more: bool) -> Result<hearken_rooms::Reply, String> {
    reply(
        200,
        json!({ "ok": true, "room": room, "events": events, "more": more }),
    )
}

fn turn(seq: u64, seat: u8, n: i32) -> Value {
    json!({ "seq": seq, "type": "turn", "at": 1000 + seq, "seat": seat, "inputs": [[1, n]] })
}

fn client() -> (Rooms, Arc<Scripted>) {
    let mut rooms = Rooms::new(Some(RoomsConfig::new(
        "https://example.test/api/games/demo",
        "example.test",
    )));
    let script = Scripted::new();
    rooms.set_transport(script.clone());
    rooms.set_token(Some("tok".into()));
    (rooms, script)
}

/// Runs `update_at(now)` until nothing is on its way, collecting notices.
fn settle(rooms: &mut Rooms, now: Instant) -> Vec<Notice> {
    let mut notices = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        notices.extend(rooms.update_at(now));
        let busy = rooms.describe()[0].contains("in flight");
        if !busy || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    notices
}

fn seat(n: u8) -> Seat {
    Seat::new(n).unwrap()
}

#[test]
fn nothing_is_sent_without_a_server_or_a_token() {
    let mut none = Rooms::new(None);
    assert!(!none.is_offered());
    assert_eq!(
        none.create("pebbles", &(), false).err(),
        Some(RoomsError::NotOffered)
    );

    let mut rooms = Rooms::new(Some(RoomsConfig::new("https://x.test", "x.test")));
    assert!(rooms.is_offered());
    assert!(!rooms.is_connected());
    assert_eq!(rooms.join("M4K7T8").err(), Some(RoomsError::NotConnected));
    rooms.set_token(Some("tok".into()));
    assert_eq!(
        rooms.create("Pebbles", &(), false).err(),
        Some(RoomsError::BadKind)
    );
    assert_eq!(rooms.join("M4K7").err(), Some(RoomsError::BadCode));
    assert_eq!(
        rooms
            .say(&room_id(), "  ")
            .err()
            .map(|e| e.code().to_string()),
        Some("empty".into())
    );
}

#[test]
fn a_room_is_made_and_a_player_joins() {
    let (mut rooms, script) = client();
    let now = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    let ticket = rooms.create("pebbles", &Setup { pile: 10 }, true).unwrap();
    assert!(rooms.is_pending(ticket));
    let notices = settle(&mut rooms, now);
    assert_eq!(rooms.result(ticket), Some(Ok(Answer::Room(room_id()))));
    assert_eq!(rooms.result(ticket), None, "an answer is taken once");
    assert_eq!(notices, vec![Notice::Updated(room_id())]);
    let info = &rooms.room(&room_id()).unwrap().info;
    assert_eq!(info.phase, Phase::Waiting);
    assert!(info.is_host());
    assert_eq!(info.you(), Some(seat(0)));
    assert_eq!(info.others(), Vec::<String>::new());

    let sent = script.requests.lock().unwrap()[0].clone();
    assert_eq!(sent.0, "POST");
    assert!(sent.1.ends_with("/rooms?kind=pebbles"), "{}", sent.1);
    assert_eq!(sent.2, json!({ "context": { "pile": 10 }, "open": true }));

    // The second player's join: a code of any shape the player types.
    script.push(ok_room(room(
        "waiting",
        2,
        &[(0, "Ada"), (1, "Bo")],
        json!({ "you": 1 }),
    )));
    let ticket = rooms.join(" m4k-7t8 ").unwrap();
    let notices = settle(&mut rooms, now);
    assert_eq!(rooms.result(ticket), Some(Ok(Answer::Room(room_id()))));
    assert_eq!(notices, vec![Notice::Updated(room_id())]);
    assert_eq!(
        script.requests.lock().unwrap()[1].2,
        json!({ "code": "M4K7T8" })
    );
    assert_eq!(
        rooms.room(&room_id()).unwrap().info.others(),
        vec!["Ada".to_string()]
    );
}

#[test]
fn the_server_refusals_are_named() {
    let (mut rooms, script) = client();
    let now = Instant::now();
    script.push(reply(404, json!({ "ok": false, "error": "unknown-code" })));
    script.push(reply(
        409,
        json!({ "ok": false, "error": "wrong-phase", "phase": "playing" }),
    ));
    script.push(reply(409, json!({ "ok": false, "error": "full" })));
    script.push(reply(
        422,
        json!({ "ok": false, "error": "too-many-rooms" }),
    ));
    script.push(reply(
        400,
        json!({ "ok": false, "error": "odd", "message": "  Pile too  big" }),
    ));
    script.push(reply(500, json!({ "ok": false })));
    script.push(Err("timed out".into()));
    // One at a time: the script answers in request order.
    let errors: Vec<RoomsError> = (0..7)
        .map(|_| {
            let ticket = rooms.join("M4K7T8").unwrap();
            settle(&mut rooms, now);
            rooms.result(ticket).unwrap().unwrap_err()
        })
        .collect();
    assert_eq!(errors[0], RoomsError::UnknownCode);
    assert_eq!(errors[1], RoomsError::WrongPhase(Phase::Playing));
    assert_eq!(errors[1].spoken("x.test"), "That game is in play.");
    assert_eq!(errors[2], RoomsError::Full);
    assert_eq!(errors[3], RoomsError::TooManyRooms);
    assert_eq!(
        errors[4],
        RoomsError::Refused {
            code: "odd".into(),
            message: Some("Pile too big".into())
        }
    );
    assert_eq!(errors[4].spoken("x.test"), "Pile too big.");
    assert_eq!(errors[5], RoomsError::Unavailable);
    assert_eq!(errors[6], RoomsError::Offline);
    assert_eq!(
        errors[6].spoken("x.test"),
        "Could not reach x.test. Check your connection."
    );
}

#[test]
fn only_so_many_requests_go_at_once() {
    let (mut rooms, script) = client();
    for _ in 0..hearken_rooms::MAX_IN_FLIGHT {
        script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
        rooms.join("M4K7T8").unwrap();
    }
    assert_eq!(rooms.join("M4K7T8").err(), Some(RoomsError::Busy));
    settle(&mut rooms, Instant::now());
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    assert!(rooms.join("M4K7T8").is_ok());
}

#[test]
fn a_watched_room_is_polled_and_its_events_kept() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    rooms.create("pebbles", &Setup { pile: 10 }, false).unwrap();
    settle(&mut rooms, t0);

    // Watching polls at once, from the last event read (none yet).
    let joined = room("waiting", 2, &[(0, "Ada"), (1, "Bo")], json!({}));
    script.push_for(
        "?after=0",
        ok_events(
            joined.clone(),
            vec![
                json!({ "seq": 1, "type": "join", "at": 1001, "seat": 0, "name": "Ada" }),
                json!({ "seq": 2, "type": "join", "at": 1002, "seat": 1, "name": "Bo" }),
            ],
            false,
        ),
    );
    rooms.watch(&room_id(), true);
    let notices = settle(&mut rooms, t0);
    assert_eq!(
        notices,
        vec![Notice::Events(room_id(), 1), Notice::Updated(room_id())]
    );
    let r = rooms.room(&room_id()).unwrap();
    assert_eq!(r.read_seq(), 2);
    assert!(!r.behind());
    assert!(r.is_watched());
    assert!(script
        .paths()
        .last()
        .unwrap()
        .ends_with(&format!("rooms/{ID}?after=0")));

    // Not polled again before the active interval has passed.
    assert!(settle(&mut rooms, t0 + Duration::from_millis(500)).is_empty());
    assert_eq!(script.requests.lock().unwrap().len(), 2);

    // The match starts: a start event and the room in play. A poll that
    // says "more" is followed at once by another.
    let started = room(
        "playing",
        4,
        &[(0, "Ada"), (1, "Bo")],
        json!({
            "seed": "7",
            "snapshot": { "ticks": 1, "finished": false, "outcome": {
                "pile": 7, "next_seat": 1, "winner": null,
                "summary": ["Their move, 7 left", "Your move, 7 left"] } },
        }),
    );
    script.push_for("?after=2", ok_events(
        started.clone(),
        vec![json!({ "seq": 3, "type": "start", "at": 1003, "seed": "7", "context": { "pile": 10 } })],
        true,
    ));
    script.push_for(
        "?after=3",
        ok_events(started.clone(), vec![turn(4, 0, 3)], false),
    );
    let t1 = t0 + POLL_ACTIVE;
    let notices = settle(&mut rooms, t1);
    assert_eq!(
        notices,
        vec![
            Notice::Events(room_id(), 3),
            Notice::Updated(room_id()),
            Notice::Phase(room_id(), Phase::Playing),
            Notice::Events(room_id(), 4),
        ]
    );
    let r = rooms.room(&room_id()).unwrap();
    assert_eq!(r.read_seq(), 4);
    assert_eq!(r.info.seed(), Some(7));
    assert_eq!(r.info.next_seat(), Some(seat(1)));
    assert!(
        !r.info.my_turn(),
        "seat 1 is to move, and the reader is seat 0"
    );
    assert_eq!(
        r.info.summary_for(seat(0)).as_deref(),
        Some("Their move, 7 left")
    );
    assert_eq!(r.events_after(2).count(), 2);

    // Nothing has changed for a minute: polls slow to the quiet pace.
    let quiet = t1 + QUIET_AFTER + Duration::from_secs(1);
    script.push_for("?after=4", ok_events(started.clone(), vec![], false));
    settle(&mut rooms, quiet);
    script.push_for("?after=4", ok_events(started.clone(), vec![], false));
    let before = script.requests.lock().unwrap().len();
    settle(&mut rooms, quiet + POLL_ACTIVE + Duration::from_secs(1));
    assert_eq!(
        script.requests.lock().unwrap().len(),
        before,
        "quiet: no poll yet"
    );
    settle(&mut rooms, quiet + POLL_QUIET);
    assert_eq!(script.requests.lock().unwrap().len(), before + 1);

    // The room ends: it stops being watched.
    let over = room(
        "finished",
        7,
        &[(0, "Ada"), (1, "Bo")],
        json!({ "seed": "7" }),
    );
    script.push_for(
        "?after=4",
        ok_events(
            over,
            vec![turn(5, 1, 3), turn(6, 0, 3), turn(7, 1, 1)],
            false,
        ),
    );
    let t2 = quiet + POLL_QUIET * 2;
    let notices = settle(&mut rooms, t2);
    assert!(notices.contains(&Notice::Phase(room_id(), Phase::Finished)));
    let r = rooms.room(&room_id()).unwrap();
    assert!(!r.is_watched());
    assert_eq!(r.read_seq(), 7);
    assert_eq!(
        rooms.describe()[1],
        "room M4K7T8 pebbles: Finished, seq 7 read 7"
    );
}

#[test]
fn chat_is_sent_clean_and_read_back() {
    let (mut rooms, script) = client();
    let now = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    rooms.create("pebbles", &Setup { pile: 10 }, false).unwrap();
    settle(&mut rooms, now);
    script.push_for("/events", reply(200, json!({ "ok": true, "seq": 2 })));
    let ticket = rooms.say(&room_id(), "  hi\u{200b} there ").unwrap();
    script.push_for("?after=0", ok_events(
        room("waiting", 2, &[(0, "Ada")], json!({})),
        vec![json!({ "seq": 2, "type": "chat", "at": 1002, "seat": 0, "name": "Ada", "text": "hi there" })],
        false,
    ));
    rooms.watch(&room_id(), true);
    settle(&mut rooms, now);
    assert_eq!(rooms.result(ticket), Some(Ok(Answer::Seq(2))));
    let sent = script
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.1.ends_with("/events"))
        .unwrap()
        .2
        .clone();
    assert_eq!(sent, json!({ "type": "chat", "text": "hi there" }));
    let lines: Vec<(&str, &str)> = rooms.room(&room_id()).unwrap().chat().collect();
    assert_eq!(lines, vec![("Ada", "hi there")]);
}

#[test]
fn the_connection_goes_offline_after_two_silent_polls_and_comes_back() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    rooms.create("pebbles", &Setup { pile: 10 }, false).unwrap();
    settle(&mut rooms, t0);
    rooms.watch(&room_id(), true);

    script.push_for("?after=0", Err("no route".into()));
    assert!(settle(&mut rooms, t0).is_empty());
    assert_eq!(
        rooms.connection(),
        Connection::Polling,
        "one miss is not offline"
    );
    script.push_for("?after=0", Err("no route".into()));
    let notices = settle(&mut rooms, t0 + POLL_ACTIVE);
    assert_eq!(notices, vec![Notice::Connection(Connection::Offline)]);
    assert_eq!(rooms.connection(), Connection::Offline);

    script.push_for(
        "?after=0",
        ok_events(room("waiting", 1, &[(0, "Ada")], json!({})), vec![], false),
    );
    let notices = settle(&mut rooms, t0 + POLL_ACTIVE * 2);
    assert_eq!(notices, vec![Notice::Connection(Connection::Polling)]);
    assert_eq!(rooms.connection(), Connection::Polling);
}

#[test]
fn a_refused_poll_drops_the_watch_and_a_signed_out_token_is_reported() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    rooms.create("pebbles", &Setup { pile: 10 }, false).unwrap();
    settle(&mut rooms, t0);
    rooms.watch(&room_id(), true);
    script.push_for(
        "?after=0",
        reply(403, json!({ "ok": false, "error": "not-a-member" })),
    );
    let notices = settle(&mut rooms, t0);
    assert_eq!(
        notices,
        vec![Notice::Dropped(room_id(), RoomsError::NotAMember)]
    );
    assert!(!rooms.room(&room_id()).unwrap().is_watched());

    script.push(reply(401, json!({ "ok": false, "error": "unauthorized" })));
    let ticket = rooms.list().unwrap();
    settle(&mut rooms, t0);
    assert_eq!(rooms.result(ticket), Some(Err(RoomsError::SignedOut)));
    assert!(rooms.was_signed_out());
    assert!(!rooms.was_signed_out(), "reported once");

    // A new token forgets the rooms of the old one, and late answers for
    // the old token are ignored.
    rooms.set_token(Some("tok2".into()));
    assert_eq!(rooms.rooms().count(), 0);
}

#[test]
fn a_list_refreshes_the_rooms_known() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    script.push(ok_room(room("waiting", 1, &[(0, "Ada")], json!({}))));
    rooms.create("pebbles", &Setup { pile: 10 }, false).unwrap();
    settle(&mut rooms, t0);
    let other = json!({
        "id": "fedcba9876543210fedcba98", "kind": "pebbles", "code": "TTTTTT", "phase": "playing",
        "seats": { "min": 2, "max": 2 }, "seq": 9, "unread": 3, "context": {}, "you": 1,
    });
    script.push(reply(
        200,
        json!({ "ok": true, "rooms": [
        other,
        room("waiting", 2, &[(0, "Ada"), (1, "Bo")], json!({ "unread": 1 })),
    ] }),
    ));
    let ticket = rooms.list().unwrap();
    let notices = settle(&mut rooms, t0);
    let Some(Ok(Answer::List(list))) = rooms.result(ticket) else {
        panic!("no list");
    };
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].unread, 3);
    assert_eq!(list[0].phase, Phase::Playing);
    assert_eq!(notices, vec![Notice::Updated(room_id())]);
    assert_eq!(
        rooms.rooms().count(),
        1,
        "a list does not add rooms the game has not opened"
    );
    assert_eq!(rooms.room(&room_id()).unwrap().info.members.len(), 2);
}

#[test]
fn a_match_applies_the_turns_the_server_ordered() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    let started = room(
        "playing",
        3,
        &[(0, "Ada"), (1, "Bo")],
        json!({ "seed": "7" }),
    );
    script.push(ok_room(started.clone()));
    rooms.join("M4K7T8").unwrap();
    settle(&mut rooms, t0);
    let r = rooms.room(&room_id()).unwrap();

    assert_eq!(
        Match::<Pebbles>::new(
            &room("waiting", 1, &[(0, "Ada")], json!({}))
                .pipe(|v| serde_json::from_value(v).unwrap())
        )
        .err(),
        Some(hearken_rooms::MatchError::NotStarted)
    );
    let mut m = Match::<Pebbles>::new(&r.info).unwrap();
    assert_eq!(m.sim().pile(), 10);
    assert_eq!(m.sync(r), Sync::Idle);

    // The player's turn goes to the server, not to the rules.
    script.push_for("/events", reply(200, json!({ "ok": true, "seq": 4 })));
    m.post(&mut rooms, &room_id(), &[Take(3)]).unwrap();
    assert_eq!(
        m.post(&mut rooms, &room_id(), &[Take(1)]).err(),
        Some(RoomsError::Busy)
    );
    assert_eq!(m.sync(rooms.room(&room_id()).unwrap()), Sync::Posting);
    settle(&mut rooms, t0);
    let sent = script
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.1.ends_with("/events"))
        .unwrap()
        .2
        .clone();
    assert_eq!(sent, json!({ "type": "turn", "inputs": [[1, 3]] }));
    m.update(&mut rooms);
    assert_eq!(m.take_refusal(), None);
    assert_eq!(m.sim().pile(), 10, "not applied until the server orders it");

    // The turns come back in the poll: ours, then the other seat's.
    script.push_for(
        "?after=0",
        ok_events(started.clone(), vec![turn(4, 0, 3), turn(5, 1, 2)], false),
    );
    rooms.watch(&room_id(), true);
    settle(&mut rooms, t0);
    let r = rooms.room(&room_id()).unwrap();
    assert_eq!(m.sync(r), Sync::Behind(2));
    let first = m.apply_next(r).unwrap();
    assert_eq!(
        (first.seq, first.seat, first.inputs.clone(), first.left),
        (4, seat(0), vec![Take(3)], false)
    );
    assert_eq!(m.sim().pile(), 7);
    assert_eq!(m.sync(r), Sync::Behind(1));
    let rest = m.apply_all(r);
    assert_eq!(rest.len(), 1);
    assert_eq!(m.sim().pile(), 5);
    assert_eq!(m.sim().turn(), seat(0));
    assert_eq!(m.sync(r), Sync::Idle);
    assert_eq!(m.applied_seq(), 5);

    // The other seat leaves: the rules see `Left` and the match ends.
    script.push_for(
        "?after=5",
        ok_events(
            room(
                "finished",
                6,
                &[(0, "Ada"), (1, "Bo")],
                json!({ "seed": "7" }),
            ),
            vec![json!({ "seq": 6, "type": "left", "at": 1006, "seat": 1, "reason": "left" })],
            false,
        ),
    );
    settle(&mut rooms, t0 + POLL_ACTIVE);
    let r = rooms.room(&room_id()).unwrap();
    let left = m.apply_all(r);
    assert_eq!(left.len(), 1);
    assert!(left[0].left);
    assert!(m.finished());
    assert_eq!(m.recording().outcome().winner, Some(0));
    assert_eq!(m.recording().to_run().ticks, 3);
}

#[test]
fn a_refused_turn_is_reported_once() {
    let (mut rooms, script) = client();
    let t0 = Instant::now();
    let started = room(
        "playing",
        3,
        &[(0, "Ada"), (1, "Bo")],
        json!({ "seed": "7" }),
    );
    script.push(ok_room(started));
    rooms.join("M4K7T8").unwrap();
    settle(&mut rooms, t0);
    let mut m = Match::<Pebbles>::new(&rooms.room(&room_id()).unwrap().info).unwrap();
    script.push_for(
        "/events",
        reply(
            409,
            json!({ "ok": false, "error": "wrong-phase", "phase": "finished" }),
        ),
    );
    m.post(&mut rooms, &room_id(), &[Take(1)]).unwrap();
    settle(&mut rooms, t0);
    m.update(&mut rooms);
    assert_eq!(
        m.take_refusal(),
        Some(RoomsError::WrongPhase(Phase::Finished))
    );
    assert_eq!(m.take_refusal(), None);
    assert_eq!(m.sync(rooms.room(&room_id()).unwrap()), Sync::Idle);
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
