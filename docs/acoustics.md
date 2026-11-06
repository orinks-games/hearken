# Acoustics

Hearken can work out how each positioned sound reaches the player through
the game's world, then render it that way. The game describes the world (walls,
boxes, poles, floors, ceilings, their materials, and the weather) and the engine
does the rest:

- the propagation delay, which can glide for Doppler;
- loss with distance and through the air, per frequency band;
- transmission through walls, which muffles more for heavy materials;
- the path round corners, so a sound behind a doorway is heard from the
  doorway;
- early reflections off nearby surfaces, each from its own direction;
- a shared reverb tuned to the room the player stands in;
- pan for speakers, or an HRTF for headphones;
- optionally, obstacle sense: silent walls and poles heard through a quiet
  noise floor.

It is opt-in. A game that keeps calling `Audio::update(occluded)`, or
leaves `Game::acoustic_scene` alone, sounds exactly as before.

The work is split over two crates:

- `hearken-acoustics` is the model. It is pure and deterministic: the same
  inputs give the same bits on every platform, wasm32 included, so a game's
  rules can ask it questions (can that guard hear the player?) and online
  replays still verify.
- `hearken-audio` renders the model's answers. It is re-exported as
  `hearken_audio::acoustics` (and so `hearken_app::audio::acoustics`).

## Turning it on

In a `hearken-app` game, give the engine a scene:

```rust
use std::sync::Arc;
use hearken_app::audio::acoustics::{Builtin, Scene, SceneBuilder, Shape};

struct MyData {
    scene: Arc<Scene>,
    // ...
}

impl Game for MyGame {
    // ...
    fn acoustic_scene(data: &MyData) -> Option<Arc<Scene>> {
        Some(data.scene.clone())
    }
}
```

While `acoustic_scene` returns a scene, the engine calls
`Audio::update_acoustic(&scene)` each frame instead of `update`, and
`Game::occluded` is not asked. If the game has not called
`Audio::set_acoustics`, the path turns on with the default
`AcousticSettings`.

Without `hearken-app`, call the pool directly:

```rust
audio.set_acoustics(Some(AcousticSettings::default()));
// each frame:
audio.set_listener(pos, facing);
audio.update_acoustic(&scene);
```

Then play positioned sounds as usual (`play_at`, `start_loop` with a
position, `start_voice` or `start_feed` with a position). The acoustic path
covers:

- positioned sounds started while it is on, on a bus in
  `AcousticSettings::buses` (effects, ambience and voice by default);
- not unpositioned sounds, the UI bus or music, which play as before;
- not sounds started before the path was on. They keep the plain model,
  but `update_acoustic` muffles them when the scene blocks their line of
  sight, as the `occluded` hook would.

Give the same `Arc` each frame while the scene is unchanged. A sound
whose position, listener and scene are all as they were is not worked out
again. To change the scene, make a new `Arc`. `Arc::make_mut` does this,
because the pool keeps a clone. `Scene::set_atmosphere` changes the weather
without rebuilding anything.

`Audio::set_acoustics(None)` turns the path off. Sounds carrying the
acoustic insert go back to the plain spatial model, and the reverb rings
out.

## Describing the world

A scene is built in code with `SceneBuilder`, or loaded from JSON with
`Scene::from_json` (or from a `SceneDesc` with `Scene::from_desc`). Axes are
metres: x east, y north, z up. Headings are degrees clockwise from north,
as everywhere in Hearken.

Shapes:

| Shape | What it is | Helper |
| --- | --- | --- |
| `Wall { origin, u, v }` | a thin flat panel, both faces reflect | `Shape::wall_between(x0, y0, x1, y1, z, height)` |
| `Box { min, max }` | a solid axis-aligned box (a crate, a building) | `Shape::cuboid(a, b)` |
| `Cylinder { base, radius, height }` | an upright pole, trunk or pillar | `Shape::pole(x, y, z, radius, height)` |
| `Ground { z }` | an endless floor | |
| `Ceiling { z }` | an endless ceiling | |

A 2D tile game can make each wall tile a `Box` from the floor to a fixed
height and add one `Ground`.

Materials have absorption and scattering per octave band and a
transmission loss per band. The model has 20 built-in materials; use
`Builtin::Concrete.id()` and so on, or a name such as `"concrete"` in a
file. A game adds its own with `SceneBuilder::add_material`, or in the file's
`materials` list. A material in a file can give a surface density in
kg/m² in place of a transmission loss, and the mass law works the loss out.

A scene file:

```json
{
  "atmosphere": { "temperature_c": 5, "relative_humidity": 80, "pressure_kpa": 101.3 },
  "materials": [
    {
      "name": "hay_bale",
      "absorption": [0.3, 0.45, 0.6, 0.7, 0.75, 0.8, 0.8],
      "surface_density_kg_m2": 40
    }
  ],
  "objects": [
    { "kind": "ground", "z": 0, "material": "grass" },
    { "kind": "box", "min": { "x": 2, "y": 2, "z": 0 }, "max": { "x": 4, "y": 3, "z": 1 }, "material": "hay_bale" },
    { "kind": "cylinder", "base": { "x": -3, "y": 5, "z": 0 }, "radius": 0.15, "height": 4, "material": "wood" }
  ]
}
```

Scene files may come from players or mods, so loading never trusts them.
A NaN, an infinity, a negative size, an unknown material or a shape past
the `Limits` is dropped. The `Report` that comes back lists what was
dropped and why, up to 64 entries plus a count. The file size and the
object and material counts are capped. Unknown fields are skipped.

The searches are capped too. `QueryOptions::max_shape_tests` (default one
million) bounds the shapes the path search, and again the reflection
search, may test, so many shapes crowded into one spot cannot stall a
frame. A search that hits a cap sets `truncated`.

## What each sound gets

Each frame, for each acoustic sound, `update_acoustic` runs
`hearken_acoustics::propagate` from the sound to the listener and hands
the result to the sound's insert:

- Delay: the main arrival's travel time at the scene's speed of sound.
  With `doppler` on (the default), the delay glides as distances change.
  The pitch then shifts by `1 + v / c` for a source approaching at `v`.
  That is first order: a sound coming straight at you at 30 m/s is shifted
  by 8.7 %, where the exact moving-source value is 9.6 %. With `doppler`
  off, the delay is held and crossfades to the new value once it is 30 ms
  out, so the pitch never shifts. With `delay` off, the main arrival plays
  at once and the reflections keep their lag behind it.
- Gain per band: spreading as `reference_distance / distance` (0 dB at
  1 m by default, never louder closer in), air absorption, and
  transmission through every shape crossed. On the audio thread this is a
  cascade of six shelving filters between the seven octave bands.
- Round corners: when the straight line is blocked, the model looks for
  the shortest way round, over corners, with the ISO 9613-2 barrier loss.
  What gets through the wall and what comes round the corner add up. The
  sound is heard from the corner nearest the listener when that path is
  the stronger one.
- Early reflections: image sources of order 1 and 2, up to 8 taps per
  sound. Each tap has its own delay, its own direction, and a low, mid and
  high gain. A reflection keeps its tap while it moves, so a moving source
  glides its reflections instead of swapping them. Reflections are why a
  footstep next to a wall sounds next to a wall.
- Reverb send: the room round the listener is estimated from 256 fixed
  rays: volume, surface, Eyring RT60 per band, and reverberant level. It is
  estimated again when the listener moves 0.5 m or the scene changes. Every
  sound sends to one shared reverb, an 8-line feedback delay network tuned to
  that room. Its level is the room's reverberant level, scaled by how much of
  the sound reaches the listener's space.
- Direction: on speakers, each path is panned with the same
  constant-power law as Kira's panner, and a sound behind gets the same
  quieter, duller cue `SpatialConfig` gives (`behind_gain`,
  `behind_lowpass_hz`). On headphones, see below.

Parameter changes ramp over `ramp_ms` (50 ms by default), so nothing
clicks.

### Settings

`AcousticSettings` fields:

| Field | Default | What it does |
| --- | --- | --- |
| `query` | `QueryOptions::default()` | reflection order and count, search caps, reference distance |
| `reverb` | `ReverbOptions::default()` | rays for the room estimate |
| `buses` | effects, ambience, voice | which buses get the acoustic path |
| `doppler` | `true` | glide the delay (Doppler) or crossfade it |
| `delay` | `true` | keep the propagation delay |
| `reflections` | `true` | early reflection taps |
| `room` | `true` | the shared reverb |
| `hrtf` | `false` | headphone rendering (needs the `hrtf` feature) |
| `ramp_ms` | `50` | how long changes ramp |
| `max_delay_s` | `0.5` | longest delay a sound holds, 0.01 to 2 s (0.5 s is about 170 m); farther sounds arrive at this delay |
| `obstacle_sense` | `0` | strength of the obstacle-sense floor, 0 to 1 |
| `sense_radius_m` | `4` | how far obstacle sense reaches |
| `room_move_m` | `0.5` | how far the listener moves before the room is estimated again |

Physical spreading falls faster than the plain spatial model's default
rolloff. If a game's mix was tuned on the plain model, raise
`query.reference_distance` (and `reverb.reference_distance` with it) rather
than turning sounds up one by one.

## Headphones (HRTF)

With the `hrtf` feature (on by default in `hearken-app`) and
`AcousticSettings::hrtf` set, each sound's main arrival is convolved with a
head-related transfer function, through the `sofar` crate. This gives
front, back and height cues that a pan cannot. Reflections stay panned.
HRTF is for headphones; leave it as a player setting and keep it off for
speakers.

The bundled set is the MIT KEMAR dummy-head measurement (normal pinna),
by Bill Gardner and Keith Martin, MIT Media Lab, 1994. Its documentation
puts no restriction on use and asks that the authors are cited. A game that
ships it should credit them, for example in its credits or `help.md`.
`crates/hearken-audio/hrtf/README.md` has the source, the checksum and the
licence text.

A game can use its own measurement instead:

```rust
audio.set_hrtf(HrtfSource::Sofa(std::fs::read("assets/my_head.sofa")?.into()));
```

The set loads at the mixer's sample rate the first time it is needed. A
file over 64 MB, one that will not parse, or one that makes the reader
fail is reported in the log, and sounds stay panned. Sounds started before
the set loaded, or before `hrtf` was turned on, stay panned until they are
started again.

## Obstacle sense

People who listen for their surroundings hear a silent wall or pole
because it changes the sound around it. A near wall blocks the ambient
sound from its side and reflects back what comes from the other side,
slightly later. The two copies comb-filter: the sound takes on a colour,
and the bass lifts as the wall gets very close. A pole shadows the high
frequencies from its direction.

`AcousticSettings::obstacle_sense` (0 to 1) renders this over a quiet
generated noise floor: eight noise sources round the head, one per 45
degrees. Each frame, a ray along each sector finds walls and boxes within
`sense_radius_m`, and nearby poles shadow the sector they stand in. The
floor plays as a feed on the ambience bus, so the ambience volume applies.
At full strength it is about 45 dB below full scale (RMS, each ear). It works on any
backend that plays feeds.

## Backends

| Backend | What it does |
| --- | --- |
| Kira | everything above: the insert after any bus insert, the shared reverb on its own send track |
| others (FMOD, test doubles, a game's own) | a plain mix per sound: the gain over the middle bands, the pan, and a low-pass fitted to how much the highs fell; no delay, reflections or reverb |

A backend renders the model by answering `true` to
`Backend::renders_acoustics` and running the `AcousticDsp` it is given in
`Backend::play_acoustic`. It can also take `Backend::set_room` for a
reverb of its own.

On the audio thread, nothing in the acoustic path allocates, locks or
waits. Settings cross from the game thread through a sequence lock over
atomics: the audio side never waits, and if it catches a write half done it
keeps the last values. Filter state is flushed so it never decays into
denormals. Each sound's delay line is sized when the sound starts: 0.5 s at
96 kHz is about 190 KB.

## Asking the model directly

A game's rules can use `hearken-acoustics` on its own, with no audio:

- `direct_path`: is the straight line blocked, and how much gets through;
- `diffracted_path`: the way round;
- `line_of_sight`: a yes or no;
- `navigate`: a walking route with a clearance, for a beacon at the next
  corner;
- `nearby_surfaces`: walls and poles round the listener, with direction,
  distance, material and size.

These give the same answer on every machine, so they are safe in rules that
online runs replay.

## Where the numbers come from

- Speed of sound: Cramer (1993), with temperature, humidity and pressure.
- Air absorption: ISO 9613-1, at the exact octave-band centres. This
  matches the ISO 9613-2 Table 2 values.
- Barrier loss round corners: ISO 9613-2.
- Reverberation: Sabine and Eyring. The engine uses Eyring, which stays right
  in absorbent rooms.
- Materials: each built-in material cites its source in
  `crates/hearken-acoustics/src/material.rs`. Some caveats:
  - The cited tables stop at 4 kHz, so the 8 kHz value repeats the 4 kHz
    one.
  - Scattering values are estimates, not measured data.
  - Metal and rock have no table entry. They use glazed tile and brick
    values as stand-ins.
  - The grass, soil, gravel, snow and foliage values come from the same
    Everest and Pohlmann table as the indoor ones. The foliage row is
    balsam firs, each with 20 sq ft (1.9 m2) of ground and 11 ft (3.4 m)
    high.
- HRTF: MIT KEMAR, Gardner and Martin (1994), as above.

## Limits

- Paths round obstacles are searched at one height, midway between source
  and listener, and then sloped. Sound does not go over the top of an
  obstacle.
- Cylinders give no mirror reflections. Their curved surfaces count as
  scattering.
- The reverb is a statistical model, and it fits closed rooms best. With
  only a ground plane it still reports an RT60 of about a second in the
  low bands, but at about -26 dB, so it is barely heard.
- Doppler is first order (see Delay above).
- The HRTF covers the main arrival only. Its block adds 64 frames of
  latency, which the delay takes back when the sound is that far away.
- Obstacle sense uses its own noise floor. It does not yet filter the
  game's own ambience.
- Sounds already playing keep what they started with: the path does not
  add itself to them later.
- Cost, in release builds on the test scene: about 26 µs per sound per
  frame for the propagation query (skipped when nothing moved), and about
  93 µs for each room estimate.
