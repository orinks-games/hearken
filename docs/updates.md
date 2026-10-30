# Updates

A released Hearken game can update itself: it checks for a newer version,
tells the player, downloads it on request, and swaps it in, with every step
spoken. The developer can publish updates anywhere: on GitHub releases, on
any HTTPS host (a static site, a bucket, a CDN), or through a source the
game writes itself. Every update is signed with the game's own key, so an
update from a plain web host is as safe as one from GitHub.

The checks and the swap are in `hearken-update`, which has no window or
speech code. The screens are in `hearken-app`, behind its `update` feature.

## Turning it on

1. Turn on `hearken`'s `update` feature in the game's `Cargo.toml`.

2. Say where updates are published, from `Game::updates`:

   ```rust
   use hearken::UpdateConfig;

   impl hearken::Game for MyGame {
       // ...
       fn updates() -> Option<UpdateConfig> {
           // GitHub releases of owner/repo:
           UpdateConfig::github("My Game", "me", "my-game")
           // Or a manifest on any HTTPS host:
           // UpdateConfig::manifest_url(
           //     "My Game",
           //     "https://example.com/my-game/update-{channel}-{platform}.json",
           // )
       }
   }
   ```

   The first argument is the game's name as players hear it ("My Game
   version 1.4.0"). Optional settings:

   - `.channels(&["stable", "beta"])`: the channels a player can follow,
     the default first. With more than one, Settings gets an "Update
     channel" row. A channel is lowercase letters, digits and hyphens.
   - `.manual("example.com slash my-game")`: where to get an update by hand,
     spoken when one cannot install itself. GitHub sources say their
     releases page unless told otherwise.
   - `.quiet()`: check at start without announcing. The game reads
     `ctx.updates.available()` and offers it its own way, for instance by
     pushing `UpdatePromptScreen::new(update)` from its title screen.
   - `.no_check_at_start()`: only check when the player asks.

   A game with its own host implements `hearken::update::UpdateSource`: it
   finds the manifest's bytes for a game, platform and channel, and says
   where the archive is. It gets the same checks as the built-in sources.

3. Add `update = true` to `[package.metadata.hearken]`:

   ```toml
   [package.metadata.hearken]
   update = true
   ```

4. Run `hearken release`. The first time, it makes the game's update key
   and prints a line to commit:

   ```toml
   update-key = "ed25519:3d40…"
   ```

   Commit it. From then on, a release signed with any other key is refused.

## What players get

- **A check at start**, off the game loop, when the game starts. If there
  is a newer version, the game says so once, queued behind what it is
  saying: "My Game version 1.4.0 is available. To install it, choose Check
  for updates in Settings." The player can turn the check at start off.
- **Settings rows**: "Check for updates" (it reads "Install version 1.4.0"
  once the check at start found one), "Update channel" when the game has
  more than one, and "Check for updates at start".
- **The offer**: the version, its size and the player's own version, then
  Download and restart, What's new, Remind me later, and Skip this version.
  A skipped version is not offered at start again; a later one is, and the
  player can still install the skipped one from Settings.
- **What's new**: the release notes, a line at a time with Up and Down, or
  all at once with Enter.
- **The download**: spoken at 25, 50 and 75 percent; Tab reads progress;
  Escape cancels at once and leaves the game as it was. A transfer that
  goes quiet for a minute stops and says so. Once it is checked and
  unpacked, the game starts the swap, says "Installing the update. The game
  closes and starts again by itself.", and quits.
- When this copy cannot update itself (a folder it cannot write to, a Mac
  app opened straight from a download), the update is saved somewhere the
  player can find, the home folder for an app or an AppImage, and the game
  says where.

These copies never update themselves, and say why when asked:

- developer builds: anything `hearken release` did not build with an update
  key, including `cargo run` and `hearken play` from source;
- a program run from a Cargo `target` folder, even a release build;
- iOS, where the App Store or TestFlight does it;
- a copy started by AG Now (`AGNOW` set to anything but `0`), which updates
  its games itself.

## Publishing a release

Each platform's release is built on its own computer or CI job, so each
writes its own manifest. `hearken release` writes, in `dist/`:

- the archive, `my-game-1.4.0-windows-x64.zip` (or `linux-x64.tar.gz`,
  `macos-arm64.zip`, `macos.zip` for Intel Macs);
- its signed manifest, `update-stable-windows-x64.json`.

`--channel=beta` publishes on another channel, and `--notes=NOTES.md` puts
release notes (Markdown) in the manifest.

```sh
hearken release --channel=stable --notes=NOTES.md
```

Upload both files, on every platform, under the names `hearken release`
gave them:

- **GitHub**: as assets of one release. A player's copy reads the
  repository's newest 100 releases, newest first, and takes the first that
  has its channel's manifest, so a beta and a stable release can sit side
  by side. Drafts are skipped. For GitHub Enterprise, give the source its
  API: `GitHubReleases::new(owner, repo)?.api("https://git.example.com/api/v3")`
  with `UpdateConfig::new`.
- **Any HTTPS host**: in the folder the game's manifest URL points at. The
  URL may contain `{platform}`, `{channel}` and `{game}` (the package
  name), filled in on each check, so
  `https://example.com/my-game/update-{channel}-{platform}.json` finds
  `update-stable-linux-x64.json` there. The archive is fetched from the
  same folder. Overwrite the manifest with each release.

A manifest may also name its archive's full HTTPS URL (`url`), to keep the
archives on another host; `hearken release` leaves it out.

## Keys

The update key is the game's own and stays the same for good: every copy a
player has trusts only manifests signed with it.

- The secret is in the developer's config folder,
  `hearken/update-keys/<package>.key`, made by the first `hearken release`.
  **Back it up.** If it is lost, players' copies accept no later update,
  and they must download one by hand.
- On CI, store its 64 hex digits (`hearken update-key --print` shows them)
  as a secret and pass it as `HEARKEN_UPDATE_SECRET`. Keys are never made
  on CI.
- The public half is committed as `update-key`, and `hearken release`
  builds it into the game through `hearken-update`'s build script
  (`HEARKEN_UPDATE_KEY`, with the package, version and channel beside it).
- `hearken update-key` shows where the key is and whether it matches the
  committed `update-key`. Never commit the secret.

## The manifest

```json
{
  "format": 1,
  "key": "ed25519:3d40…",
  "signature": "…128 hex digits…",
  "signed": "{\"format\":1,\"game\":\"my-game\",\"version\":\"1.4.0\",…}"
}
```

`signed` is the manifest itself, as a string; the Ed25519 signature covers
its exact bytes after the fixed prefix `hearken-update manifest v1\n`. Its
fields:

| Field | What it is |
|-------|------------|
| `format` | `1`. |
| `game` | The package name. |
| `version` | The release's version, as in `Cargo.toml`. |
| `channel` | The channel it is published on. |
| `platform` | `windows-x64`, `linux-x64`, `linux-arm64`, `macos-arm64` or `macos`. |
| `archive` | The archive's file name: letters, digits, `-`, `_`, `.`, `+`. |
| `url` | Optional: the archive's HTTPS URL, when it is not beside the manifest. |
| `size` | The archive's size in bytes. |
| `sha256` | The archive's SHA-256, in lowercase hex. |
| `notes` | Release notes, as Markdown. |
| `published` | Optional: when it was released, RFC 3339. |

`key` only tells a developer which key signed a file; the game ignores it.

A copy refuses a manifest that is unsigned, signed with another key,
malformed, for another game, platform or channel, or not newer than the
running version. Versions compare as Cargo's do: `1.5.0-beta.2` is older
than `1.5.0`, and build metadata (`+linux`) is ignored. An older version is
never installed, so a host, a mirror or anyone in between cannot roll a
player back to a version with a known flaw.

## How an update is installed

1. The archive is downloaded, never more than its signed size, and its
   SHA-256 is checked against the signed one before anything opens it. A
   server that declares another size is refused at once.
2. It is unpacked into a new folder under the system's temporary folder,
   as untrusted input even though it is signed: no entry may have an
   absolute path, a drive, `..` or `:`; only files, folders and links are
   allowed, never devices, pipes or hard links; a link must point down from
   where it is, and nothing is written through one; at most 200,000 entries
   and 32 GB, counted as they are written. `__MACOSX` and `._` files from
   macOS are skipped. The update must hold the running program under the
   same name, or it is refused.
3. A small script is written beside that folder and started detached. It
   waits for the game's process to exit (up to five minutes, then gives up
   without touching anything), swaps the update in, starts the game again
   and deletes itself.

   - **A folder install** (Windows, Linux): the install is copied to a
     backup first, and if that fails, nothing is touched. The update is then
     copied over the install, and if that fails, the backup is copied back.
     The `saves` folder of a portable copy is never copied either way. On
     Windows the script uses
     robocopy, which refuses `\\?\` paths, so they are written in their
     plain form; a `%` in a path is doubled for cmd, and `$`, backticks,
     backslashes and quotes are escaped for sh.
   - **A Mac app**: the whole app is swapped. The old app is moved aside
     (`My Game.app.old`) until the new one is in place, and moved back if it
     is not. An app running from App Translocation (opened straight from a
     quarantined download) is never swapped: the player is told where the
     new one is saved instead.
   - **An AppImage**: the file itself is swapped, by copying the new one in
     beside it and renaming it over the old one. The mount it runs from is
     never touched. `hearken release` does not make AppImages; a game that
     ships one is updated this way.

## Accepted limits

- **Whoever has the secret key can publish updates.** Anything it signs,
  every player's copy will install. Keep it out of the repository,
  back it up offline, and give CI only the secret it needs. There is no key
  rotation: a new key needs players to download a release by hand.
- **Freezing.** A host, or anyone between it and the player, can keep
  serving an old manifest, or none. The game then offers nothing newer; it
  never installs anything older. Manifests carry no expiry.
- **The same version is never offered again.** Rebuilding `1.4.0` and
  publishing it under the same version reaches no one who has `1.4.0`; bump
  the version.
- **Moving from a beta to stable waits for a newer stable.** A player on
  `1.5.0-beta.2` who switches to stable is offered `1.5.0` once it is out,
  not `1.4.3`.
- **GitHub reads the newest 100 releases.** A channel whose last release is
  older than that is not found.
- **Files a release drops stay behind** in a folder install: the update is
  copied over the install, never mirrored, so the player's own files there
  are never deleted. A failed copy is put back from the backup, but files
  that only the new version has may remain.
- **Windows waits by process ID**, polling `tasklist` once a second. If the
  game's ID is taken by another process within that second, the script
  waits for that one too, up to five minutes.
- **The whole archive downloads every time.** There are no partial or delta
  updates, and a stopped download starts again from the beginning.
- **Extended attributes are dropped** from a Mac app's files when it is
  unpacked. Hearken apps keep nothing there, and their code signature does
  not cover them.
- **Links in an update must point down** (no `..`). A Mac framework's usual
  links do; anything else is refused.
