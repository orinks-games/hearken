//! Seated runs through the Pebbles example: a match recorded as one run,
//! replayed, read mid-way, and built from a server's turns.

#[path = "../examples/pebbles.rs"]
mod pebbles;

use hearken_replay::pebbles::{Pebbles, Setup, Take};
use hearken_replay::wasm::{PlayRequest, PlayTurn};
use hearken_replay::{assert_replays, progress, replay, InputCode, Recording, Seat, Seated};
use pebbles::hearken_rules;
use serde_json::{json, Value};

fn seat(n: u8) -> Seat {
    Seat::new(n).unwrap()
}

/// A match of 10 pebbles: seat 1 takes 3, seat 2 takes 3, seat 1 takes 3,
/// seat 2 must take the last and loses.
fn recorded() -> Recording<Pebbles> {
    let mut run = Recording::<Pebbles>::new(7, &Setup { pile: 10 }).unwrap();
    for (s, n) in [(0, 3), (1, 3), (0, 3), (1, 1)] {
        assert!(run.step(&[Seated::new(seat(s), Take(n))]));
    }
    run
}

#[test]
fn a_match_records_and_replays_as_one_run() {
    let run = recorded();
    assert!(run.finished());
    let standing = run.outcome();
    assert_eq!(standing.winner, Some(0));
    assert_eq!(standing.next_seat, None);
    assert_eq!(standing.summary, vec!["You won", "You lost"]);
    assert_replays(&run);
    let file = run.to_run();
    // Every input carries its seat in the top four bits.
    assert_eq!(file.inputs[1].code >> 12, 1);
    assert_eq!(file.inputs[1].code & 0xfff, 1);
    let replayed = replay::<Pebbles>(&file).unwrap();
    assert_eq!(replayed.pile(), 0);
}

#[test]
fn the_wrong_seat_is_ignored_and_leaving_loses() {
    let mut run = Recording::<Pebbles>::new(1, &Setup { pile: 10 }).unwrap();
    // Seat 2 tries to move first: nothing happens.
    run.step(&[Seated::new(seat(1), Take(3))]);
    assert_eq!(run.sim().pile(), 10);
    assert_eq!(run.sim().turn(), seat(0));
    run.step(&[Seated::new(seat(0), Take(2))]);
    assert_eq!(run.sim().pile(), 8);
    run.step(&[Seated::Left { seat: seat(1) }]);
    assert!(run.finished());
    assert_eq!(run.outcome().winner, Some(0));
    assert_replays(&run);
}

#[test]
fn progress_reads_a_match_in_the_middle() {
    let mut run = Recording::<Pebbles>::new(1, &Setup { pile: 10 }).unwrap();
    run.step(&[Seated::new(seat(0), Take(3))]);
    let file = run.to_run();
    assert_eq!(
        replay::<Pebbles>(&file).err().map(|e| e.code()),
        Some("unfinished")
    );
    let so_far = progress::<Pebbles>(&file).unwrap();
    assert!(!so_far.finished);
    assert_eq!(so_far.ticks, 1);
    assert_eq!(so_far.outcome.pile, 7);
    assert_eq!(so_far.outcome.next_seat, Some(1));
    assert_eq!(
        so_far.outcome.summary[1],
        "Their move, 7 left".replace("Their", "Your")
    );
    assert_eq!(so_far.outcome.summary[0], "Their move, 7 left");
}

#[test]
fn a_server_builds_the_same_run_from_its_turns() {
    let request = PlayRequest {
        rules: "pebbles".into(),
        seed: json!("7"),
        context: json!({ "pile": 10 }),
        turns: vec![
            PlayTurn {
                seat: 0,
                inputs: vec![(1, 3)],
                left: false,
            },
            PlayTurn {
                seat: 1,
                inputs: vec![(1, 3)],
                left: false,
            },
            PlayTurn {
                seat: 0,
                inputs: vec![(1, 3)],
                left: false,
            },
            PlayTurn {
                seat: 1,
                inputs: vec![(1, 1)],
                left: false,
            },
        ],
        finish: true,
    };
    let built = request.to_run::<Pebbles>().unwrap();
    let mut live = recorded().to_run();
    live.build = built.build;
    assert_eq!(built, live);
}

fn play(request: Value) -> Value {
    serde_json::from_str(&hearken_rules::play(request.to_string().as_bytes())).unwrap()
}

#[test]
fn the_module_plays_a_match_from_a_servers_turns() {
    let turns = json!([
        { "seat": 0, "inputs": [[1, 3]] },
        { "seat": 1, "inputs": [[1, 3]] },
    ]);
    let answer = play(json!({
        "rules": "pebbles", "seed": "7", "context": { "pile": 10 }, "turns": turns
    }));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["finished"], false);
    assert_eq!(answer["ticks"], 2);
    assert_eq!(answer["outcome"]["pile"], 4);
    assert_eq!(answer["outcome"]["next_seat"], 0);

    // Asked to finish, an unfinished match is refused.
    let refused = play(json!({
        "rules": "pebbles", "seed": "7", "context": { "pile": 10 }, "turns": turns, "finish": true
    }));
    assert_eq!(refused["error"], "unfinished");

    // A seat leaving ends it.
    let left = play(json!({
        "rules": "pebbles", "seed": "7", "context": { "pile": 10 },
        "turns": [{ "seat": 0, "inputs": [[1, 3]] }, { "seat": 0, "left": true }],
        "finish": true
    }));
    assert_eq!(left["ok"], true, "{left}");
    assert_eq!(left["outcome"]["winner"], 1);

    // An input the rules do not have, a seat past 16, other rules.
    for (request, code) in [
        (
            json!({ "rules": "pebbles", "seed": "7", "context": { "pile": 10 },
                "turns": [{ "seat": 0, "inputs": [[9, 1]] }] }),
            "bad-input",
        ),
        (
            json!({ "rules": "pebbles", "seed": "7", "context": { "pile": 10 },
                "turns": [{ "seat": 16, "inputs": [[1, 1]] }] }),
            "bad-input",
        ),
        (
            json!({ "rules": "marbles", "seed": "7", "context": { "pile": 10 } }),
            "wrong-rules",
        ),
        (json!({ "rules": "pebbles", "seed": "7" }), "bad-request"),
    ] {
        assert_eq!(play(request)["error"], code);
    }
}

#[test]
fn seated_codes_read_back_through_the_recording() {
    let input = Seated::new(seat(3), Take(2));
    let (code, value) = input.to_code();
    assert_eq!(<Seated<Take>>::from_code(code, value), Some(input));
}

/// Writes seated runs for the JavaScript host's tests, beside Coin Dash's.
#[test]
fn writes_fixtures_for_the_javascript_host() {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let dir = target.join("hearken-replay-fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    let run = recorded();
    let file = run.to_run().encode().unwrap();
    std::fs::write(dir.join("pebbles-7-10.hkrun"), &file).unwrap();
    std::fs::write(
        dir.join("pebbles.json"),
        serde_json::to_string_pretty(&json!({
            "file": "pebbles-7-10.hkrun",
            "seed": "7",
            "context": { "pile": 10 },
            "turns": [
                { "seat": 0, "inputs": [[1, 3]] },
                { "seat": 1, "inputs": [[1, 3]] },
                { "seat": 0, "inputs": [[1, 3]] },
                { "seat": 1, "inputs": [[1, 1]] },
            ],
            "outcome": serde_json::to_value(run.outcome()).unwrap(),
        }))
        .unwrap(),
    )
    .unwrap();
}
