# Waiting for speech

A player with a slow speech rate hears each line later than a player with
a fast one. A countdown that runs while the game is still talking (a
customer's patience, a fuse, the time to answer) is shorter for them, by
however long the line took to say. Hearken can stop such a clock until the
line has been heard.

## Asking whether speech is playing

`ctx.speech_busy()` is true while either channel is still speaking: the
main channel (`say`, `say_queued`, `say_preview`) and the event channel
(`say_event`, `say_event_with`, while the game keeps its pacer, which is
the default).

Each line counts from an estimate of how long it takes to say, from its
length, the same estimate the event pacer uses (`PacerConfig::duration_s`).
An interrupting line starts the estimate over; a queued line adds to it;
`stop_speech` ends it. Blank lines count for nothing.

Once a game has asked, Hearken also asks the voices each frame, while
something was said in the last few seconds, whether they are still
speaking. A voice that can answer (NVDA 2026.3 and later, SAPI, OneCore)
corrects the estimate: a fast voice that has finished ends the wait
early, and a slow one still speaking past the estimate keeps it going for
as long as its reports say so (each report counts for a second, so the gap
between two never reads as quiet). A voice that cannot answer (JAWS, older NVDA) leaves the
estimate as it is. The game loop never waits for the answer. A game that
never asks makes no extra calls to the screen reader.

## A clock that waits

`SpeechHold` turns that into game time. Keep one on the screen, and pass
the countdown's time through it:

```rust
use hearken::SpeechHold;

struct Counter {
    patience: f64,
    hold: SpeechHold,
}

fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
    self.patience -= self.hold.dt(ctx, dt);
    if self.patience <= 0.0 {
        // The customer leaves.
    }
}
```

`hold.dt(ctx, dt)` returns `0.0` while speech is playing and `dt` once it
is quiet. `hold.held()` says whether the last call held the clock (for a
status line, say), and `hold.reset()` starts it fresh.

A hold lasts at most `max_hold_s` (20 seconds by default, counted in the
frame time passed to `dt`), so a game that keeps talking, or a screen
reader busy reading something else, cannot freeze the clock. A quiet gap
shorter than `rearm_quiet_s` (half a second by default) lets time run but
does not start the count over. After the maximum, the clock runs until
speech has been quiet for `rearm_quiet_s`, then holds again. Change them
with `SpeechHold::with_max_hold(10.0)` or by setting the fields.

The hold is a small state machine (see [state-machines.md](state-machines.md)):
`hold.state()` is a `SpeechHoldState`: `Flowing`, `Holding { for_s,
quiet_s }` or `Expired { quiet_s }`, and every change happens in
`SpeechHold::step`.

## When to use it

- A timer the player has to hear a line to beat: an order, a question,
  the instructions for the next move.
- A wait that starts with an announcement, such as a turn timer that
  begins with "Your turn."

## When not to

- Anything locked to music or a beat. A rhythm game's notes must arrive on
  time whatever is being said, so its clock never waits.
- Real-time danger that the player can hear coming from sound alone. If a
  car is approaching, it keeps approaching while a line is read.
- A clock that is meant to run while the game talks, where hearing the
  line quickly is part of the challenge.

## Testing

`Headless` uses a capture voice that cannot report, so the estimate alone
drives `speech_busy` and `SpeechHold`, on the same clock as the pacer:
`h.frame(dt)` moves both. To test a voice that reports, script it with
`h.speech.borrow_mut().set_voice_speaking(Some(true))`, and give its reports
an age with `set_voice_report_age`.
