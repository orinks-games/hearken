# M1 follow-ups

Open items recorded when M1 closed (2026-09-26), updated when M1b closed.
Each is small; pick them up in whichever milestone touches the code first.

## Carry into M2 (audio)

- **BASS path per game.** `hearken-bass-sys` reads only `HEARKEN_BASS_PATH`; the
  spec says a game may name its own. `hearken-audio` should pass
  `<PREFIX>_BASS_PATH`, and the one-element loop in `loader.rs` goes.
- **Clamp `channel_get_data_float`.** Cap the byte count at `0x0FFF_FFFC` so a
  buffer over 256 MB can never set `BASS_DATA_*` flag bits.
- **Test a playing callback stream dropped mid-pull.** The drop ordering in
  `safe/callback.rs` exists for BASS's mixer thread; only decode streams are
  tested today.
- **Earcons.** Freight Fate plays move, select and back sounds in menus, a tick
  per typed character and an error at a text field's edge, select and error
  sounds on the shortcuts screen, and a notify sound on a timed message.
  Hearken's are silent until `hearken-audio` lands; text-entry edges are silent
  for the same reason.

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
