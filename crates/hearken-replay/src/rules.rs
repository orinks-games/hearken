//! A game's rules as something a run can be replayed through: the
//! [`Replayable`] trait, the [`Recording`] that plays and records at once,
//! and [`replay`] and [`verify`], which play a run again.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;

use crate::format::{canonical_json, valid_rules_name, BuildId, FormatError, Input, Run};

/// One of a game's inputs as the run file stores it: a code and a value.
/// `from_code` must undo `to_code` exactly, and give `None` for any pair
/// `to_code` never makes, so every input has one encoding.
pub trait InputCode: Sized {
    fn to_code(&self) -> (u16, i32);
    fn from_code(code: u16, value: i32) -> Option<Self>;
}

/// A game's rules, as plain code with no screen, sound or clock, so the
/// same code runs in the game and, built for WebAssembly, on a server.
///
/// The rules must be deterministic: the same seed, context and inputs give
/// the same outcome on every platform. That means no wall-clock time, no
/// randomness except from `seed`, no iteration over a `HashMap`, and no
/// floating-point maths that differs between platforms (`sin`, `exp`,
/// `powf` and friends can) in anything that decides the outcome.
///
/// The rules are also the referee. A replayed run's inputs come from a
/// file anyone can write, so `step` must treat every input as possible at
/// every tick and ignore one the game would not allow then, rather than
/// trusting the menus and screens to have stopped it.
pub trait Replayable: Sized {
    /// The game's name for these rules, such as `"belfry"`: 1 to 64 of
    /// `a-z`, `0-9`, `-` and `_`. A run names the rules it ran.
    const RULES: &'static str;
    /// The most steps a run may take. A server refuses a longer run before
    /// replaying it, so this bounds the work one run can cost.
    const MAX_TICKS: u64;

    /// One input, such as the game's `Action` pressed or released.
    type Input: InputCode;
    /// What the rules start from besides the seed: a mode, a difficulty,
    /// a level. `()` when there is nothing.
    type Context: Serialize + DeserializeOwned;
    /// What a run comes to: a score, a time, items found, a route. A
    /// server sends it to the game's handler as JSON. Numbers above 2^53
    /// lose precision in JavaScript, so send large ones as strings.
    type Outcome: Serialize;

    /// The rules at the start of a run.
    fn new(seed: u64, context: &Self::Context) -> Self;
    /// One step, with the inputs that arrived on it (often none).
    fn step(&mut self, inputs: &[Self::Input]);
    /// Whether the run is over. A run ends on the step this turns true.
    fn finished(&self) -> bool;
    /// What the run has come to so far.
    fn outcome(&self) -> Self::Outcome;
}

/// The rules running live, with every step recorded, so the game plays
/// exactly what it will later submit. The rules are only reachable through
/// [`Recording::sim`], read-only: a change made any other way would not be
/// in the recording.
///
/// ```ignore
/// let mut run = Recording::<Dash>::new(seed, &context)?;
/// clock.advance(dt, |_| { run.step(&inputs.drain(..).collect::<Vec<_>>()); });
/// if run.finished() { ctx.online.submit("daily", &run.to_run())?; }
/// ```
pub struct Recording<R: Replayable> {
    sim: R,
    run: Run,
}

impl<R: Replayable> Recording<R> {
    /// A new run of `R` from `seed` and `context`, made by this program's
    /// rules build ([`BuildId::this_build`]).
    pub fn new(seed: u64, context: &R::Context) -> Result<Self, FormatError> {
        if !valid_rules_name(R::RULES) {
            return Err(FormatError::BadRules);
        }
        let value = serde_json::to_value(context).map_err(|_| FormatError::BadContext)?;
        let context_json = canonical_json(&value);
        if context_json.len() > crate::format::MAX_CONTEXT_LEN {
            return Err(FormatError::BadContext);
        }
        // The rules start from the context as the run file will carry it,
        // so a field the file drops (`#[serde(skip)]`) or changes is dropped
        // or changed in play too, not only in the replay.
        let carried: R::Context =
            serde_json::from_str(&context_json).map_err(|_| FormatError::BadContext)?;
        Ok(Recording {
            sim: R::new(seed, &carried),
            run: Run {
                build: BuildId::this_build(),
                rules: R::RULES.to_string(),
                seed,
                context: context_json,
                ticks: 0,
                inputs: Vec::new(),
            },
        })
    }

    /// The rules as they stand, to read from. Rules that change through a
    /// shared reference (a `Cell` or `RefCell` inside) must not be changed
    /// here: that change would not be in the recording.
    pub fn sim(&self) -> &R {
        &self.sim
    }

    /// Steps taken so far.
    pub fn tick(&self) -> u64 {
        self.run.ticks
    }

    /// Whether the run is over.
    pub fn finished(&self) -> bool {
        self.sim.finished()
    }

    /// The outcome so far.
    pub fn outcome(&self) -> R::Outcome {
        self.sim.outcome()
    }

    /// Runs one step with `inputs` and records it. Does nothing once the
    /// run is over, and returns whether the step ran.
    ///
    /// The rules are given each input as the run file will carry it (its
    /// code read back with `from_code`), so play and replay see the same
    /// inputs. An input whose code does not read back is a bug in the
    /// game's `InputCode`: debug builds panic, and others leave it out of
    /// both the step and the record.
    pub fn step(&mut self, inputs: &[R::Input]) -> bool {
        if self.sim.finished() {
            return false;
        }
        let tick = self.run.ticks;
        let mut carried = Vec::with_capacity(inputs.len());
        for input in inputs {
            let (code, value) = input.to_code();
            let Some(read) =
                R::Input::from_code(code, value).filter(|r| r.to_code() == (code, value))
            else {
                debug_assert!(
                    false,
                    "an input's code ({code}, {value}) does not read back"
                );
                continue;
            };
            self.run.inputs.push(Input { tick, code, value });
            carried.push(read);
        }
        self.sim.step(&carried);
        self.run.ticks += 1;
        true
    }

    /// Whether the run has outgrown what a server replays: more steps than
    /// `R::MAX_TICKS`, or more inputs than a run file holds. A game can end
    /// such a run, or tell the player it will not count, instead of finding
    /// out when the server refuses it.
    pub fn too_long(&self) -> bool {
        self.run.ticks > R::MAX_TICKS || self.run.inputs.len() > crate::format::MAX_INPUTS
    }

    /// The run so far, to submit or save.
    pub fn to_run(&self) -> Run {
        self.run.clone()
    }

    /// The run, giving up the rules.
    pub fn into_run(self) -> Run {
        self.run
    }
}

/// What a server requires of a run before it counts: the rules it must
/// have run, and the seed and context it must have started from. The
/// server's handler supplies these (a challenge's seed from its date, say),
/// never the run.
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub rules: String,
    pub seed: u64,
    pub context: serde_json::Value,
}

/// A run that replayed, and what it came to.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified<O> {
    /// The run's ID ([`Run::id`]).
    pub run_id: String,
    pub rules: String,
    pub build: BuildId,
    pub seed: u64,
    pub ticks: u64,
    /// The outcome the replay computed.
    pub outcome: O,
}

/// Why a run did not replay or verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// The run file did not read.
    Format(FormatError),
    /// The run names other rules than these, or than the server expects.
    WrongRules,
    /// The run started from another seed than the server expects.
    WrongSeed,
    /// The run started from another context than the server expects.
    WrongContext,
    /// The run's context is not one these rules take.
    BadContext,
    /// The run is longer than [`Replayable::MAX_TICKS`].
    TooLong,
    /// An input these rules have no meaning for.
    BadInput,
    /// The inputs ran out before the run was over.
    Unfinished,
    /// The run kept going after it was over.
    PastTheEnd,
}

impl VerifyError {
    /// A short stable name, for logs and a server's reply.
    pub fn code(&self) -> &'static str {
        match self {
            VerifyError::Format(e) => e.code(),
            VerifyError::WrongRules => "wrong-rules",
            VerifyError::WrongSeed => "wrong-seed",
            VerifyError::WrongContext => "wrong-context",
            VerifyError::BadContext => "bad-context",
            VerifyError::TooLong => "too-long",
            VerifyError::BadInput => "bad-input",
            VerifyError::Unfinished => "unfinished",
            VerifyError::PastTheEnd => "past-the-end",
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for VerifyError {}

impl From<FormatError> for VerifyError {
    fn from(e: FormatError) -> Self {
        VerifyError::Format(e)
    }
}

/// Plays `run` again through `R` from its own seed and context, and gives
/// back the rules where it ended. Refuses a run of other rules, one longer
/// than `R::MAX_TICKS`, one with an input `R` has no meaning for, one that
/// ends before its inputs do, and one whose inputs end before it does.
pub fn replay<R: Replayable>(run: &Run) -> Result<R, VerifyError> {
    if run.rules != R::RULES {
        return Err(VerifyError::WrongRules);
    }
    if run.ticks > R::MAX_TICKS {
        return Err(VerifyError::TooLong);
    }
    if !crate::format::is_canonical_json(&run.context) {
        return Err(VerifyError::Format(FormatError::BadContext));
    }
    let context: R::Context =
        serde_json::from_str(&run.context).map_err(|_| VerifyError::BadContext)?;
    // Every input is checked before the first step, so a bad one late in a
    // long run costs nothing to refuse.
    let mut decoded = Vec::with_capacity(run.inputs.len());
    let mut last = 0u64;
    for input in &run.inputs {
        if input.tick < last || input.tick >= run.ticks {
            return Err(VerifyError::Format(FormatError::BadInputs));
        }
        last = input.tick;
        let value = R::Input::from_code(input.code, input.value).ok_or(VerifyError::BadInput)?;
        if value.to_code() != (input.code, input.value) {
            return Err(VerifyError::BadInput);
        }
        decoded.push(value);
    }
    let mut sim = R::new(run.seed, &context);
    let mut next = 0;
    for tick in 0..run.ticks {
        if sim.finished() {
            return Err(VerifyError::PastTheEnd);
        }
        let start = next;
        while next < run.inputs.len() && run.inputs[next].tick == tick {
            next += 1;
        }
        sim.step(&decoded[start..next]);
    }
    if !sim.finished() {
        return Err(VerifyError::Unfinished);
    }
    Ok(sim)
}

/// Whether the run's context and the one a server expects mean the same to
/// `R`: each is read as `R::Context` and written back, so `1` and `1.0` for
/// a float, `0.1` for an `f32` and a missing `Option` field all match the
/// form a recording wrote. Neither side's text is trusted to be in that form.
fn same_context<R: Replayable>(run: &str, expected: &serde_json::Value) -> bool {
    if run == canonical_json(expected) {
        return true;
    }
    let Ok(ours) = serde_json::from_str::<R::Context>(run) else {
        return false;
    };
    let Ok(theirs) = R::Context::deserialize(expected) else {
        return false;
    };
    match (serde_json::to_value(&ours), serde_json::to_value(&theirs)) {
        (Ok(a), Ok(b)) => canonical_json(&a) == canonical_json(&b) && keeps(expected, &b),
        _ => false,
    }
}

/// Whether `read` (the expected context as `R` read it back) still has every
/// field the server asked for. A field the rules do not know is dropped on
/// reading, and a server that asks for one is asking for something no run
/// can show it played, so that is a mismatch, not a match.
fn keeps(asked: &serde_json::Value, read: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (asked, read) {
        (Value::Object(a), Value::Object(r)) => a.iter().all(|(k, v)| match r.get(k) {
            Some(rv) => keeps(v, rv),
            None => v.is_null(),
        }),
        (Value::Object(_), _) => false,
        (Value::Array(a), Value::Array(r)) => {
            a.len() == r.len() && a.iter().zip(r).all(|(a, r)| keeps(a, r))
        }
        (Value::Array(_), _) => false,
        _ => true,
    }
}

/// Checks that `run` is what `expected` asks for, replays it, and gives the
/// outcome the replay computed. This is the whole of what a server trusts:
/// nothing the run says about its own result is read.
pub fn verify<R: Replayable>(
    run: &Run,
    expected: &Expected,
) -> Result<Verified<R::Outcome>, VerifyError> {
    if run.rules != expected.rules || run.rules != R::RULES {
        return Err(VerifyError::WrongRules);
    }
    if run.seed != expected.seed {
        return Err(VerifyError::WrongSeed);
    }
    if !same_context::<R>(&run.context, &expected.context) {
        return Err(VerifyError::WrongContext);
    }
    let run_id = run.id()?;
    let sim = replay::<R>(run)?;
    Ok(Verified {
        run_id,
        rules: run.rules.clone(),
        build: run.build,
        seed: run.seed,
        ticks: run.ticks,
        outcome: sim.outcome(),
    })
}

/// Replays `recording`'s run and checks it comes to the same outcome the
/// live rules did: the determinism check a game's tests run before its
/// rules go online. Panics, saying where, when it does not.
pub fn assert_replays<R>(recording: &Recording<R>)
where
    R: Replayable,
    R::Outcome: PartialEq + std::fmt::Debug,
{
    let run = recording.to_run();
    let file = run.encode().expect("the run encodes");
    let read = Run::decode(&file).expect("the run reads back");
    assert_eq!(read, run, "the run changed on its way through the file");
    let replayed = replay::<R>(&read).unwrap_or_else(|e| panic!("the run did not replay: {e}"));
    assert_eq!(
        replayed.outcome(),
        recording.outcome(),
        "the replay came to another outcome than the live run"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize)]
    struct Setup {
        speed: f64,
        slow: f32,
        mode: Option<String>,
    }

    struct Tap;
    impl InputCode for Tap {
        fn to_code(&self) -> (u16, i32) {
            (0, 0)
        }
        fn from_code(code: u16, value: i32) -> Option<Self> {
            (code == 0 && value == 0).then_some(Tap)
        }
    }

    struct Once(bool);
    impl Replayable for Once {
        const RULES: &'static str = "once";
        const MAX_TICKS: u64 = 10;
        type Input = Tap;
        type Context = Setup;
        type Outcome = u8;
        fn new(_: u64, _: &Setup) -> Self {
            Once(false)
        }
        fn step(&mut self, _: &[Tap]) {
            self.0 = true;
        }
        fn finished(&self) -> bool {
            self.0
        }
        fn outcome(&self) -> u8 {
            1
        }
    }

    #[test]
    fn a_context_matches_by_what_it_means() {
        let setup = Setup {
            speed: 1.0,
            slow: 0.1,
            mode: None,
        };
        let mut rec = Recording::<Once>::new(5, &setup).unwrap();
        rec.step(&[]);
        let run = rec.into_run();
        // As a JavaScript handler would write it: 1 not 1.0, 0.1 as typed,
        // and no key for the missing mode.
        let expect = |context| Expected {
            rules: "once".into(),
            seed: 5,
            context,
        };
        let js = serde_json::json!({ "speed": 1, "slow": 0.1 });
        assert_eq!(verify::<Once>(&run, &expect(js)).unwrap().outcome, 1);
        let other = serde_json::json!({ "speed": 2, "slow": 0.1 });
        assert_eq!(
            verify::<Once>(&run, &expect(other)),
            Err(VerifyError::WrongContext)
        );
        let extra = serde_json::json!({ "speed": 1, "slow": 0.1, "event": "fair" });
        assert_eq!(
            verify::<Once>(&run, &expect(extra)),
            Err(VerifyError::WrongContext)
        );
        let junk = serde_json::json!({ "speed": "fast" });
        assert_eq!(
            verify::<Once>(&run, &expect(junk)),
            Err(VerifyError::WrongContext)
        );
    }
}
