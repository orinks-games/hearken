# Sealed release data

Date: 2026-10-27. Status: shipped in #23, then changed on 2026-10-28.

**Update, 2026-10-28:** the ledger is gone. Players asked what the
`hearken-seal.ledger` file in their saves folder was, so the data folder
now holds only the game's own files. Sealed files are format version 2:
no folder id and no generation, just the file's name bound in. That drops
the ledger's protections: an older copy of a file, a deleted file, or a
file copied from another install now reads as good, the same way a whole
restored or copied folder already did. Editing, swapping one file for
another, other keys, and (with witnesses) plain files over sealed ones are
still caught. Trusted results belong to the server replay planned for
online play. The ledger sections below are kept as history.

## Goal

A player of a released Hearken game should not be able to give themselves a
high score, an unlock or a finished save by editing a file. Wicket's keeps
every game's records as plain JSON in `records.json`, so anyone with
Notepad can change them today. Its sounds are already packed, but
`sounds.pak` is encrypted, not authenticated, and a loose `sounds` folder or
`<PREFIX>_SOUNDS` still replaces it.

Hearken is a general engine, so the answer is an engine mechanism that any
game opts into. Nothing in it names a game, a file or a record format. Each
game picks its own files and its own policy for what to do when one has
been tampered with. Wicket's is the first game to adopt it.

**Success:**

- In a sealed release, a data file changed in a text or hex editor, swapped
  for another sealed file, restored from an older copy, deleted, replaced by
  a plain file, or brought in from someone else's folder is reported to the
  game as tampered. It is never quietly trusted.
- In a packed release, a changed byte in `sounds.pak` is detected. A loose
  `sounds` folder and `<PREFIX>_SOUNDS` are ignored.
- The key is never stored whole in the program, and it never goes into any
  repository.
- Updating the game keeps every player's saves.
- Developer builds read and write plain files, as they do now.
- Settings (`settings.json`: volumes, speech, keys) stay plain. They are
  the player's own preferences and nothing to cheat with.

## What this can and cannot stop

The game runs on the player's computer, and the key it seals with is inside
the program they were given. Someone willing to take that program apart in
a debugger can recover the key and forge any file. No offline scheme can
prevent that, and this one doesn't claim to. It raises the cost of cheating
from "open a text editor" to "reverse engineer a Rust binary".

| Attack | Result |
|---|---|
| Edit a sealed file in a text or hex editor | `Tampered(Forged)` |
| Copy `b.json` over `a.json` (another file name, same game) | `Tampered(Forged)`: the name is bound into the seal |
| Bring in a sealed file from another game, or from a build made with another key | `Tampered(Forged)` |
| Restore an older copy of one file (a backup from before a bad run) | `Tampered(RolledBack)` |
| Delete a sealed file to reset it | `Tampered(Removed)` |
| Replace a sealed file with a hand-written plain one | `Tampered(Unsealed)` |
| Copy in someone else's single file, even one whose generation is exactly one ahead of this folder's | `Tampered(Forged)`: every file is sealed with its folder's random id |
| Copy in someone else's ledger alone | Every file reads `Tampered(Forged)`: the ledger's folder id is not theirs |
| Edit the ledger (the folder's history) | `Tampered(LedgerForged)` |
| Delete the ledger | `Tampered(Unexpected)` for every sealed file: the ledger is written before the first file is sealed, so no sealed file without one is genuine |
| Get a file set aside (or reset), then put a plain file or an old copy in its place | `Tampered(Unsealed)` or `Tampered(RolledBack)`: the ledger keeps a gone file's generation |
| Delete the ledger and every sealed file, or switch to a fresh folder (`portable.txt`, `XDG_DATA_HOME`), then drop in a hand-written plain file for the legacy import | `Tampered(Unsealed)` while a witness outside the folder is there (see [Legacy plain files](#legacy-plain-files)) |
| Point `<PREFIX>_DATA_DIR` at a folder of crafted files | Ignored in a sealed build |
| Search the program for the key's 32 bytes or 64 hex digits | Not found: the key is split into shares (as the pack key already is) |
| Edit, cut short or reorder `sounds.pak` | The changed sound, or the whole pack, fails to authenticate |
| Drop a `sounds` folder beside a packed game, or set `<PREFIX>_SOUNDS` | Ignored in a packed build |
| Build the game from source with no key, or with a different key | That build cannot read release saves (`Locked`) and will not overwrite them. Release builds reject its files. |

What it does not stop, and the docs say so:

- **Reverse engineering.** Recovering the key from the program, or patching
  the program, defeats everything here. The key is split into random
  shares, so searching the program for it finds nothing, but a debugger
  and some scripting can rebuild it, and any sealed file confirms a guess.
  Because the key stays the same across releases (so updates keep saves),
  one extraction works for every release until the developer changes the
  key, which starts every player over.
- **Restoring the whole data folder** (accepted). If a player restores
  every file, including the ledger, from one earlier moment, the folder is
  consistent with itself, and nothing offline can tell. A rollback is only
  detected when it is partial. This lets a player undo a bad result (retry
  a challenge), not invent a score. A second counter outside the folder
  could notice a restore, but it would also turn a player's own legitimate
  backup restore (after a reinstall, say) into "tampered" and cost them
  their records, and the outside counter can be snapshotted just as
  easily. In portable mode the saves sit inside the install, so copying the
  whole install copies them too.

  A finer form of the same thing: the ledger catches up when a file is
  exactly one generation ahead of it (the repair for a crash between the
  file and the ledger). A player who keeps a copy of the ledger at
  generation N can put it back beside any of several files written at
  N+1 and keep the one they like. That picks among results the game
  really produced, which a full-folder snapshot of each result already
  allows, so tightening the catch-up (a "pending" ledger entry written
  before each file, say) would cost a second synced write per save and
  stop nothing a snapshot does not.
- **Sharing a whole data folder** (accepted). Every copy of a game uses the
  same key, so a complete saves folder (files and ledger together) copied
  from a friend works. Tying saves to one machine or one install would
  break moving to a new computer or reinstalling, which players do far more
  often than they cheat, and the binding would live on the same disk the
  player controls. Single files cannot be mixed between installs: each is
  bound to its folder's id.
- **Legacy import, while a game allows it** (accepted, bounded). See
  [Legacy plain files](#legacy-plain-files).
- **Legacy import laundered through a whole folder** (accepted while a game
  imports; the main reason to stop importing soon). The two gaps above
  chain: a player runs the game once with a fresh home or config folder
  (or with the witnesses deleted) and a hand-written plain file, which the
  game imports and seals into a folder that is consistent with itself.
  They then copy that whole folder (files and ledger) over their real
  one, which a real install accepts the same way it accepts a friend's
  folder. The result is any score they like, with file copies and one
  environment variable. Binding a folder to its install (its id recorded
  in the witness, say) does not close it, since the forged witness is
  just as easy to copy, and it would cost players who move their saves
  between computers. It closes when the game stops importing: from then
  on no plain file is ever sealed, so there is nothing to launder.
- **Memory editors, speed hacks and the system clock.** Daily challenges go
  by the clock, so they trust it.
- **Genuine disk corruption.** It looks the same as tampering. Writes are
  atomic (temporary file, flush, rename), so a crash never leaves half a
  file. The engine sets a bad file aside instead of deleting it, so it can
  still be recovered.

## Decisions

| Decision | Notes |
|---|---|
| New crate `hearken-seal`, re-exported as `hearken::seal` | Pure library with no engine dependency, like `hearken-pack`. |
| XChaCha20-Poly1305 (RustCrypto `chacha20poly1305`) with a random 24-byte nonce for each write | Authenticates and encrypts. Random nonces mean no counter has to be stored to avoid reusing one. |
| The file key comes from the game secret through HKDF-SHA256 (`hkdf`, `sha2`) | Keeps the stored secret separate from the key in use, and leaves room for other purposes later. |
| The file name, the format version and the generation are associated data | Swapping files, or changing the header, fails authentication. |
| A sealed **ledger** in each data folder records every sealed file's generation | Catches partial rollback, deletion and plain files written over sealed ones. |
| One secret per game, **stable across releases** | Kept by the developer and passed to the build. It is not created per release like the pack key, because a new key would invalidate every save when the game updates. |
| The secret is baked in as shares by a new build-dependency crate, `hearken-bake` | `hearken-pack`'s share code moves into it, so both keys use one implementation. |
| A file keeps its own name and path. Sealed files and plain files are told apart by the magic bytes | `records.json` stays `records.json`. Migration needs no rename, and a dev build and a release agree on where each file lives. |
| The engine returns a verdict; the game chooses what to do with it | The engine never decides that a tampered file should be loaded, and never decides by itself to throw away a player's progress. |
| A build without a key never overwrites a sealed file | A developer who runs a dev build against their own release saves doesn't wipe them. |
| Opt-in through `seal = true`, plus a committed `seal-id` fingerprint | Stops a release (for example, CI with a missing secret) from going out with the wrong key. |
| `sounds.pak` version 2: ChaCha20-Poly1305 for each sound and for the index | Any changed byte is detected. Version 1 is dropped, because a pack only ever has to match the program it ships with. |
| A packed build reads only its pack | No `<PREFIX>_SOUNDS` and no loose `sounds` folder. Built-in clips still apply. |
| A sealed build ignores `<PREFIX>_DATA_DIR` | That override is for development and tests. |

## Crate layout

```
crates/
  hearken-bake   new, no dependencies: splits a 32-byte key into shares as Rust code (build-dependency only)
  hearken-seal   new: Key, Vault, Opened, Tamper; build.rs bakes HEARKEN_SEAL_KEY through hearken-bake
  hearken-pack   format v2 (authenticated); build.rs now uses hearken-bake
  hearken-audio  discover_clips: a packed build reads only its pack
  hearken-app    Ctx gains `vault`; Paths ignores <PREFIX>_DATA_DIR when sealing; re-exports seal
  hearken-cli    seal / seal-id metadata; the key lookup in a new seal_key.rs; one line in release.rs
```

Dependencies only point down. `hearken-seal` depends on `chacha20poly1305`,
`hkdf`, `sha2` and `getrandom`. `hearken-cli` depends on `hearken-seal` to
create keys and compute their IDs. The engine and the CLI share the
fingerprint function, so the two can't disagree about a key's ID.

## The file format

```text
offset size
0      4    "HKSL"
4      1    version = 1
5      3    reserved, zero
8      8    generation (u64 LE), 1 for the first write of a name in this folder
16     24   nonce (random)
40     ..   ciphertext of the file's bytes, then the 16-byte Poly1305 tag
```

- Key: `HKDF-SHA256(salt = "hearken-seal", ikm = game secret, info = "files v1")`.
- Associated data: the 40-byte header, then `"hearken-seal/v1\0"`, then the
  folder's 16-byte id from the ledger (zeros for the ledger itself and for
  witnesses), then the file's name in UTF-8. The game's bytes are encrypted as they are. The
  engine doesn't know or care that they are JSON.
- A file whose first four bytes are not `HKSL` is **plain**.
- A file name is a single path component: `[A-Za-z0-9._-]+`, not starting
  with `.`, at most 64 bytes, and not the ledger's name. Any other name is
  rejected with `InvalidInput`. Games choose names freely within that rule.

### The ledger

`hearken-seal.ledger`, in the same data folder, sealed in the same format
under the reserved name `hearken-seal.ledger`. Its plaintext starts with
`folder\t<32 hex digits>\n`, the folder's random id, chosen when the
ledger is first written. Then comes one line per sealed file, sorted by
name: `name\tgeneration\n`, or `name\tgeneration\tgone\n` for a file
the game removed or set aside. A gone file keeps its generation. The vault
holds the ledger in memory after its first read.

The folder id goes into every file's associated data. A file copied in
from another install, or from before a forged ledger was started over,
does not authenticate, whatever its generation.

Write order for `write(name, bytes)`:

0. If the folder has no ledger yet, write one (the new folder id, no
   entries) first, so no file is ever sealed under an id that was not kept.
1. `g` = the ledger's generation for `name` + 1 (1 when it has none).
2. Seal the bytes with generation `g`, write `name.tmp`, flush, and
   rename it over `name`.
3. Set the ledger entry to `g` and write the ledger the same way.

A crash between steps 2 and 3 leaves the file one generation ahead of the
ledger. The read rule accepts that and repairs the ledger.

Read rule for `read(name)` in a sealed build. `L` is the ledger's
generation for the name, or 0 when the ledger has no entry for it or no
ledger exists. "Expected" means the entry exists and is not gone.
"History" means the ledger exists, or a witness does:

| File on disk | Ledger | Verdict |
|---|---|---|
| none | not expected | `Missing` |
| none | expected | `Tampered(Removed)` |
| plain | not expected, no history | `Legacy(bytes)` |
| plain | expected, or any history | `Tampered(Unsealed)` |
| sealed | no ledger | `Tampered(Unexpected)` |
| sealed, does not authenticate with this folder's id | any | `Tampered(Forged)` |
| sealed, generation `g` | `g == L`, expected | `Sealed(bytes)` |
| sealed, generation `g` | `g == L + 1` | `Sealed(bytes)`, and the ledger is repaired to `g` |
| sealed, generation `g` | `g <= L` otherwise | `Tampered(RolledBack)` |
| sealed, generation `g` | `g > L + 1` | `Tampered(Unexpected)` |
| any | the ledger exists but does not authenticate | `Tampered(LedgerForged)` |

The catch-up row only ever accepts a file sealed with this folder's id,
so it repairs a crash and nothing else. Deleting the ledger leaves every
sealed file `Unexpected`. The only state consistent with no ledger is a
fresh folder.

### Witnesses

`Vault::with_witnesses(paths)` names marker files outside the data folder,
and `Vault::checking_witnesses(paths)` names more that it reads but never
writes. A witness is sealed under `.hearken-seal.witness`, a name no data
file can have, so a witness never authenticates as a data file or the
reverse. It holds the names still importable: when the first witness is
written, every plain file under a valid name then in the folder, less each
name sealed since (the vault brings every witness it writes up to date on
each sealing write). A plain file reads as `Legacy` while no witness exists,
or while every witness there lists its name; otherwise it is
`Tampered(Unsealed)`. A witness that does not authenticate lists nothing.
So a player upgrading with several old saves imports each of them, one at a
time, while a hand-written file under a name that was not an old save, or
over one already sealed, is refused. Writing a witness is best effort
(through a temporary file, then a rename): a failure is logged and saving
goes on.

Witnesses only guard the legacy import, so `hearken-app` hands them to the
vault only for a game with `Game::IMPORTS_PLAIN_SAVES = true`; the default
is false, and such a game never writes one. A portable copy writes
`<Game>.hearken-seal` beside the executable, so it leaves nothing in the
player's user folders; an installed one writes
`<config dir>/<DATA_DIR>.hearken-seal`. Each checks the other's, so adding
or deleting `portable.txt` (which moves the data folder) does not make the
game forget it has sealed. Neither is ever inside the data folder.

The cost falls on a player with more than one copy. The config-folder
witness is shared by every installed copy for that user (and checked by a
portable one), so once one has sealed, a plain file in another copy's
folder reads as `Tampered(Unsealed)` unless its name was among the old
saves in the first copy's folder. The game sets it aside rather than
deleting it, so it can still be recovered by hand, but it is not imported.
A game that imports should tell players to update by unpacking over their
old copy, or by moving their old saves in before the new copy first runs,
and should turn the import off once players have moved over.

A build without the key will not add a new plain file to a folder that
holds a sealed one (a release's), since the release would take it for
tampering.

## The key

### Its lifecycle

1. The developer adds `seal = true` to the game's
   `[package.metadata.hearken]`.
2. The first `hearken release` finds no key. It creates one in
   `<config dir>/hearken/seal-keys/<package>.key` (permissions `0600` on
   Unix, 64 hex digits plus a comment line), and prints:
   - where the key is, and that losing it means every player's saves stop
     loading in the next release;
   - the key's ID, and the line to commit: `seal-id = "k1:3f9a0c2e7b14d685"`;
   - how to give it to CI: store the 64 hex digits as a secret and expose
     it as `HEARKEN_SEAL_SECRET`. `hearken seal-key --print` prints them.
3. Later releases find the key in this order:
   1. `HEARKEN_SEAL_SECRET` (CI);
   2. the key file.

   The CLI passes the key to Cargo as `HEARKEN_SEAL_KEY`. The CLI's input
   variable and the build's variable have different names on purpose, so
   that a secret exported for CI does not turn every `cargo build` and
   `cargo test` into a sealed build.
4. Release refuses, and makes nothing, when:
   - `seal-id` is committed but no key is found. A new key is never created
     in this case. This protects against CI with a missing secret.
   - the key's ID differs from the committed `seal-id`.
   - `CI` is set and no key is found. Keys are never created on a CI
     runner.
   - the game's `Cargo.lock` has no `hearken-seal`. Its Hearken is too old
     to seal (the same check as `engine_reads_packs`).
5. A game with `seal` off still builds with `HEARKEN_SEAL_KEY` set to empty,
   so a variable in the developer's shell can't leak into a release.

The key and its hex digits are never printed, except by
`hearken seal-key --print`, and never written to the stage, `dist/` or the
log. The ID is the first 8 bytes of `SHA-256("hearken-seal key id\0" || secret)`,
as `k1:` followed by 16 hex digits. It is safe to commit.

### Baking the key in

`hearken-bake::scattered(&[u8; 32]) -> String` is `hearken-pack`'s current
`scattered` function, moved without changes: three random shares, a random
rotation and a random order, rebuilt only when the key is used. Different
for every build. `hearken-bake::parse_hex(&str) -> Option<[u8; 32]>` comes
with it. Both `hearken-pack/build.rs` and `hearken-seal/build.rs` call
them, with `cargo:rerun-if-env-changed` set for their own variable.

`hearken_seal::build_key()` returns `Some` only in a build made with
`HEARKEN_SEAL_KEY` set, and the vault derives the file key from it each
time it is needed. Neither the secret nor the derived key is kept in a
`static`.

## Public API

### `hearken-seal` (re-exported as `hearken::seal`)

```rust
/// The build variable `hearken release` passes the game's secret in.
pub const KEY_VAR: &str = "HEARKEN_SEAL_KEY";
/// The ledger's file name, reserved in every data folder.
pub const LEDGER: &str = "hearken-seal.ledger";

/// A game's sealing secret. Debug prints only its ID.
#[derive(Clone, PartialEq, Eq)]
pub struct Key(/* [u8; 32], private */);

impl Key {
    pub fn from_bytes(bytes: [u8; 32]) -> Key;
    /// A new random key, from the operating system.
    pub fn generate() -> std::io::Result<Key>;
    /// 64 hex digits, surrounding whitespace allowed.
    pub fn from_hex(hex: &str) -> Option<Key>;
    pub fn to_hex(&self) -> String;
    /// "k1:" and 16 hex digits: safe to commit, names the key without revealing it.
    pub fn id(&self) -> String;
}

/// The key this build seals with: set by `hearken release`, `None` in every other build.
pub fn build_key() -> Option<Key>;

/// What reading a data file found.
#[derive(Debug)]
pub enum Opened {
    /// No file, and none was ever sealed here.
    Missing,
    /// An authentic sealed file: its bytes.
    Sealed(Vec<u8>),
    /// A plain file, in a build that does not seal (a developer build).
    Plain(Vec<u8>),
    /// A plain file in a sealing build, with no sealed history: written by
    /// an older version of the game, or by hand. The game decides whether
    /// to import it.
    Legacy(Vec<u8>),
    /// A sealed file in a build that cannot open it (a developer build
    /// running against a release's saves). The vault will not overwrite it.
    Locked,
    /// The file was changed, swapped, restored, removed or forged.
    Tampered(Tamper),
    /// The disk would not give the file up (permissions, I/O).
    Failed(std::io::Error),
}

/// How a sealed file was tampered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tamper {
    /// Does not authenticate: edited, cut short, another file's, another game's or another key's.
    Forged,
    /// A plain file where a sealed one was written.
    Unsealed,
    /// An older sealed copy than the one last written.
    RolledBack,
    /// A sealed copy this folder's history does not account for.
    Unexpected,
    /// Deleted after it was sealed.
    Removed,
    /// The folder's ledger itself does not authenticate.
    LedgerForged,
}

impl Opened {
    /// The bytes of `Sealed` or `Plain`: the only verdicts a game can trust as they are.
    pub fn trusted(self) -> Option<Vec<u8>>;
}

/// A game's data folder, read and written through the seal.
pub struct Vault { /* dir, Option<Key>, cached ledger */ }

impl Vault {
    /// The folder at `dir`, sealing with this build's key, if it has one.
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Vault;
    /// The folder at `dir` with a given key, or none: for tests and tools.
    pub fn with_key(dir: impl Into<std::path::PathBuf>, key: Option<Key>) -> Vault;
    /// Whether files are sealed (a release built with `seal = true`).
    pub fn is_sealing(&self) -> bool;
    pub fn dir(&self) -> &std::path::Path;
    /// Reads `name`; never trusts a file that fails the seal.
    pub fn read(&mut self, name: &str) -> Opened;
    /// Writes `name` atomically: sealed when sealing, plain otherwise. A
    /// build that does not seal refuses (`PermissionDenied`) to overwrite a
    /// sealed file. A sealing build overwriting a plain file first keeps it
    /// as `<name>.plain-backup`.
    pub fn write(&mut self, name: &str, bytes: &[u8]) -> std::io::Result<()>;
    /// Deletes `name` on purpose (a reset the player chose); the ledger marks
    /// it gone, keeping its generation.
    pub fn remove(&mut self, name: &str) -> std::io::Result<()>;
    /// Moves a file the game will not use to `<name>.tampered[-n]`, so it
    /// can still be looked at, and marks it gone in the ledger, so the next
    /// read is `Missing`, a plain file later put there is still `Unsealed`,
    /// and the next write starts a new generation. Returns where it went
    /// (`None` when there was no file).
    pub fn set_aside(&mut self, name: &str) -> std::io::Result<Option<std::path::PathBuf>>;
    /// Leaves markers at `paths` (outside the folder) once it seals, so a
    /// wiped or fresh folder offers no `Legacy` import. See Witnesses.
    pub fn with_witnesses(self, paths: Vec<std::path::PathBuf>) -> Vault;
    pub fn witnesses(&self) -> &[std::path::PathBuf];
}
```

### `hearken-app` (and so `hearken`)

```rust
pub use hearken_seal as seal;
pub use hearken_seal::{Opened, Tamper, Vault};

pub struct Ctx<G: Game> {
    // ...existing fields...
    /// The game's data folder, through the seal. `Vault::new(&paths.data_dir)`.
    pub vault: Vault,
}
```

- `Ctx::new` keeps its signature and builds the vault from `paths`. The
  testing harness gets a vault without a key automatically.
- `Paths::discover` ignores `<PREFIX>_DATA_DIR` (and logs that it did) when
  `hearken_seal::build_key().is_some()`. The `Game::ENV_PREFIX` doc comment
  says so.
- No new `Game` items. Which files are sealed is decided by which files a
  game writes through `ctx.vault`. Policy is the game's own code.
- `Settings` stays plain in every build, and goes on writing `settings.json`
  as it does now.

### `hearken-pack` (version 2)

The signatures stay the same, except that `Pack::unseal` now returns
`std::io::Result<Vec<u8>>`, with `InvalidData` when the tag fails.
`Pack::read` reports the same error.

### `hearken-audio`

```rust
/// Where a game's sounds come from, and what the player should hear if
/// they are broken.
pub struct Discovered {
    pub clips: Box<dyn ClipSource>,
    /// Set when a packed build's `sounds.pak` is missing or damaged.
    pub notice: Option<String>,
}
pub fn discover_clips(env_prefix: &str) -> Discovered;
/// The same choice, with every input explicit, for tests.
pub fn discover_clips_from(env_prefix: &str, pack_key: Option<[u8; 32]>, exe_dir: Option<&Path>) -> Discovered;
```

`run.rs` pushes `notice` onto `ctx.notices`. The notice is "This copy of
the game's sounds is missing or damaged. Reinstall the game to hear it."

## Pack hardening

The `sounds.pak` version 2 layout:

```text
"HKPK"  version = 2 (1 byte)  3 reserved bytes  index length (u32 LE)
index: ChaCha20-Poly1305, nonce 0, AAD = the 12-byte header; plaintext as in v1
sound n: ChaCha20-Poly1305, nonce n + 1, AAD = the sound's key (UTF-8); length in the index includes the 16-byte tag
```

- A changed header, index, or reordering of entries fails `Pack::open`. A
  changed sound fails when that sound is read, and plays nothing. The
  failure is logged once per key. Moving one sound's bytes into another's
  slot fails too, because the nonce and the key are both bound.
- `PackedClips` still caches each sound encrypted and authenticates it on
  each play. ChaCha20-Poly1305 checks at several GB/s, so the cost is
  negligible.
- **Packed builds don't fall back.** When `hearken_pack::build_key()` is
  `Some`, `discover_clips` uses the pack and nothing else: no
  `<PREFIX>_SOUNDS`, no loose `sounds` folder beside the program or in the
  current directory. A missing or damaged pack plays built-in clips only,
  and sets `notice`. Files that aren't sounds and that release ships loose
  in `sounds/` (credits) are not affected.
- The pack key stays per release. Packs and saves don't share a key.

## Per-game configuration

```toml
[package.metadata.hearken]
portable = true
pack = true
seal = true                         # data files written through ctx.vault are sealed in a release
seal-id = "k1:3f9a0c2e7b14d685"     # the key's ID, which hearken release checks; added after the first release
include = ["sounds", "README.md", "LICENSE"]
```

`Project` gains `seal: bool` and `seal_id: Option<String>`, parsed next to
`pack`.

## CLI changes

Kept small and in their own places, so PR #15 merges cleanly:

- `project.rs`: two fields and their parsing, plus one test.
- New `seal_key.rs`:
  - `pub fn release_key(project: &Project) -> Result<Option<hearken_seal::Key>, String>`
    covers lookup, creation, the CI and `seal-id` rules, the printed
    instructions and the lock-file check;
  - `pub fn key_file(package: &str) -> Option<PathBuf>`.
- `release.rs`: one call before the build, and the env slice grows by one
  pair, `(SEAL_KEY_VAR, &seal_hex)`, which is always passed, empty when
  `seal` is off. No other changes.
- `main.rs`: one usage line and one match arm for `hearken seal-key
  [--print]`. Without `--print`, it shows where the key is and its ID.
  With `--print`, it also shows the hex digits, for a CI secret.
- README: `seal` and `seal-id` in the metadata example, and a short
  "Sealed saves" section covering the key file, backing it up, the CI
  secret, and the limits described above.

## Legacy plain files

Existing Wicket's players have a plain `records.json`. A sealed release
reads it as `Legacy(bytes)`, because the folder has no sealed history. The
game decides what to do:

- **Import** (what Wicket's does at first): parse it, use it, and write it
  back through the vault. That seals it, keeps the original as
  `records.json.plain-backup`, and starts the ledger. From then on, a plain
  `records.json` reads as `Tampered(Unsealed)`.
- **Refuse** (the right choice for a game that never shipped plain saves,
  and for Wicket's once the transition is over): treat `Legacy` as
  tampered.

`Legacy` is offered only while the folder has never sealed (no ledger)
and no witness says the game has sealed for this player. Once a file is
set aside or removed, its gone entry keeps a later plain file `Unsealed`.
Deleting the ledger and the sealed files, or switching to a fresh folder
(`portable.txt`, `XDG_DATA_HOME`), still meets the witness, so the plain
file reads as `Unsealed`.

**Remaining risk while import is allowed (accepted):** a genuine legacy
file and a hand-written one are the same bytes, so a player who finds and
deletes the witnesses too, or who runs the game as a new user account, on
another computer, or with the platform folders moved (`XDG_CONFIG_HOME` on
Linux, for a copy that is not portable), looks exactly like a player
upgrading from an older version, and gets their plain file imported. No
offline check can tell those apart without refusing real upgrades. The
risk ends when the game stops importing. Wicket's makes
that a single constant (below), turned off in a later release after most
players have run a sealed one. Players who skip every transition release
start fresh. The docs say so. While import is on, an imported folder
can also be copied over a real install's folder (see
[What this can and cannot stop](#what-this-can-and-cannot-stop)), so a
forged import reaches any install, not only the fresh one it was made in.
That is why the import should be on for one transition period, not for
good. A game that can tell implausible values (a score above what a board
allows) may also check imported ones before trusting them; the engine
cannot, since it does not know what the bytes mean.

## How Wicket's adopts it

- `crates/wickets/Cargo.toml`: `seal = true`. After the first sealed
  release, `seal-id = "…"`. The release workflow exposes the repository
  secret as `HEARKEN_SEAL_SECRET` in the Package step only. The key itself
  is not committed.
- `parlor-core/src/records.rs`: `Records::file` reads through `ctx.vault`
  instead of `std::fs` (it takes the vault, not the folder). `save` writes
  through `ctx.vault.write("records.json", …)`. The policy:

  | Verdict | Wicket's |
  |---|---|
  | `Sealed`, `Plain` | parse as today |
  | `Missing` | start fresh |
  | `Legacy` | import while `Parlor`'s `IMPORTS_PLAIN_SAVES` (`true` for now) is true; otherwise `set_aside` and start fresh |
  | `Locked` | start fresh in memory and don't save. The vault refuses anyway. Log it. |
  | `Tampered(_)` | `set_aside`, start fresh, and say once: "Your records were changed outside the game, so they start over." |
  | `Failed` | start fresh in memory, don't save over the file, log it |

  "Start fresh" after `Tampered` is the honest choice offline. Keeping the
  last good copy would let a player restore it. A JSON file that won't
  parse, inside an authentic seal, is handled as it is today.
- Tests use `Vault::with_key(tempdir, Some(Key::generate()))` to cover
  every row above. The rest of the test suite stays without a key.

## Test plan

**hearken-bake**
- `scattered` output, compiled and run in a doctest, rebuilds the key. Two
  calls produce different shares.

**hearken-seal** (unit tests in the crate)
- Round trip: sealed, and plain with no key.
- Flipping any single byte of a sealed file (a loop over every offset)
  gives `Forged`. So does cutting it short or adding bytes.
- Copying `a.json` over `b.json` gives `Forged`. A file sealed with
  another key gives `Forged`.
- Restoring an older copy of one file gives `RolledBack`. Deleting it gives
  `Removed`. A plain file over a sealed one gives `Unsealed`.
- Deleting the ledger with a generation-3 file present gives `Unexpected`.
  An edited ledger gives `LedgerForged`.
- Crash window: a file one generation ahead of the ledger reads `Sealed`,
  and the ledger is repaired.
- A plain file in a fresh folder reads `Legacy`. Writing it leaves
  `.plain-backup`, after which a plain file reads `Unsealed`.
- No key: a sealed file reads `Locked`, and `write` returns
  `PermissionDenied` with the file unchanged.
- `remove` and `set_aside` forget history: the next `read` is `Missing`.
- Bad names (`../x`, `a/b`, `.hidden`, the ledger's name, longer than 64
  bytes) give `InvalidInput`.
- `Key::id` is stable and does not contain the hex digits. `Debug` doesn't
  print the key.
- `build_key` matches `HEARKEN_SEAL_KEY`, as pack's test does.

**hearken-pack**
- v2 round trip. Every byte of a small pack flipped in turn: the open, or
  the read of the affected sound, fails. No flip is ever accepted.
- An entry's bytes swapped with another's of the same length fails.
- A v1 pack fails with "a version this build cannot read".

**hearken-audio**
- `discover_clips_from` with a pack key and a valid pack ignores
  `<PREFIX>_SOUNDS` and a loose `sounds` folder.
- With a pack key and no pack, or a damaged one, it plays nothing loose and
  sets `notice`.
- Without a key, it behaves as today.

**hearken-app**
- A sealed `Paths` ignores `<PREFIX>_DATA_DIR`. Tested through a
  `discover_with(…, sealing: bool)` seam.
- `Ctx::new` gives a vault without a key in tests.

**hearken-cli**
- `seal` and `seal-id` are parsed.
- `release_key`:
  - the key comes from `HEARKEN_SEAL_SECRET`, or from the key file;
  - a key is created only when there is no `seal-id` and `CI` is unset;
  - it refuses on a missing key with `seal-id` set, on an ID mismatch, and
    in CI with no key;
  - it refuses when `Cargo.lock` lacks `hearken-seal`;
  - stdout never contains the hex digits.

  Test through injected env, config dir and lock text, not the real ones.
- `HEARKEN_SEAL_KEY` is passed empty when `seal` is off.

**End to end** (run by hand, then by the adversarial agent)
- Build the template, or Wicket's against the local Hearken with
  `--config 'patch…'`, through `hearken release` with `seal = true` and a
  known key.
- The program contains neither the 32 key bytes nor the 64 hex digits.
- The sealed `records.json` is unreadable.
- Each attack in the table above, carried out on the staged release, gives
  its stated verdict and Wicket's response.
- A dev build of the same game leaves the release's saves untouched.

## Adversarial review checklist

The adversarial agent works through the table in
[What this can and cannot stop](#what-this-can-and-cannot-stop). It also
tries:

- symlinks or hard links in place of data files;
- a directory named `records.json`;
- a zero-length file;
- an empty or garbled ledger;
- two names that differ only in case (Windows and macOS file systems);
- a `.tmp` file left behind by a crash;
- `HEARKEN_SEAL_KEY` set at run time. It must have no effect, because the
  key exists only at build time;
- `strings` and a byte search of the program for the key, its hex digits
  and the derived file key;
- whether any string in the engine names Wicket's, records or a particular
  game.

Any trick that gets a changed record accepted without recovering the key
from the program is a bug in this design.
