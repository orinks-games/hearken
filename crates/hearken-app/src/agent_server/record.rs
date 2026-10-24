//! `record_start` and `record_stop`: a playtest kept as two files side by
//! side. A WAV of the final mix, what the player hears, when the sound
//! backend can record; and a text log, timed from the same start, of every
//! spoken line, every sound that started (with where it played), and every
//! key: the agent's and any a person pressed in the window.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::bindings::Chord;
use crate::clock;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::InputEvent;
use hearken_audio::{Audio, Heard};

pub(super) struct Recording {
    log: BufWriter<File>,
    log_path: PathBuf,
    /// `None` when the backend could not record sound.
    wav: Option<PathBuf>,
    /// The agent server's real-time clock at the start, so the log's times
    /// line up with the WAV, lockstep or not.
    started: f64,
}

/// Where a recording goes: `asked` without its extension, or a dated name
/// in a `recordings` folder beside the game.
fn base_path(asked: Option<&str>, game: &str) -> PathBuf {
    if let Some(p) = asked.filter(|p| !p.trim().is_empty()) {
        return Path::new(p.trim()).with_extension("");
    }
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
        .join("recordings");
    let t = clock::now_or_utc();
    let stem = format!(
        "{game}-{:04}-{:02}-{:02}_{:02}-{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    );
    let mut base = dir.join(&stem);
    let mut n = 2;
    while base.with_extension("log").exists() || base.with_extension("wav").exists() {
        base = dir.join(format!("{stem}-{n}"));
        n += 1;
    }
    base
}

impl Recording {
    /// Starts both files; the reply says where they are.
    pub(super) fn start<G: Game>(
        ctx: &mut Ctx<G>,
        asked: Option<&str>,
        now: f64,
    ) -> Result<(Recording, String), String> {
        let base = base_path(asked, G::DATA_DIR);
        if let Some(dir) = base.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let log_path = base.with_extension("log");
        let file = File::create(&log_path)
            .map_err(|e| format!("could not create {}: {e}", log_path.display()))?;
        let mut log = BufWriter::new(file);
        let wav_path = base.with_extension("wav");
        let (wav, note) = match ctx.audio.start_capture(&wav_path) {
            Ok(()) => (Some(wav_path), String::new()),
            Err(e) => (None, format!(" No sound file: {e}.")),
        };
        let _ = writeln!(
            log,
            "# {} playtest, times in seconds from the start.{note}",
            G::TITLE
        );
        let mut reply = format!("recording: {}", log_path.display());
        if let Some(w) = &wav {
            reply.push_str(&format!("\nsound: {}", w.display()));
        }
        reply.push_str(&note);
        log::info!("agent server: {reply}");
        let rec = Recording {
            log,
            log_path,
            wav,
            started: now,
        };
        Ok((rec, reply))
    }

    fn line(&mut self, clock: f64, text: &str) {
        let _ = writeln!(self.log, "[{:9.3}] {text}", clock - self.started);
    }

    pub(super) fn said(&mut self, clock: f64, line: &str) {
        self.line(clock, &format!("said: {line}"));
    }

    pub(super) fn sound(&mut self, clock: f64, s: &Heard) {
        self.line(clock, &format!("sound: {}", describe_sound(s)));
    }

    /// A key down or up, from the agent or from a person at the window.
    pub(super) fn key(&mut self, clock: f64, who: &str, event: &InputEvent) {
        let (what, key, mods) = match event {
            InputEvent::KeyDown {
                key,
                mods,
                repeat: false,
                ..
            } => ("down", *key, *mods),
            InputEvent::KeyUp { key, mods } => ("up", *key, *mods),
            _ => return,
        };
        let name = Chord { key, mods }
            .saved()
            .unwrap_or_else(|| format!("{key:?}"));
        self.line(clock, &format!("{who} key {what}: {name}"));
    }

    /// Ends both files; the reply says where they are.
    pub(super) fn stop(mut self, clock: f64, audio: Option<&mut Audio>) -> String {
        self.line(clock, "end");
        let _ = self.log.flush();
        let mut reply = format!("recorded: {}", self.log_path.display());
        match audio.and_then(|a| a.stop_capture()) {
            Some(Ok(wav)) => reply.push_str(&format!("\nsound: {}", wav.display())),
            Some(Err(e)) => reply.push_str(&format!("\nThe sound file did not finish: {e}")),
            // Already finished by the audio shutdown, or never started.
            None => {
                if let Some(wav) = &self.wav {
                    reply.push_str(&format!("\nsound: {}", wav.display()));
                }
            }
        }
        log::info!("agent server: {reply}");
        reply
    }
}

/// A started sound as the agent hears of it: its key, "(loop)" for a loop,
/// and where it plays when the game placed it.
pub(super) fn describe_sound(s: &Heard) -> String {
    let mut text = s.key.clone();
    if s.looping {
        text.push_str(" (loop)");
    }
    if let Some(p) = s.pos {
        text.push_str(&format!(" at ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_recording_keeps_its_folder_and_drops_its_extension() {
        let b = base_path(Some("out/take.wav"), "game");
        assert_eq!(b, Path::new("out/take"));
    }

    #[test]
    fn a_default_recording_is_dated_under_recordings() {
        let b = base_path(None, "wickets");
        let name = b.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("wickets-20"), "{name}");
        assert_eq!(b.parent().unwrap().file_name().unwrap(), "recordings");
    }

    #[test]
    fn a_sound_says_where_it_played() {
        let s = Heard {
            key: "bell".into(),
            pos: Some(hearken_audio::Vec3::new(-2.5, 1.0, 0.0)),
            looping: true,
        };
        assert_eq!(describe_sound(&s), "bell (loop) at (-2.5, 1.0, 0.0)");
    }
}
