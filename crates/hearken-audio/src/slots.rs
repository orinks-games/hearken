//! The ambience bed and music slots, pause-all/resume-all, and the duck.

use crate::pool::{clamped_or, slot_mut, Audio};
use crate::types::Bus;
use std::collections::HashSet;

impl Audio {
    /// Sets the ambience bed, crossfading the old one out and the new one in
    /// over `fades.bed_ms`. The same key already playing does nothing;
    /// `None` fades the bed out.
    pub fn set_bed(&mut self, key: Option<&str>) {
        if let Some(k) = key {
            // Checked against the liveness-filtered `bed()`, not the raw
            // stored key: a bed whose handle already died (reaped by
            // `update`) must be restartable with the same key, not treated
            // as still playing.
            if self.bed() == Some(k) {
                return;
            }
        }
        let fade = self.fades.bed_ms;
        if let Some((_, old)) = self.bed.take() {
            self.stop(old, fade);
        }
        let Some(k) = key else {
            return;
        };
        let sound = self.start_at_volume(k, Bus::AMBIENCE, true, 0.0);
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.volume = 1.0;
        }
        self.resend(sound, fade);
        self.bed = Some((k.to_string(), sound));
    }

    /// The ambience bed's key, while it is still playing.
    pub fn bed(&self) -> Option<&str> {
        self.bed
            .as_ref()
            .filter(|(_, sound)| self.is_playing(*sound))
            .map(|(k, _)| k.as_str())
    }

    /// Plays music on `Bus::MUSIC`, fading the old track out over
    /// `fades.music_ms` while the new one starts at full volume. The same
    /// key and loop flag already playing does nothing; `None` stops it.
    pub fn play_music(&mut self, key: Option<&str>, looping: bool) {
        if let Some(k) = key {
            // Checked against the liveness-filtered `music()`, not the raw
            // stored key: a track that already ended and was reaped must be
            // restartable with the same key, not treated as still playing.
            if self.music() == Some(k)
                && self.music.as_ref().is_some_and(|(_, ml, _)| *ml == looping)
            {
                return;
            }
        }
        let fade = self.fades.music_ms;
        if let Some((_, _, old)) = self.music.take() {
            self.stop(old, fade);
        }
        let Some(k) = key else {
            return;
        };
        let sound = if looping {
            self.start_loop(k, None, Bus::MUSIC)
        } else {
            self.play(k, Bus::MUSIC)
        };
        self.music = Some((k.to_string(), looping, sound));
    }

    /// The music's key, while it is still playing.
    pub fn music(&self) -> Option<&str> {
        self.music
            .as_ref()
            .filter(|(_, _, sound)| self.is_playing(*sound))
            .map(|(k, _, _)| k.as_str())
    }

    /// Pauses every live sound that is not already paused, remembering that
    /// this call paused it so `resume_all` resumes only that.
    pub fn pause_all(&mut self) {
        let sounds = self.live_sounds(|live| !live.paused);
        for sound in sounds {
            // `pause` clears `paused_by_all` (an explicit pause is never
            // pause_all's doing), so this call sets it back to true right
            // after, marking it as this call's doing instead.
            self.pause(sound);
            if let Some(live) = slot_mut(&mut self.slots, sound) {
                live.paused_by_all = true;
            }
        }
    }

    /// Resumes exactly what the last `pause_all` paused.
    pub fn resume_all(&mut self) {
        let sounds = self.live_sounds(|live| live.paused_by_all);
        for sound in sounds {
            self.resume(sound);
        }
    }

    /// Sets which buses the duck lowers. Re-sends (over `fades.duck_ms`)
    /// every sound on a bus that entered or left the set, so a change made
    /// during an active duck leaves no sound at a stale gain.
    pub fn set_duck_buses(&mut self, buses: &[Bus]) {
        let new_buses: HashSet<Bus> = buses.iter().copied().collect();
        let changed: HashSet<Bus> = self
            .mixing
            .duck_buses
            .symmetric_difference(&new_buses)
            .copied()
            .collect();
        self.mixing.duck_buses = new_buses;
        if changed.is_empty() {
            return;
        }
        let duck_ms = self.fades.duck_ms;
        let sounds = self.live_sounds(|live| changed.contains(&live.bus));
        for sound in sounds {
            self.resend(sound, duck_ms);
        }
    }

    /// Lowers the ducked buses to `level`, clamped to `0..=1` (`1.0` is no
    /// duck), ramping every sound on a ducked bus over `fades.duck_ms`.
    pub fn set_duck(&mut self, level: f32) {
        self.mixing.duck_level = clamped_or(self.mixing.duck_level, level, 0.0, 1.0);
        let duck_ms = self.fades.duck_ms;
        let sounds = self.live_sounds(|live| self.mixing.duck_buses.contains(&live.bus));
        for sound in sounds {
            self.resend(sound, duck_ms);
        }
    }

    /// The duck's current level.
    pub fn duck(&self) -> f32 {
        self.mixing.duck_level
    }
}
