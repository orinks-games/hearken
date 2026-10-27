//! The example rules, recorded and replayed natively, through the same
//! answers the WebAssembly module gives a server. Also writes the runs the
//! JavaScript host's tests verify with the module (`server/test`).

#[path = "../examples/coin_dash.rs"]
mod coin_dash;

use coin_dash::{hearken_rules, Control, Dash, Setup};
use hearken_replay::{
    assert_replays, replay, verify, BuildId, Expected, FormatError, Input, Recording, VerifyError,
};
use serde_json::{json, Value};

/// Plays `seconds` of Coin Dash from `seed`, chasing the coin.
fn play(seed: u64, seconds: u32) -> Recording<Dash> {
    let mut rec = Recording::<Dash>::new(seed, &Setup { seconds }).unwrap();
    let mut held: Option<Control> = None;
    while !rec.finished() {
        let want = match rec.sim().coin().cmp(&rec.sim().lane()) {
            std::cmp::Ordering::Less => Some(Control::Left(true)),
            std::cmp::Ordering::Greater => Some(Control::Right(true)),
            std::cmp::Ordering::Equal => None,
        };
        let mut inputs = Vec::new();
        if want != held {
            match held {
                Some(Control::Left(_)) => inputs.push(Control::Left(false)),
                Some(Control::Right(_)) => inputs.push(Control::Right(false)),
                _ => {}
            }
            if let Some(w) = want {
                inputs.push(w);
            }
            held = want;
        }
        rec.step(&inputs);
    }
    rec
}

fn expected(seed: u64, seconds: u32) -> Expected {
    Expected {
        rules: "coin-dash".into(),
        seed,
        context: json!({ "seconds": seconds }),
    }
}

#[test]
fn a_recorded_run_replays_to_the_same_score() {
    let rec = play(42, 20);
    assert_replays(&rec);
    let score = rec.outcome();
    assert_eq!(score.coins + score.missed, 20);
    assert!(score.coins > 10, "{score:?}");
    let verified = verify::<Dash>(&rec.to_run(), &expected(42, 20)).unwrap();
    assert_eq!(verified.outcome, score);
    assert_eq!(verified.ticks, 20 * coin_dash::HZ);
}

#[test]
fn the_server_decides_the_seed_and_context() {
    let run = play(42, 20).into_run();
    assert_eq!(
        verify::<Dash>(&run, &expected(43, 20)),
        Err(VerifyError::WrongSeed)
    );
    assert_eq!(
        verify::<Dash>(&run, &expected(42, 21)),
        Err(VerifyError::WrongContext)
    );
    let mut other = expected(42, 20);
    other.rules = "sonar-dive".into();
    assert_eq!(verify::<Dash>(&run, &other), Err(VerifyError::WrongRules));
}

#[test]
fn a_run_must_end_where_the_rules_do() {
    let mut run = play(7, 5).into_run();
    run.ticks -= 1;
    run.inputs.retain(|i| i.tick < run.ticks);
    assert_eq!(replay::<Dash>(&run).err(), Some(VerifyError::Unfinished));
    let mut run = play(7, 5).into_run();
    run.ticks += 1;
    assert_eq!(replay::<Dash>(&run).err(), Some(VerifyError::PastTheEnd));
    let mut run = play(7, 5).into_run();
    run.ticks = 601 * coin_dash::HZ;
    assert_eq!(replay::<Dash>(&run).err(), Some(VerifyError::TooLong));
}

#[test]
fn inputs_have_one_meaning() {
    let mut run = play(7, 5).into_run();
    run.inputs.push(Input {
        tick: run.ticks - 1,
        code: 1,
        value: 2,
    });
    assert_eq!(replay::<Dash>(&run).err(), Some(VerifyError::BadInput));
    let mut run = play(7, 5).into_run();
    run.inputs.insert(
        0,
        Input {
            tick: 0,
            code: 9,
            value: 1,
        },
    );
    assert_eq!(replay::<Dash>(&run).err(), Some(VerifyError::BadInput));
}

#[test]
fn quitting_ends_the_run_early() {
    let mut rec = Recording::<Dash>::new(1, &Setup { seconds: 60 }).unwrap();
    for _ in 0..250 {
        rec.step(&[]);
    }
    rec.step(&[Control::Quit]);
    assert!(rec.finished());
    assert!(!rec.step(&[]), "a finished run takes no more steps");
    assert_replays(&rec);
    let score = rec.outcome();
    assert_eq!(score.coins + score.missed, 2, "{score:?}");
}

#[test]
fn the_module_answers_match_native_verification() {
    let rec = play(99, 10);
    let file = rec.to_run().encode().unwrap();
    let request = json!({ "rules": "coin-dash", "seed": "99", "context": { "seconds": 10 } });
    let answer: Value = serde_json::from_str(&hearken_rules::verify(
        &file,
        request.to_string().as_bytes(),
    ))
    .unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(
        answer["outcome"],
        serde_json::to_value(rec.outcome()).unwrap()
    );
    assert_eq!(answer["run_id"], hearken_replay::run_id(&file));
    let inspect: Value = serde_json::from_str(&hearken_rules::inspect(&file)).unwrap();
    assert_eq!(inspect["seed"], "99");
    assert_eq!(inspect["known"], true);
    assert_eq!(hearken_rules::rules(), r#"{"rules":["coin-dash"]}"#);

    for (request, error) in [
        (
            json!({ "rules": "coin-dash", "seed": "98", "context": { "seconds": 10 } }),
            "wrong-seed",
        ),
        (
            json!({ "rules": "coin-dash", "seed": "099", "context": { "seconds": 10 } }),
            "bad-request",
        ),
        (json!({ "rules": "coin-dash", "seed": 99 }), "bad-request"),
        (
            json!({ "rules": "coin-dash", "seed": "99", "context": { "seconds": 10, "x": 1 } }),
            "wrong-context",
        ),
    ] {
        let answer: Value = serde_json::from_str(&hearken_rules::verify(
            &file,
            request.to_string().as_bytes(),
        ))
        .unwrap();
        assert_eq!(answer["error"], error, "{request}");
    }
    let answer: Value = serde_json::from_str(&hearken_rules::verify(b"nope", b"{}")).unwrap();
    assert_eq!(answer["error"], "bad-request");
}

#[test]
fn a_build_id_is_none_without_a_release() {
    // Tests run without HEARKEN_RULES_ID baked in or set.
    if std::env::var("HEARKEN_RULES_ID").is_err() {
        assert_eq!(play(1, 1).to_run().build, BuildId::NONE);
    }
}

#[test]
fn a_context_too_large_is_refused_up_front() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Big(String);
    struct Huge;
    impl hearken_replay::Replayable for Huge {
        const RULES: &'static str = "huge";
        const MAX_TICKS: u64 = 1;
        type Input = Control;
        type Context = Big;
        type Outcome = ();
        fn new(_: u64, _: &Big) -> Self {
            Huge
        }
        fn step(&mut self, _: &[Control]) {}
        fn finished(&self) -> bool {
            true
        }
        fn outcome(&self) {}
    }
    let err = Recording::<Huge>::new(1, &Big("x".repeat(20_000))).err();
    assert_eq!(err.map(|e| e.code()), Some(FormatError::BadContext.code()));
}

/// Runs for the JavaScript host's tests, with the score native replay gave.
#[test]
fn writes_fixtures_for_the_javascript_host() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/hearken-replay-fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    let mut list = Vec::new();
    for (seed, seconds) in [(99u64, 10u32), (u64::MAX, 60), (12345, 30)] {
        let rec = play(seed, seconds);
        let mut run = rec.to_run();
        run.build = BuildId([0; 32]);
        let file = run.encode().unwrap();
        let name = format!("coin-dash-{seed}-{seconds}.hkrun");
        std::fs::write(dir.join(&name), &file).unwrap();
        list.push(json!({
            "file": name,
            "seed": seed.to_string(),
            "context": { "seconds": seconds },
            "outcome": serde_json::to_value(rec.outcome()).unwrap(),
            "run_id": hearken_replay::run_id(&file),
        }));
    }
    std::fs::write(
        dir.join("fixtures.json"),
        serde_json::to_string_pretty(&list).unwrap(),
    )
    .unwrap();
}
