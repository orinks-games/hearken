//! The voice's own reports correcting the projection (`observe_voice`).
use super::*;
use std::cell::Cell;
use std::rc::Rc;

fn make_pacer() -> (EventSpeechPacer, Rc<Cell<f64>>) {
    let now = Rc::new(Cell::new(100.0));
    let clock = Rc::clone(&now);
    let pacer = EventSpeechPacer::with_clock(PacerConfig::default(), Box::new(move || clock.get()));
    (pacer, now)
}

fn advance(now: &Rc<Cell<f64>>, seconds: f64) {
    now.set(now.get() + seconds);
}

const LINE: &str = "Speed limit drops to 55 miles per hour in half a mile.";

#[test]
fn test_silence_after_the_grace_ends_the_projection() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush(LINE, EventPriority::Ambient, Protect::Yes, None);
    advance(&now, 1.0);
    assert!(pacer.busy());
    pacer.observe_voice(false, 0.0);
    assert!(!pacer.busy());
}

#[test]
fn test_silence_inside_the_grace_is_ignored() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush(LINE, EventPriority::Ambient, Protect::Yes, None);
    advance(&now, 0.1);
    pacer.observe_voice(false, 0.0);
    assert!(pacer.busy(), "the voice may not have started the line yet");
}

#[test]
fn test_an_old_silent_report_from_before_the_line_is_ignored() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush(LINE, EventPriority::Ambient, Protect::Yes, None);
    advance(&now, 1.0);
    // Asked 0.9 s ago: 0.1 s after the line, inside the grace.
    pacer.observe_voice(false, 0.9);
    assert!(pacer.busy());
}

#[test]
fn test_speaking_past_the_estimate_holds_the_channel_busy() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush("Short.", EventPriority::Ambient, Protect::Yes, None);
    advance(&now, 2.0);
    assert!(!pacer.busy());
    assert!(pacer.wants_voice_report());
    pacer.observe_voice(true, 0.0);
    assert!(pacer.busy());
    advance(&now, pacer.config().voice_hold_s + 0.01);
    assert!(
        !pacer.busy(),
        "the hold lasts only until the next report is due"
    );
}

#[test]
fn test_speaking_while_the_estimate_runs_changes_nothing() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush(LINE, EventPriority::Ambient, Protect::Yes, None);
    advance(&now, 1.0);
    pacer.observe_voice(true, 0.0);
    // The line is ~4.5 s by estimate; the report does not shorten it.
    advance(&now, 2.0);
    assert!(pacer.busy());
}

#[test]
fn test_no_reports_wanted_once_quiet_and_lingered() {
    let (mut pacer, now) = make_pacer();
    assert!(!pacer.wants_voice_report(), "nothing was ever said");
    pacer.should_flush("Short.", EventPriority::Ambient, Protect::Yes, None);
    assert!(pacer.wants_voice_report());
    advance(&now, 10.0);
    assert!(!pacer.wants_voice_report());
}

#[test]
fn test_a_finished_line_is_not_rescued_by_a_later_interrupt() {
    let (mut pacer, now) = make_pacer();
    pacer.note_queued(LINE, EventPriority::Important, Protect::Yes, None);
    advance(&now, 0.6);
    pacer.observe_voice(false, 0.0);
    let cut = pacer.note_interrupt("Brake now.", EventPriority::Critical, Protect::Yes, None);
    assert_eq!(cut, None, "the voice said it had finished the line");
}

#[test]
fn test_silence_completes_a_pending_delivery() {
    let (mut pacer, now) = make_pacer();
    pacer.should_flush(LINE, EventPriority::Important, Protect::Yes, None);
    pacer.track_delivery("warned", LINE);
    advance(&now, 1.0);
    assert!(pacer.delivery_pending());
    pacer.observe_voice(false, 0.0);
    assert_eq!(
        pacer.delivery_status("warned"),
        Some(DeliveryStatus::Completed)
    );
}

#[test]
fn test_staleness_follows_the_real_voice() {
    let (mut pacer, now) = make_pacer();
    // A long backlog by estimate, which a fast voice already finished.
    for n in 0..3 {
        pacer.should_flush(
            &format!("{LINE} {n}"),
            EventPriority::Ambient,
            Protect::Yes,
            None,
        );
    }
    advance(&now, 1.0);
    pacer.observe_voice(false, 0.0);
    assert!(!pacer.would_start_stale("Next.", EventPriority::Important));
}

#[test]
fn test_quiet_needs_a_fresh_silent_report_after_the_last_line() {
    let (mut pacer, now) = make_pacer();
    assert!(!pacer.voice_reporting());
    assert!(!pacer.voice_quiet(), "no report yet");
    pacer.observe_voice(false, 0.0);
    assert!(pacer.voice_quiet());
    pacer.should_flush(LINE, EventPriority::Ambient, Protect::Yes, None);
    assert!(!pacer.voice_quiet(), "a line was just handed over");
    advance(&now, 0.1);
    pacer.observe_voice(false, 0.0);
    assert!(!pacer.voice_quiet(), "inside the start grace");
    advance(&now, 0.5);
    pacer.observe_voice(true, 0.0);
    assert!(!pacer.voice_quiet());
    advance(&now, 1.0);
    pacer.observe_voice(false, 0.0);
    assert!(pacer.voice_quiet());
    advance(&now, 1.5);
    assert!(!pacer.voice_reporting(), "the report went stale");
    assert!(!pacer.voice_quiet());
}

#[test]
fn test_an_older_report_never_overrides_a_newer_one() {
    let (mut pacer, _now) = make_pacer();
    pacer.observe_voice(true, 0.0);
    pacer.observe_voice(false, 0.5);
    assert!(!pacer.voice_quiet());
}
