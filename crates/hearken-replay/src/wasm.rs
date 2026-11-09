//! A game's rules as a WebAssembly module a server runs: what
//! [`export_rules!`](crate::export_rules) puts in the module, and the JSON
//! it answers with.
//!
//! The module's exports (ABI version 2). Pointers and lengths are `u32`;
//! an answer is a `u64` holding its pointer in the high half and its length
//! in the low half, and the host frees it with `hearken_free`.
//!
//! | Export | What it does |
//! |--------|--------------|
//! | `memory` | The module's memory. |
//! | `hearken_abi() -> u32` | The ABI version, 2. |
//! | `hearken_alloc(len) -> ptr` | Room for `len` bytes the host writes into. |
//! | `hearken_free(ptr, len)` | Gives back what `hearken_alloc` or an answer took. |
//! | `hearken_rules() -> u64` | JSON: the rules this module has, `{"rules":["belfry"]}`. |
//! | `hearken_inspect(run, run_len) -> u64` | JSON: what a run file says about itself, without replaying it. |
//! | `hearken_verify(run, run_len, req, req_len) -> u64` | JSON: the run checked against the request and replayed. |
//! | `hearken_play(req, req_len) -> u64` | JSON: a match's turns, each stamped with its seat, built into a run and replayed, finished or not (ABI 2). |
//!
//! The request is `{"rules":"belfry","seed":"12345","context":{...}}`: the
//! seed as a decimal string (or a number below 2^53), the context as the
//! JSON the rules take. Answers are `{"ok":true,...}` or
//! `{"ok":false,"error":"<code>"}`.
//!
//! A play request is `{"rules":"pebbles","seed":"123","context":{...},
//! "turns":[{"seat":0,"inputs":[[1,2]]},{"seat":1,"left":true}],
//! "finish":false}`: one tick per turn, each input's code stamped with the
//! turn's seat ([`crate::seats`]), `left` for a seat that left the match.
//! With `finish` the rules must have ended; without it the answer says
//! whether they have. See `docs/online.md`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::format::{canonical_json, run_id, BuildId, FormatError, Input, Run, MAX_INPUTS};
use crate::rules::{progress, verify, Expected, Replayable, VerifyError};
use crate::seats::{stamp, Seat, LEFT_CODE, MAX_SEATED_CODE};

/// The module ABI this crate exports. ABI 1 lacked `hearken_play`.
pub const ABI: u32 = 2;

/// The longest request read, in bytes.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// One set of rules a module carries: its name, and how to verify a run of
/// it. Made by [`export_rules!`](crate::export_rules).
pub struct Entry {
    pub rules: &'static str,
    pub verify: fn(&Run, &Expected) -> Result<Value, VerifyError>,
    pub play: fn(&PlayRequest) -> Result<Value, VerifyError>,
}

/// One turn of a match as a server stores it: the seat that moved and its
/// inputs, or a seat that left.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PlayTurn {
    pub seat: u8,
    #[serde(default)]
    pub inputs: Vec<(u16, i32)>,
    #[serde(default)]
    pub left: bool,
}

/// What a server asks `hearken_play`: the match so far.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PlayRequest {
    pub rules: String,
    pub seed: Value,
    pub context: Value,
    #[serde(default)]
    pub turns: Vec<PlayTurn>,
    /// Whether the rules must have ended.
    #[serde(default)]
    pub finish: bool,
}

impl PlayRequest {
    /// The turns as one run for `R`: the context as the rules write it,
    /// one tick per turn, every code stamped with its seat.
    pub fn to_run<R: Replayable>(&self) -> Result<Run, VerifyError> {
        let seed = seed_of(&self.seed).ok_or(VerifyError::WrongSeed)?;
        let read: R::Context =
            R::Context::deserialize(&self.context).map_err(|_| VerifyError::BadContext)?;
        let written = serde_json::to_value(&read).map_err(|_| VerifyError::BadContext)?;
        let context = canonical_json(&written);
        if context.len() > crate::format::MAX_CONTEXT_LEN {
            return Err(VerifyError::BadContext);
        }
        if self.turns.len() as u64 > R::MAX_TICKS {
            return Err(VerifyError::TooLong);
        }
        let mut inputs = Vec::new();
        for (tick, turn) in self.turns.iter().enumerate() {
            let seat = Seat::new(turn.seat).ok_or(VerifyError::BadInput)?;
            if turn.left {
                inputs.push(Input {
                    tick: tick as u64,
                    code: stamp(seat, LEFT_CODE),
                    value: 0,
                });
                continue;
            }
            for &(code, value) in &turn.inputs {
                if code > MAX_SEATED_CODE {
                    return Err(VerifyError::BadInput);
                }
                inputs.push(Input {
                    tick: tick as u64,
                    code: stamp(seat, code),
                    value,
                });
            }
            if inputs.len() > MAX_INPUTS {
                return Err(VerifyError::Format(FormatError::BadInputs));
            }
        }
        Ok(Run {
            build: BuildId::NONE,
            rules: self.rules.clone(),
            seed,
            context,
            ticks: self.turns.len() as u64,
            inputs,
        })
    }
}

/// `progress::<R>` over a play request's turns, as JSON, for an [`Entry`].
pub fn play_entry<R: Replayable>(request: &PlayRequest) -> Result<Value, VerifyError> {
    let run = request.to_run::<R>()?;
    let played = progress::<R>(&run)?;
    if request.finish && !played.finished {
        return Err(VerifyError::Unfinished);
    }
    let outcome = serde_json::to_value(&played.outcome).map_err(|_| VerifyError::BadContext)?;
    Ok(json!({
        "ok": true,
        "rules": played.rules,
        "seed": played.seed.to_string(),
        "ticks": played.ticks,
        "finished": played.finished,
        "outcome": outcome,
    }))
}

/// `verify::<R>` with its outcome as JSON, for an [`Entry`].
pub fn verify_entry<R: Replayable>(run: &Run, expected: &Expected) -> Result<Value, VerifyError> {
    let verified = verify::<R>(run, expected)?;
    let outcome = serde_json::to_value(&verified.outcome).map_err(|_| VerifyError::BadContext)?;
    Ok(json!({
        "ok": true,
        "run_id": verified.run_id,
        "rules": verified.rules,
        "build": verified.build.to_hex(),
        "seed": verified.seed.to_string(),
        "ticks": verified.ticks,
        "outcome": outcome,
    }))
}

fn refusal(code: &str) -> String {
    json!({ "ok": false, "error": code }).to_string()
}

/// `{"rules":[...]}`: the rules in `entries`.
pub fn rules_json(entries: &[Entry]) -> String {
    let names: Vec<&str> = entries.iter().map(|e| e.rules).collect();
    json!({ "rules": names }).to_string()
}

/// What the run file `file` says about itself, as JSON: its ID, build,
/// rules (and whether this module has them), seed, context, ticks and how
/// many inputs it holds. Nothing is replayed, so none of it is verified.
pub fn inspect_json(file: &[u8], entries: &[Entry]) -> String {
    let run = match Run::decode(file) {
        Ok(run) => run,
        Err(e) => return refusal(e.code()),
    };
    let context: Value = serde_json::from_str(&run.context).unwrap_or(Value::Null);
    json!({
        "ok": true,
        "run_id": run_id(file),
        "build": run.build.to_hex(),
        "rules": run.rules,
        "known": entries.iter().any(|e| e.rules == run.rules),
        "seed": run.seed.to_string(),
        "context": context,
        "ticks": run.ticks,
        "inputs": run.inputs.len(),
    })
    .to_string()
}

/// The request's seed: a decimal string, or a whole JSON number below 2^53.
fn seed_of(value: &Value) -> Option<u64> {
    match value {
        Value::String(s)
            if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            // One spelling per seed: no leading zeros.
            if s.len() > 1 && s.starts_with('0') {
                return None;
            }
            s.parse().ok()
        }
        Value::Number(n) => n.as_u64().filter(|&n| n < (1u64 << 53)),
        _ => None,
    }
}

/// The request as an [`Expected`]; `None` when it is not one.
pub fn expected_of(request: &[u8]) -> Option<Expected> {
    if request.len() > MAX_REQUEST_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(request).ok()?;
    let rules = value.get("rules")?.as_str()?.to_string();
    let seed = seed_of(value.get("seed")?)?;
    let context = value.get("context")?.clone();
    Some(Expected {
        rules,
        seed,
        context,
    })
}

/// The run file `file` checked against `request` and replayed by the
/// matching entry, as JSON.
pub fn verify_json(file: &[u8], request: &[u8], entries: &[Entry]) -> String {
    if !entries_valid(entries) {
        return refusal("bad-module");
    }
    let Some(expected) = expected_of(request) else {
        return refusal("bad-request");
    };
    let run = match Run::decode(file) {
        Ok(run) => run,
        Err(e) => return refusal(e.code()),
    };
    let Some(entry) = entries.iter().find(|e| e.rules == run.rules) else {
        return refusal(VerifyError::WrongRules.code());
    };
    match (entry.verify)(&run, &expected) {
        Ok(answer) => answer.to_string(),
        Err(e) => refusal(e.code()),
    }
}

/// A match's turns (`request`, a [`PlayRequest`] as JSON) built into a run
/// and replayed by the matching entry, as JSON.
pub fn play_json(request: &[u8], entries: &[Entry]) -> String {
    if !entries_valid(entries) {
        return refusal("bad-module");
    }
    if request.len() > MAX_REQUEST_BYTES.max(crate::format::MAX_RUN_BYTES) {
        return refusal("bad-request");
    }
    let Ok(request) = serde_json::from_slice::<PlayRequest>(request) else {
        return refusal("bad-request");
    };
    let Some(entry) = entries.iter().find(|e| e.rules == request.rules) else {
        return refusal(VerifyError::WrongRules.code());
    };
    match (entry.play)(&request) {
        Ok(answer) => answer.to_string(),
        Err(e) => refusal(e.code()),
    }
}

/// Whether `entries` name each set of rules once, with valid names.
pub fn entries_valid(entries: &[Entry]) -> bool {
    entries.iter().enumerate().all(|(i, e)| {
        crate::format::valid_rules_name(e.rules) && entries[..i].iter().all(|o| o.rules != e.rules)
    })
}

#[doc(hidden)]
pub mod abi {
    //! The pointer glue behind the exports. A module's exports call these;
    //! nothing else should.

    /// Room for `len` bytes, for the host to write into.
    pub fn alloc(len: u32) -> u32 {
        let boxed: Box<[u8]> = vec![0u8; len as usize].into_boxed_slice();
        Box::into_raw(boxed) as *mut u8 as usize as u32
    }

    /// Frees what `alloc` or an answer took.
    ///
    /// # Safety
    /// `ptr` and `len` must be exactly what `alloc` returned, or an
    /// answer's halves, not yet freed.
    pub unsafe fn free(ptr: u32, len: u32) {
        if ptr == 0 {
            return;
        }
        let slice = core::ptr::slice_from_raw_parts_mut(ptr as usize as *mut u8, len as usize);
        drop(Box::from_raw(slice));
    }

    /// The bytes the host wrote at `ptr`.
    ///
    /// # Safety
    /// `ptr` and `len` must be a live `alloc` the host filled.
    pub unsafe fn bytes<'a>(ptr: u32, len: u32) -> &'a [u8] {
        if len == 0 {
            return &[];
        }
        core::slice::from_raw_parts(ptr as usize as *const u8, len as usize)
    }

    /// Hands `answer` to the host: its pointer and length in one `u64`.
    pub fn answer(answer: String) -> u64 {
        let boxed = answer.into_bytes().into_boxed_slice();
        let len = boxed.len() as u64;
        let ptr = Box::into_raw(boxed) as *mut u8 as usize as u64;
        (ptr << 32) | len
    }
}

/// Exports a game's rules for a server: put it once in the crate
/// `hearken release` builds for WebAssembly (`rules = "<crate>"` in the
/// game's `[package.metadata.hearken]`), naming every [`Replayable`] the
/// game puts online.
///
/// ```ignore
/// hearken_replay::export_rules!(belfry::Peal, sonar_dive::Dive);
/// ```
///
/// It also adds `hearken_rules::verify(run, request)`,
/// `hearken_rules::inspect(run)` and `hearken_rules::play(request)` to the
/// crate on every target, so a test can ask the same questions natively
/// that a server asks the module.
#[macro_export]
macro_rules! export_rules {
    ($($rules:ty),+ $(,)?) => {
        /// The rules this crate exports, as the server sees them.
        pub mod hearken_rules {
            #[allow(unused_imports)]
            use super::*;

            /// One entry for each set of rules exported.
            pub fn entries() -> ::std::vec::Vec<$crate::wasm::Entry> {
                ::std::vec![$(
                    $crate::wasm::Entry {
                        rules: <$rules as $crate::Replayable>::RULES,
                        verify: $crate::wasm::verify_entry::<$rules>,
                        play: $crate::wasm::play_entry::<$rules>,
                    }
                ),+]
            }

            /// The module's `hearken_verify`, natively.
            pub fn verify(run: &[u8], request: &[u8]) -> ::std::string::String {
                $crate::wasm::verify_json(run, request, &entries())
            }

            /// The module's `hearken_inspect`, natively.
            pub fn inspect(run: &[u8]) -> ::std::string::String {
                $crate::wasm::inspect_json(run, &entries())
            }

            /// The module's `hearken_play`, natively.
            pub fn play(request: &[u8]) -> ::std::string::String {
                $crate::wasm::play_json(request, &entries())
            }

            /// The module's `hearken_rules`, natively.
            pub fn rules() -> ::std::string::String {
                $crate::wasm::rules_json(&entries())
            }

            #[cfg(target_arch = "wasm32")]
            mod exports {
                #[unsafe(no_mangle)]
                pub extern "C" fn hearken_abi() -> u32 {
                    $crate::wasm::ABI
                }

                #[unsafe(no_mangle)]
                pub extern "C" fn hearken_alloc(len: u32) -> u32 {
                    $crate::wasm::abi::alloc(len)
                }

                /// # Safety
                /// Only the host calls it, with what `hearken_alloc` or an
                /// answer gave.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn hearken_free(ptr: u32, len: u32) {
                    $crate::wasm::abi::free(ptr, len)
                }

                #[unsafe(no_mangle)]
                pub extern "C" fn hearken_rules() -> u64 {
                    $crate::wasm::abi::answer(super::rules())
                }

                /// # Safety
                /// Only the host calls it, with a filled `hearken_alloc`.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn hearken_inspect(run: u32, run_len: u32) -> u64 {
                    let run = $crate::wasm::abi::bytes(run, run_len);
                    $crate::wasm::abi::answer(super::inspect(run))
                }

                /// # Safety
                /// Only the host calls it, with filled `hearken_alloc`s.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn hearken_verify(
                    run: u32,
                    run_len: u32,
                    request: u32,
                    request_len: u32,
                ) -> u64 {
                    let run = $crate::wasm::abi::bytes(run, run_len);
                    let request = $crate::wasm::abi::bytes(request, request_len);
                    $crate::wasm::abi::answer(super::verify(run, request))
                }

                /// # Safety
                /// Only the host calls it, with a filled `hearken_alloc`.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn hearken_play(request: u32, request_len: u32) -> u64 {
                    let request = $crate::wasm::abi::bytes(request, request_len);
                    $crate::wasm::abi::answer(super::play(request))
                }
            }
        }
    };
}
