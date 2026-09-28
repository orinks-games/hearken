//! The ambience bed and music slots, pause-all/resume-all, and the duck.

use crate::pool::{clamped_or, slot_mut, Audio, Sound};
use crate::types::Bus;

impl Audio {
    /// Sets the ambience bed, crossfading the old one out and the new one in
    /// over `fades.bed_ms`. The same key already playing does nothing;
    /// `None` fades the bed out.
    pub fn set_bed(&mut self, key: Option<&str>) {
        if let Some(k) = key {
            if self.bed.as_ref().is_some_and(|(bk, _)| bk == k) {
                return;
            }
        }
        let fade = self.fades.bed_ms;
        if let Some((_, old)) = self.bed.take() {
            self.stop(old, fade);
        }
        let Some(k) = key else {
            self.bed = None;
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

    /// Plays music on `Bus::MUSIC`, crossfading the old track out and the
    /// new one in over `fades.music_ms`. The same key and loop flag already
    /// playing does nothing; `None` stops it.
    pub fn play_music(&mut self, key: Option<&str>, looping: bool) {
        if let Some(k) = key {
            if self
                .music
                .as_ref()
                .is_some_and(|(mk, ml, _)| mk == k && *ml == looping)
            {
                return;
            }
        }
        let fade = self.fades.music_ms;
        if let Some((_, _, old)) = self.music.take() {
            self.stop(old, fade);
        }
        let Some(k) = key else {
            self.music = None;
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
        let sounds: Vec<Sound> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                (!live.paused).then_some(Sound {
                    index: i as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for sound in sounds {
            if let Some(live) = slot_mut(&mut self.slots, sound) {
                live.paused_by_all = true;
            }
            self.pause(sound);
        }
    }

    /// Resumes exactly what the last `pause_all` paused.
    pub fn resume_all(&mut self) {
        let sounds: Vec<Sound> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                live.paused_by_all.then_some(Sound {
                    index: i as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for sound in sounds {
            if let Some(live) = slot_mut(&mut self.slots, sound) {
                live.paused_by_all = false;
            }
            self.resume(sound);
        }
    }

    /// Sets which buses the duck lowers.
    pub fn set_duck_buses(&mut self, buses: &[Bus]) {
        self.mixing.duck_buses = buses.iter().copied().collect();
    }

    /// Lowers the ducked buses to `level`, clamped to `0..=1` (`1.0` is no
    /// duck), ramping every sound on a ducked bus over `fades.duck_ms`.
    pub fn set_duck(&mut self, level: f32) {
        self.mixing.duck_level = clamped_or(self.mixing.duck_level, level, 0.0, 1.0);
        let duck_ms = self.fades.duck_ms;
        let sounds: Vec<Sound> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                self.mixing.duck_buses.contains(&live.bus).then_some(Sound {
                    index: i as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for sound in sounds {
            self.resend(sound, duck_ms);
        }
    }

    /// The duck's current level.
    pub fn duck(&self) -> f32 {
        self.mixing.duck_level
    }
}
