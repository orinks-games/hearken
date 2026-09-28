# M1 follow-ups

Open items recorded when M1 closed (2026-09-26), updated when M1b and M2 closed.
Each is small; pick them up in whichever milestone touches the code first.

## Carry into M2b

- Occlusion is answered one frame late, so a very short one-shot behind a wall
  starts unoccluded.
- A pacer-less game's speech duck length uses `PacerConfig`'s default numbers.

## Carry into M4 (playtest)

- **`<PREFIX>_NO_CONTROLLER`.** Freight Fate keeps pads out of agent sessions
  with an environment variable; add it with the agent server.

## Deliberately left

- A second unreadable settings file overwrites the previous `settings.json.bad`.
- No test forces a failed settings save on Escape (logged, not panicking).
- Practice's resume line is covered only by the owner's listening pass.
- Controller rumble and Freight Fate's controller mapping diagnostics (the
  missing-trigger warning, `--controller-diagnostics`) were not carried; add
  them when a game needs haptics or a pad misbehaves.
- Pause does not pause one-shots started after it (menu earcons play
  normally); a game that needs a frozen world stops its own loops.
