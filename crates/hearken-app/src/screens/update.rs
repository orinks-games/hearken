//! The update screens: check, the offer, what's new, and the download.
//!
//! Everything slow runs on worker threads. The screens poll them each
//! frame and speak as things change, so the game, and the screen reader,
//! never wait on the network, and Escape always leaves at once.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use hearken_update::{DownloadError, Install, Limits, Phase, Staged, Update};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::sound::UiSound;
use crate::state::State;
use crate::text::end_sentence;
use crate::updates::Checker;

/// The archive's size as spoken: "12 megabytes".
pub fn megabytes(bytes: u64) -> String {
    let mb = (bytes as f64 / 1e6).round() as u64;
    match mb {
        0 => "less than 1 megabyte".to_string(),
        1 => "1 megabyte".to_string(),
        n => format!("{n} megabytes"),
    }
}

/// What downloading `update` takes, as spoken: "12 megabytes", or, when
/// this copy has some of its files already, "3 megabytes of changes".
pub fn download_size(update: &Update) -> String {
    let size = megabytes(update.download_size());
    if update.plan.is_some() {
        format!("{size} of changes")
    } else {
        size
    }
}

/// Checks for an update when the player asks, from Settings.
pub struct UpdateCheckScreen {
    checker: Option<Checker>,
    message: String,
    /// Whether it said it is finding which files the player already has.
    planning: bool,
}

/// What the check screen says while it hashes the player's files.
const PLANNING_LINE: &str = "Found an update. Checking which files you already have.";

impl UpdateCheckScreen {
    pub fn new() -> Self {
        UpdateCheckScreen {
            checker: None,
            message: String::new(),
            planning: false,
        }
    }

    fn status(&self) -> &str {
        if !self.message.is_empty() {
            &self.message
        } else if self.planning {
            PLANNING_LINE
        } else {
            "Checking for updates."
        }
    }
}

impl Default for UpdateCheckScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> State<G> for UpdateCheckScreen {
    fn name(&self) -> &'static str {
        "update_check"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        if let Some(line) = ctx.updates.blocked_line() {
            self.message = line;
            ctx.say(self.message.clone());
            return;
        }
        if self.checker.is_none() {
            let channel = ctx.updates.channel(&ctx.settings.updates);
            self.checker = ctx.updates.check(&channel);
            ctx.say("Checking for updates.");
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if !self.message.is_empty() {
            return;
        }
        let Some(outcome) = self.checker.as_mut().and_then(|c| c.poll()).cloned() else {
            if !self.planning && self.checker.as_ref().is_some_and(|c| c.planning()) {
                self.planning = true;
                ctx.say(PLANNING_LINE);
            }
            return;
        };
        match outcome {
            Ok(Some(update)) => {
                ctx.replace(UpdatePromptScreen::<G>::new(update));
                return;
            }
            Ok(None) => {
                let current = ctx.updates.spoken_current().unwrap_or_default();
                self.message = format!(
                    "Up to date. You have {} version {current}.",
                    ctx.updates.name()
                );
            }
            Err(line) => {
                ctx.play_ui(UiSound::Error);
                self.message = format!("{} Try again later.", end_sentence(&line));
            }
        }
        ctx.say(self.message.clone());
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        if matches!(
            event.pressed(),
            Some(Key::Escape | Key::Return | Key::KpEnter)
        ) {
            ctx.play_ui(UiSound::Back);
            ctx.pop();
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        vec![
            "Check for updates".to_string(),
            String::new(),
            self.status().to_string(),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        if self.message.is_empty() && self.planning {
            format!("{PLANNING_LINE} Escape stops and goes back.")
        } else if self.message.is_empty() {
            "Checking for a newer version. Escape goes back.".to_string()
        } else {
            format!("{} Escape goes back.", self.message)
        }
    }
}

/// Offers an update that was found.
pub struct UpdatePromptScreen<G: Game> {
    menu: MenuCore<Self, G>,
    pub update: Update,
}

impl<G: Game> UpdatePromptScreen<G> {
    pub fn new(update: Update) -> Self {
        UpdatePromptScreen {
            menu: MenuCore::new("Update available").with_intro_help(
                "Download and restart installs the update now. What's new reads the changes. \
                 Skip this version stops offering it.",
            ),
            update,
        }
    }

    fn skip(&mut self, ctx: &mut Ctx<G>) {
        ctx.settings.updates.skipped = Some(self.update.version().to_string());
        ctx.save_settings();
        ctx.updates.forget();
        let line = format!(
            "Skipped version {}. Later versions are still offered.",
            hearken_update::spoken_version(self.update.version())
        );
        ctx.pop_with(line);
    }
}

impl<G: Game> Menu for UpdatePromptScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "update_prompt"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn announce(&mut self, ctx: &mut Ctx<G>) {
        let current = ctx
            .updates
            .spoken_current()
            .map(|v| format!(" You have version {v}."))
            .unwrap_or_default();
        ctx.say(format!(
            "Update available: {}, {}.{current}",
            ctx.updates.title(&self.update),
            download_size(&self.update)
        ));
        let row = self.current_text(ctx);
        ctx.say_queued(row);
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        vec![
            MenuItem::new("Download and restart", |s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.replace(UpdateDownloadScreen::new(s.update.clone()))
            })
            .help("Downloads the update, then closes the game and starts the new version."),
            MenuItem::new("What's new", |s: &mut Self, ctx: &mut Ctx<G>| {
                let title = ctx.updates.title(&s.update);
                ctx.push(WhatsNewScreen::new(title, s.update.manifest.note_lines()))
            })
            .help("The changes in this update, a line at a time."),
            MenuItem::new("Remind me later", |_s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.play_ui(UiSound::Back);
                ctx.pop()
            })
            .select_sound(None)
            .help("Offers it again the next time the game starts."),
            MenuItem::new("Skip this version", |s: &mut Self, ctx: &mut Ctx<G>| s.skip(ctx))
                .help("Stops offering this version when the game starts. Later versions are still offered."),
        ]
    }
}

/// Reads an update's notes a line at a time.
pub struct WhatsNewScreen {
    title: String,
    notes: Vec<String>,
    line: Option<usize>,
}

impl WhatsNewScreen {
    pub fn new(title: String, notes: Vec<String>) -> Self {
        let notes = if notes.is_empty() {
            vec!["This update came with no notes.".to_string()]
        } else {
            notes
        };
        WhatsNewScreen {
            title,
            notes,
            line: None,
        }
    }
}

impl<G: Game> State<G> for WhatsNewScreen {
    fn name(&self) -> &'static str {
        "whats_new"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let count = match self.notes.len() {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        ctx.say(format!(
            "What's new in {}, {count}. Down and Up read a line, Enter reads them all, \
             Escape goes back.",
            self.title
        ));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(key) = event.pressed() else {
            return;
        };
        let last = self.notes.len() - 1;
        match key {
            Key::Escape => {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
            }
            Key::Down => {
                let next = self.line.map_or(0, |l| (l + 1).min(last));
                self.line = Some(next);
                ctx.say(self.notes[next].clone());
            }
            Key::Up => {
                let next = self.line.map_or(0, |l| l.saturating_sub(1));
                self.line = Some(next);
                ctx.say(self.notes[next].clone());
            }
            Key::Home => {
                self.line = Some(0);
                ctx.say(self.notes[0].clone());
            }
            Key::End => {
                self.line = Some(last);
                ctx.say(self.notes[last].clone());
            }
            Key::Return | Key::KpEnter | Key::Space => {
                let all: Vec<String> = self.notes.iter().map(|n| end_sentence(n)).collect();
                ctx.say(all.join(" "));
            }
            _ => {}
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        let mut out = vec![format!("What's new in {}", self.title), String::new()];
        for (i, text) in self.notes.iter().enumerate() {
            let marker = if Some(i) == self.line { "> " } else { "  " };
            out.push(format!("{marker}{text}"));
        }
        out
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "Down and Up read a line, Enter reads them all, Escape goes back.".to_string()
    }
}

/// What the download worker hands back.
#[derive(Default)]
struct Outcome {
    staging: Option<PathBuf>,
    staged: Option<Staged>,
    error: Option<String>,
}

const PHASE_DOWNLOADING: u8 = 0;
const PHASE_UNPACKING: u8 = 1;

/// Seconds without a byte before the screen gives up on the worker: the
/// worker's own idle bound, plus a moment for it to say why. The backstop
/// for a worker that cannot even report.
pub const STALL_TIMEOUT_S: f64 = hearken_update::IDLE_TIMEOUT.as_secs_f64() + 30.0;

/// Seconds an unpack may take on this screen's clock.
pub const UNPACK_TIMEOUT_S: f64 = 600.0;

/// Downloads and unpacks the update, then starts the swap and closes the
/// game. Escape leaves at once, whatever the worker is doing; a transfer
/// that goes quiet, or an unpack that runs too long, is given up with a
/// spoken reason. The game is unchanged until the swap starts.
pub struct UpdateDownloadScreen {
    update: Update,
    cancelled: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    outcome: Arc<Mutex<Outcome>>,
    /// Bytes received so far, and of how many: fewer than the archive when
    /// only the changed files download.
    bytes: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    phase: Arc<AtomicU8>,
    install: Option<Install>,
    spoken_quarter: u64,
    /// The total the screen last spoke progress against.
    seen_total: u64,
    started: bool,
    finished: bool,
    seen_bytes: u64,
    idle_s: f64,
    unpack_s: f64,
    unpack_spoken: bool,
}

impl UpdateDownloadScreen {
    pub fn new(update: Update) -> Self {
        let size = update.download_size();
        UpdateDownloadScreen {
            total: Arc::new(AtomicU64::new(size)),
            update,
            cancelled: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
            outcome: Arc::new(Mutex::new(Outcome::default())),
            bytes: Arc::new(AtomicU64::new(0)),
            phase: Arc::new(AtomicU8::new(PHASE_DOWNLOADING)),
            install: None,
            spoken_quarter: 0,
            seen_total: size,
            started: false,
            finished: false,
            seen_bytes: 0,
            idle_s: 0.0,
            unpack_s: 0.0,
            unpack_spoken: false,
        }
    }

    /// The share downloaded, 0 to 1.
    pub fn progress(&self) -> f64 {
        let size = self.total.load(Ordering::SeqCst).max(1);
        (self.bytes.load(Ordering::SeqCst) as f64 / size as f64).min(1.0)
    }

    fn percent(&self) -> u64 {
        (self.progress() * 100.0).floor() as u64
    }

    fn unpacking(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == PHASE_UNPACKING
    }

    fn outcome(&self) -> std::sync::MutexGuard<'_, Outcome> {
        self.outcome.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn start<G: Game>(&mut self, ctx: &mut Ctx<G>) {
        let install = ctx.updates.host().install();
        self.install = Some(install.clone());
        let fetch = ctx.updates.fetch();
        let prefix = ctx
            .updates
            .running()
            .map_or_else(|| "hearken".to_string(), |r| r.game.clone());
        let update = self.update.clone();
        let (cancelled, done, outcome, bytes, total, phase) = (
            self.cancelled.clone(),
            self.done.clone(),
            self.outcome.clone(),
            self.bytes.clone(),
            self.total.clone(),
            self.phase.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("update-download".into())
            .spawn(move || {
                let mut staging_dir = None;
                let result = (|| -> Result<Staged, DownloadError> {
                    let staging = hearken_update::make_staging_dir(&prefix)?;
                    staging_dir = Some(staging.clone());
                    outcome.lock().unwrap_or_else(|e| e.into_inner()).staging =
                        Some(staging.clone());
                    // Only the changed files when it can, else the whole
                    // archive: the same update staged either way.
                    let staged = hearken_update::download_and_stage(
                        &update,
                        fetch.as_ref(),
                        &staging,
                        &install,
                        Limits::default(),
                        &mut |done, of| {
                            total.store(of, Ordering::SeqCst);
                            bytes.store(done, Ordering::SeqCst);
                        },
                        &mut |now| {
                            let code = match now {
                                Phase::Downloading => PHASE_DOWNLOADING,
                                Phase::Unpacking => PHASE_UNPACKING,
                            };
                            phase.store(code, Ordering::SeqCst);
                        },
                        &cancelled,
                    )?;
                    if cancelled.load(Ordering::SeqCst) {
                        return Err(DownloadError::Cancelled);
                    }
                    Ok(staged)
                })();
                let mut out = outcome.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    // Canceled after all: nothing will install it.
                    Ok(_) if cancelled.load(Ordering::SeqCst) => {
                        if let Some(staging) = &staging_dir {
                            let _ = std::fs::remove_dir_all(staging);
                        }
                    }
                    Ok(staged) => out.staged = Some(staged),
                    Err(e) => {
                        // Nothing will use half an update: never leave it
                        // in the temporary folder.
                        if let Some(staging) = &staging_dir {
                            let _ = std::fs::remove_dir_all(staging);
                        }
                        if !cancelled.load(Ordering::SeqCst) {
                            log::warn!("update download: {e}");
                            out.error = Some(download_error_line(&e));
                        }
                    }
                }
                drop(out);
                done.store(true, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            log::warn!("could not start the update download: {e}");
            self.abandon(ctx, "The download could not start.");
        }
    }

    /// Tells the worker to stop, and removes an update it already finished:
    /// set under the same lock the worker stores into, so whichever comes
    /// second removes it.
    fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let mut out = self.outcome();
        if out.staged.take().is_some() {
            if let Some(staging) = out.staging.take() {
                let _ = std::fs::remove_dir_all(staging);
            }
        }
    }

    /// Leaves on a stalled or stuck worker, saying why and what to do. The
    /// worker is told to stop, and tidies up when it next wakes.
    fn abandon<G: Game>(&mut self, ctx: &mut Ctx<G>, reason: &str) {
        log::warn!("update abandoned: {reason}");
        self.cancel();
        self.finished = true;
        ctx.play_ui(UiSound::Error);
        ctx.say(unchanged_line(ctx, reason));
        ctx.pop_quiet();
    }

    /// The stall and unpack watchdogs, on this screen's own clock.
    fn watch<G: Game>(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        if !self.unpacking() && self.unpack_spoken {
            // Back to downloading: the whole update, after all.
            self.unpack_spoken = false;
            self.unpack_s = 0.0;
            self.idle_s = 0.0;
        }
        if self.unpacking() {
            if !self.unpack_spoken {
                self.unpack_spoken = true;
                ctx.say_queued("Download complete. Unpacking the update.");
            }
            self.unpack_s += dt;
            if self.unpack_s >= UNPACK_TIMEOUT_S {
                self.abandon(ctx, "Unpacking the update took too long.");
            }
            return;
        }
        let bytes = self.bytes.load(Ordering::SeqCst);
        if bytes != self.seen_bytes {
            self.seen_bytes = bytes;
            self.idle_s = 0.0;
            return;
        }
        self.idle_s += dt;
        if self.idle_s >= STALL_TIMEOUT_S {
            let reason = format!(
                "The download stopped. Nothing arrived for {} seconds.",
                STALL_TIMEOUT_S as u64
            );
            self.abandon(ctx, &reason);
        }
    }

    fn finish<G: Game>(&mut self, ctx: &mut Ctx<G>) {
        self.finished = true;
        if self.cancelled.load(Ordering::SeqCst) {
            self.cancel();
            ctx.pop_quiet();
            return;
        }
        let (staged, staging, error) = {
            let mut out = self.outcome();
            (out.staged.take(), out.staging.clone(), out.error.take())
        };
        let (Some(staged), Some(install), Some(staging)) = (staged, self.install.clone(), staging)
        else {
            let reason = error.unwrap_or_else(|| "The download did not finish.".to_string());
            ctx.play_ui(UiSound::Error);
            ctx.say(unchanged_line(ctx, &reason));
            ctx.pop_quiet();
            return;
        };
        if !hearken_update::can_auto_apply(&staged, &install) {
            // A folder this player cannot write to, an app opened straight
            // from a download: the swap would fail, so the update is put
            // somewhere findable and the player told where.
            let dest = hearken_update::stash_for_manual_install(
                &staged,
                &staging,
                ctx.updates.host().home.as_deref(),
            );
            ctx.say(format!(
                "Download complete, but this copy cannot update itself. The new version is \
                 saved at {}. Install it from there, then start the game again.",
                dest.display()
            ));
            ctx.pop_quiet();
            return;
        }
        if let Err(e) = ctx.updates.apply(&staged, &install, &staging) {
            // Quitting now would close the game with nothing to bring it
            // back. Stay, and hand the player the update instead.
            log::warn!("could not start the update's apply script: {e}");
            let dest = hearken_update::stash_for_manual_install(
                &staged,
                &staging,
                ctx.updates.host().home.as_deref(),
            );
            ctx.play_ui(UiSound::Error);
            ctx.say(format!(
                "Download complete, but the update could not install itself. The new version \
                 is saved at {}. Install it from there, then start the game again.",
                dest.display()
            ));
            ctx.pop_quiet();
            return;
        }
        ctx.say("Installing the update. The game closes and starts again by itself.");
        ctx.quit();
    }
}

/// `reason`, then that the game is unchanged and where to get the update.
fn unchanged_line<G: Game>(ctx: &Ctx<G>, reason: &str) -> String {
    let manual = ctx.updates.manual_line();
    let line = format!(
        "{} Your game is unchanged. Try again later. {manual}",
        end_sentence(reason)
    );
    line.trim_end().to_string()
}

/// What the player hears when the download or unpack stopped.
fn download_error_line(e: &DownloadError) -> String {
    match e {
        DownloadError::Cancelled => "The update was canceled.".to_string(),
        DownloadError::Stalled(idle) => format!(
            "The download stopped. Nothing arrived for {} seconds.",
            idle.as_secs()
        ),
        DownloadError::Corrupt => {
            "The download did not match the update it was signed for, so it was not installed."
                .to_string()
        }
        DownloadError::Fetch(_) => "The download could not reach the update server.".to_string(),
        DownloadError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "The update was missing a file this game needs, so it was not installed.".to_string()
        }
        DownloadError::Io(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            "The update could not be unpacked safely, so it was not installed.".to_string()
        }
        DownloadError::Io(_) => "The update could not be saved. The disk may be full.".to_string(),
    }
}

impl<G: Game> State<G> for UpdateDownloadScreen {
    fn name(&self) -> &'static str {
        "update_download"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        if self.started {
            return;
        }
        self.started = true;
        ctx.say(format!(
            "Downloading {}, {}. The game restarts when it is done. Escape cancels.",
            ctx.updates.title(&self.update),
            download_size(&self.update)
        ));
        self.start(ctx);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        if self.finished {
            return;
        }
        let total = self.total.load(Ordering::SeqCst);
        if total != self.seen_total {
            // Only the changed files did not work out, and the whole
            // update downloads instead, from the start.
            self.seen_total = total;
            self.spoken_quarter = 0;
            self.idle_s = 0.0;
            ctx.say_queued(format!(
                "Downloading the whole update instead, {}.",
                megabytes(total)
            ));
        }
        let quarter = (self.progress() * 4.0) as u64;
        if quarter > self.spoken_quarter && quarter < 4 {
            self.spoken_quarter = quarter;
            ctx.say_queued(format!("{} percent.", quarter * 25));
        }
        if self.done.load(Ordering::SeqCst) {
            self.finish(ctx);
        } else {
            self.watch(ctx, dt);
        }
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        if self.finished {
            return;
        }
        match event.pressed() {
            Some(Key::Escape) => {
                // Leave now, whatever the worker is doing; it stops and
                // tidies up when it next wakes.
                self.cancel();
                self.finished = true;
                ctx.play_ui(UiSound::Back);
                ctx.pop_with("Update canceled.");
            }
            Some(Key::Tab | Key::Return | Key::KpEnter | Key::Space) => {
                let text = if self.unpacking() {
                    "Unpacking the update.".to_string()
                } else {
                    format!("{} percent downloaded.", self.percent())
                };
                ctx.say(text);
            }
            _ => {}
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let status = if self.unpacking() {
            "Unpacking".to_string()
        } else {
            format!("{} percent", self.percent())
        };
        vec![
            format!("Downloading {}", ctx.updates.title(&self.update)),
            String::new(),
            status,
            "Escape cancels, Tab reads progress.".to_string(),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "Tab reads how much has downloaded. Escape cancels and leaves the game as it is."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::megabytes;

    #[test]
    fn sizes_are_spoken_in_megabytes() {
        assert_eq!(megabytes(400_000), "less than 1 megabyte");
        assert_eq!(megabytes(1_200_000), "1 megabyte");
        assert_eq!(megabytes(84_600_000), "85 megabytes");
    }
}
