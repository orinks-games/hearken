# M1 follow-ups

Open items recorded when M1 closed (2026-09-26). Each is small; pick them up in
M1b or M2, whichever touches the code first.

## Carry into M2 (audio)

- **BASS path per game.** `hearken-bass-sys` reads only `HEARKEN_BASS_PATH`; the
  spec says a game may name its own. `hearken-audio` should pass
  `<PREFIX>_BASS_PATH`, and the one-element loop in `loader.rs` goes.
- **Clamp `channel_get_data_float`.** Cap the byte count at `0x0FFF_FFFC` so a
  buffer over 256 MB can never set `BASS_DATA_*` flag bits.
- **Test a playing callback stream dropped mid-pull.** The drop ordering in
  `safe/callback.rs` exists for BASS's mixer thread; only decode streams are
  tested today.
- **Menu sounds.** Freight Fate plays move, select and back earcons; Hearken's
  menus are silent until `hearken-audio` lands.

## Carry into M1b or earlier

- **Session log rotation and level.** `session.log` appends forever and follows
  `RUST_LOG`, so a player with `RUST_LOG=warn` set for another tool loses the
  transcript. Rotate to `session.log.prev` at start (after the single-instance
  check, so a second copy never rotates the live log) and add a `<PREFIX>_LOG`
  level variable, as Freight Fate does.
- **One Prism link recipe instead of three.** `hearken-speech`, `hearken-app` and
  every game's `build.rs` repeat the `/DELAYLOAD` list and
  `/INCLUDE:__pfnDliFailureHook2`. Either anchor the failure hook from Rust in
  `hearken-speech` so games need only `/DELAYLOAD`, or ship a build-dependency
  helper. Worth raising upstream with prismer too (its build script could emit
  the `/INCLUDE` itself).

## Deliberately left

- A second unreadable settings file overwrites the previous `settings.json.bad`.
- No test forces a failed settings save on Escape (logged, not panicking).
- Practice's resume line is covered only by the owner's listening pass.
