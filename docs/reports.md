# Problem reports

A Hearken game can take a problem report from the player and email it to
the developer: what happened in the player's words, the game's version and
the operating system, and the session log, with the previous session's log
when there is one. A run that never exits cleanly is noticed at the next
start, and the game offers to report it.

The game posts the report to a relay, a small web service the developer
runs, and the relay sends the email. The game never holds a mail password,
and the relay only ever emails one address, so neither is any use to a
spammer. The report type, its limits and the sending are in
`hearken-report`, which has no window or speech code. The screens are in
`hearken-app`, behind its `report` feature.

The format is the one [mailbag](https://github.com/accessware/mailbag)
(MIT, Quin Gillespie) speaks, so a `mailbag-server` takes Hearken reports
as they are, and a relay written for Hearken takes mailbag's.

## Turning it on

1. Turn on `hearken`'s `report` feature in the game's `Cargo.toml`.

2. Say where reports go, from `Game::reports`:

   ```rust
   use hearken::ReportConfig;

   impl hearken::Game for MyGame {
       // ...
       fn reports() -> Option<ReportConfig> {
           ReportConfig::new("My Game", env!("CARGO_PKG_VERSION"),
               "https://example.com/api/problem-reports")
       }
   }
   ```

   The first argument is the game's name as players hear it and as the
   relay knows it. The endpoint must be HTTPS; `http://127.0.0.1` and
   `http://localhost` are allowed for trying a relay on the same computer.
   Optional settings:

   - `.commit("3d40a1f")`: the commit the game was built from, for the
     report's details.
   - `.no_offer_after_crash()`: never asks to report a crash. The player
     can still report from Settings.

3. Run a relay, or point the game at one that already takes its reports
   (see Running a relay).

## What players get

- **Report a problem**, in Settings. The screen has a row for what
  happened, a row for an email address if the player wants a reply, a
  switch for whether the log goes along (on when a log exists), Read the
  report, and Send. Each text row opens the usual text field, so what was
  typed can be reviewed a character at a time. Read the report speaks
  everything that will be sent, a line at a time, and names each attached
  log with its size. Send says "Sending the report." and, when the relay
  takes it, "Thank you. Your report was sent." and closes the screen.
- **An offer after a crash.** When the last run ended without exiting
  cleanly (a crash, a kill, a power cut), the next start asks, once the
  game is on a screen that allows an offer: "My Game closed unexpectedly
  the last time it ran. Report the problem?" Yes opens the report screen,
  which says what it is reporting; Not now closes the question; Don't ask
  again turns the offer off, and Settings gets a row to turn it back on.
  A report sent from there is marked as a crash, and the previous
  session's log, the one with the crash in it, goes along.
- **Nothing waits on the network.** Sending runs on a worker thread. The
  screen speaks the outcome when it arrives, and Escape leaves at once.
- **What a refusal does.** An address whose domain takes no mail puts the
  player back on the address row with "Email cannot be delivered to that
  address. Check it, or leave it blank." When the relay has taken its
  limit for the day, or cannot be reached, the game offers to save the
  report as `problem-report.txt` in the data folder and says where, so it
  can be sent another way.

A game puts the screen wherever it likes with
`ctx.push(hearken::screens::ReportScreen::new())`. Mark the home or title
screen with `allows_update_prompt() -> true` so the offer after a crash
can appear there; it waits while any other screen is open. Tests
(`Headless`) and agent sessions never send a report: an agent session's
sandbox is not a player's game.

## What a report holds

The report is one JSON object, gzipped, posted with `Content-Type:
application/json` and `Content-Encoding: gzip`:

| Field | What it is |
| --- | --- |
| `app` | The game's name, up to 64 characters. |
| `version` | The game's version, up to 64 characters. |
| `commit` | The commit it was built from, if the game gave one. |
| `os` | The operating system and processor, such as `windows x86_64`. |
| `description` | What the player wrote, up to 20,000 characters. |
| `reply_to` | The player's email address, if given. |
| `crashed` | Whether the report follows a crash. |
| `attachments` | Up to four `{ name, text }` text files. A name is letters, digits, `.`, `-` and `_`; a text is at most 1 MB. A longer log keeps its end. |

The relay answers `202` when the email went out, `400` for a malformed
report, `403` for a game it does not take or an address it blocks, `413`
for a report that is too big, `422` when the reply address's domain takes
no mail, `429` when a limit is reached, and `502` when the email could not
be sent. `Report::validate` checks the same limits before anything is
posted.

## Running a relay

A relay is whatever takes that JSON and sends an email: a Next.js route
on the game's own site, a Cloudflare Worker, or `mailbag-server` on a
machine of your own. Keep these rules whichever you run:

- Take reports only for the games you name, and never email anyone but
  yourself. The player's address goes in `Reply-To`, so replying answers
  them; it is never sent to.
- Limit reports per client address and per day, and block addresses that
  abuse it. The limits are what make a public endpoint safe to publish in
  a game.
- Check that the reply address's domain takes mail (an MX lookup), so a
  typo is caught while the player can still fix it. If the lookup fails,
  take the report anyway.
- Attach each log as a text file. Put the subject the report gives in the
  email's subject, such as `[My Game 1.4.0] crash: The window closed`.

orinks.net runs one at `https://orinks.net/api/problem-reports` for the
games it hosts, through Resend; its code is in the site's repository.
