# State machines in a Hearken game

A game is full of small state machines: the screen the player is on, the
phase a round is in, whose turn it is, whether a run has started. Kept
implicit, as booleans and timers, they are rebuilt from flags in every
handler, nothing records when they change, and a tester cannot ask where
the game is. Hearken keeps two of them explicit for every game and gives
a game the pieces to do the same for its own.

Why it matters to three people at once:

- **The player** hears each phase announced once, where it begins, and a
  status key that reads the phase instead of guessing it from flags.
- **The developer** writes one enum, one transition call and one `match`,
  and reads the session log to see every change next to what was said.
- **The agent** play testing the game can inspect the phase and the screen
  stack, and a recording's log shows every change timed against speech and
  sound.

## The screen stack

The stack is the one state machine every game already has. A screen is a
`State`; the stack changes only through `StackOp`s (`push`, `pop`,
`pop_quiet`, `pop_with`, `pop_to`, `replace`, `reset_to` on `Ctx`), queued
and applied after the handler that asked, so a screen never changes the
stack it is running on. What the revealed screen hears is a `Reveal`:
announce itself, stay quiet, or speak a result first.

Every change is logged under the `hearken.screens` target and kept in
`ctx.changes`:

```
[screen] start on title
[screen] push play over title
[screen] push pause over play
[screen] pop pause
[screen] play resumes quietly
[screen] reset to title
```

`App::stack_names()` lists the stack, `ctx.top_screen()` names the top,
and the agent server's `screen` tool prints both.

## Phases inside a screen

A play screen passes through phases. Name them:

```rust
use hearken::{Phase, Phases};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hall { Dark, Lit, Ending, Done }

impl Phase for Hall {
    fn name(self) -> &'static str {
        match self {
            Hall::Dark => "dark",
            Hall::Lit => "lit",
            Hall::Ending => "ending",
            Hall::Done => "done",
        }
    }
}
```

Keep one `Phases<Hall>` on the screen, made with `Phases::new(Hall::Dark)`,
and change it in one place. `set` returns true only when the phase
changed, so the line for the new phase is spoken there and nowhere else:

```rust
fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
    match (self.phases.get(), ctx.action_for(event)) {
        (Hall::Dark, Some(Action::Primary)) => {
            if self.phases.set(ctx, Hall::Lit) {
                ctx.say("Taper lit. Reach left or right.");
            }
        }
        (Hall::Lit, Some(Action::Left)) => { /* play */ }
        _ => {}
    }
}
```

A phase that ends by itself takes a timer. This replaces a screen's own
`ending: Option<f64>`:

```rust
// The last lamp is lit: two seconds for the chime, then the results.
if self.phases.set_for(ctx, Hall::Ending, 2.0) {
    ctx.say("The hall is lit.");
}

fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
    if self.phases.tick(dt) && self.phases.set(ctx, Hall::Done) {
        self.results(ctx);
    }
}
```

The tracker never picks the next phase: what follows a timed phase is the
screen's decision, in `update`, where it can speak.

Report the phase, so the agent server, `Headless::phase` and recordings can
read it:

```rust
fn phase(&self) -> Option<&'static str> {
    Some(self.phases.name())
}
```

Each change is logged under `hearken.phase` and kept in `ctx.changes`
beside the screen changes, as `[phase] play: lit -> ending`. Nothing in
that record is ever spoken.

## Reading the phase for the player

`resume`, the status key and the help all read `self.phases.get()`, never a
second set of flags. For the status key, `StatusKey` speaks layers: the
first press answers what is wrong and the one or two numbers the player
tracks, the next press reads the fuller picture.

```rust
Some(Action::Status) => {
    let layers = vec![
        self.danger_line(),            // "" when there is no danger
        format!("{} lamps out.", out),
        self.full_report(),
    ];
    self.status.speak(ctx, layers);
}
```

Empty layers are skipped; `hearken::text::sentences` joins the facts of one
layer, leaving out the ones that are absent.

## Rules that run on a server

Rules that implement `Replayable` are a stepped machine already: `new`,
`step`, `finished`, `outcome`. Keep their phase as an enum inside the rules
and derive `finished()` from it, so the screen reads one phase from the
rules and never keeps its own copy.

## Testing phases

`Headless` reads the record:

```rust
let mut h = Headless::new(data, Play::new());
h.clear();
h.press(Key::Space);
assert_eq!(h.phase(), Some("lit"));
assert_eq!(h.transitions(), ["play: dark -> lit"]);
h.press(Key::Escape);
assert_eq!(h.screens(), ["push pause over play"]);
```

`h.changes()` gives both kinds in order. A test that asserts the
transitions, and the one line spoken at each, pins the machine itself
rather than inferring it from speech.

## Checklist

When designing or building a screen:

1. Name every phase as an enum variant; a boolean that means "we are past
   X" is a phase.
2. Change the phase in one call to `set` or `set_for`, and speak the new
   phase's line inside that `if`.
3. Report it from `State::phase`.
4. Read it, never flags, in `update`, `resume`, the status key and help.
5. Assert the transitions in a `Headless` test.
6. Leave what the player hears exactly as it was when converting an
   existing screen.
