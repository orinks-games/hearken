//! The pool's side of the acoustic path: which sounds get it, the scene
//! last seen, the room round the listener, and obstacle sense.

use std::sync::Arc;

use hearken_acoustics::{
    estimate_reverb, propagate, ReverbEstimate, Scene, SoundRender, BAND_CENTRES_HZ,
};

use super::sense::{ObstacleSense, FULL_LEVEL};
use super::voice::{speaker_cue, AcousticVoice, Post};
use super::{sense_sectors, to_model, AcousticSettings};
use crate::pool::{Audio, Live, Sound};
use crate::spatial::SpatialConfig;
use crate::types::{Bus, Listener, Vec3};

/// What an acoustic sound's mix carries in place of the plain spatial
/// model's distance gain, pan and low-pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AcousticMix {
    pub gain: f32,
    pub pan: f32,
    pub lowpass_hz: Option<f32>,
}

impl AcousticMix {
    /// For a backend that renders the model itself: the insert does it all.
    const NEUTRAL: AcousticMix = AcousticMix {
        gain: 1.0,
        pan: 0.0,
        lowpass_hz: None,
    };
}

/// What a sound's last render was worked out from, so an unchanged one is
/// not worked out again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AcousticKey {
    pos: Vec3,
    listener: Listener,
    scene: usize,
    generation: u64,
}

pub(crate) struct AcousticState {
    pub settings: AcousticSettings,
    /// Bumped whenever anything a render depends on changes, beyond the
    /// positions.
    generation: u64,
    scene: Option<Arc<Scene>>,
    room: Option<ReverbEstimate>,
    room_at: Option<(Vec3, usize)>,
    sense: Option<(Arc<ObstacleSense>, Sound)>,
    #[cfg(feature = "hrtf")]
    hrtf: HrtfState,
}

#[cfg(feature = "hrtf")]
#[derive(Default)]
struct HrtfState {
    source: super::hrtf::HrtfSource,
    set: Option<super::hrtf::HrtfSet>,
    /// The rate a load failed at, so it is not tried every frame.
    failed_at: Option<u32>,
}

impl AcousticState {
    fn new(settings: AcousticSettings) -> AcousticState {
        AcousticState {
            settings,
            generation: 1,
            scene: None,
            room: None,
            room_at: None,
            sense: None,
            #[cfg(feature = "hrtf")]
            hrtf: HrtfState::default(),
        }
    }

    pub(crate) fn wants(&self, bus: Bus, pos: Option<Vec3>) -> bool {
        pos.is_some() && self.settings.buses.contains(&bus)
    }

    fn room(&self) -> Option<&ReverbEstimate> {
        if self.settings.room {
            self.room.as_ref()
        } else {
            None
        }
    }

    #[cfg(feature = "hrtf")]
    fn hrtf_set(&self) -> Option<&super::hrtf::HrtfSet> {
        if self.settings.hrtf {
            self.hrtf.set.as_ref()
        } else {
            None
        }
    }
}

fn ptr(scene: &Arc<Scene>) -> usize {
    Arc::as_ptr(scene) as usize
}

/// The nearest plain mix to a render, for a backend that cannot run the
/// insert: the gain over the middle bands, the pan, and a low-pass fitted
/// to how much the highs fell.
fn plain_mix(r: &SoundRender, cfg: &SpatialConfig) -> AcousticMix {
    let g = r
        .gains
        .map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
    let power: f32 = g[1..6].iter().map(|v| v * v).sum::<f32>() / 5.0;
    let mut gain = power.sqrt();
    let (pan, behind) = speaker_cue(r.direction, cfg);
    // The behind cue's broadband part is its lowest band.
    gain *= behind[0];
    let mut lowpass: Option<f32> = None;
    // A second-order low-pass at fc falls as 1 / sqrt(1 + (f / fc)^4).
    let fit = |lo: f32, hi: f32, f: f32| -> Option<f32> {
        if lo <= 0.0 {
            return None;
        }
        let ratio = (hi / lo).clamp(1e-4, 1.0);
        if ratio > 0.95 {
            return None;
        }
        let x = (1.0 / (ratio * ratio) - 1.0).max(1e-9);
        Some((f / x.powf(0.25)).clamp(100.0, 20_000.0))
    };
    let behind_ratio = behind[5] / behind[0].max(1e-6);
    if let Some(hz) = fit(g[2], g[5], BAND_CENTRES_HZ[5] as f32) {
        lowpass = Some(hz);
    }
    if let Some(hz) = fit(1.0, behind_ratio, BAND_CENTRES_HZ[5] as f32) {
        lowpass = Some(lowpass.map_or(hz, |l| l.min(hz)));
    }
    AcousticMix {
        gain,
        pan,
        lowpass_hz: lowpass,
    }
}

/// Works out `live`'s render again if anything it depends on moved.
fn refresh(
    live: &mut Live,
    scene: &Arc<Scene>,
    listener: Listener,
    st: &AcousticState,
    renders: bool,
    spatial: &SpatialConfig,
) {
    let Some(pos) = live.pos else {
        return;
    };
    let key = AcousticKey {
        pos,
        listener,
        scene: ptr(scene),
        generation: st.generation,
    };
    if live.acoustic_key == Some(key) {
        return;
    }
    let Some(voice) = live.acoustic.as_mut() else {
        return;
    };
    let prop = propagate(
        scene,
        to_model(pos),
        to_model(listener.pos),
        &st.settings.query,
    );
    let facing = if listener.facing.is_finite() {
        f64::from(listener.facing)
    } else {
        0.0
    };
    let r = prop.render(facing, st.room());
    #[allow(unused_mut)]
    let mut hrtf = false;
    #[cfg(feature = "hrtf")]
    if let (Some(set), Some(link)) = (st.hrtf_set(), voice.hrtf.as_mut()) {
        link.aim(set, r.direction);
        hrtf = true;
    }
    let s = &st.settings;
    let post = Post {
        doppler: s.doppler,
        delay: s.delay,
        reflections: s.reflections,
        reverb: s.room,
        hrtf,
        ramp_ms: s.ramp_ms,
    };
    voice.post(&r, post, spatial);
    live.acoustic_mix = Some(if renders {
        AcousticMix::NEUTRAL
    } else {
        plain_mix(&r, spatial)
    });
    live.acoustic_key = Some(key);
}

impl Audio {
    /// Turns the acoustic path on with `settings`, or off with `None`.
    ///
    /// On, every positioned sound started from now on a bus in
    /// `settings.buses` carries the acoustic insert, and
    /// [`Audio::update_acoustic`] renders it through the game's scene.
    /// Off, sounds already carrying it go back to the plain spatial model.
    pub fn set_acoustics(&mut self, settings: Option<AcousticSettings>) {
        match settings {
            Some(s) => match &mut self.acoustic {
                Some(st) => {
                    if st.settings != s {
                        st.settings = s;
                        st.generation += 1;
                        st.room_at = None;
                    }
                }
                None => self.acoustic = Some(AcousticState::new(s)),
            },
            None => {
                if let Some(st) = self.acoustic.take() {
                    if let Some((_, sound)) = st.sense {
                        self.stop(sound, 200);
                    }
                }
                self.backend.set_room(None);
                for slot in &mut self.slots {
                    if let Some(live) = slot.live.as_mut() {
                        bypass(live);
                    }
                }
            }
        }
    }

    /// The acoustic settings in force, if the path is on.
    pub fn acoustics(&self) -> Option<&AcousticSettings> {
        self.acoustic.as_ref().map(|st| &st.settings)
    }

    /// The HRTF set headphone rendering uses (the bundled MIT KEMAR one by
    /// default, or a SOFA file from the game). It loads at the backend's
    /// rate the next time it is needed; a file that will not load leaves
    /// sounds panned, with a warning in the log. Sounds already playing keep
    /// what they started with.
    #[cfg(feature = "hrtf")]
    pub fn set_hrtf(&mut self, source: super::hrtf::HrtfSource) {
        let st = self
            .acoustic
            .get_or_insert_with(|| AcousticState::new(AcousticSettings::default()));
        st.hrtf = HrtfState {
            source,
            set: None,
            failed_at: None,
        };
        st.generation += 1;
    }

    /// A voice for a sound about to start, rendered at once when a scene
    /// has been seen.
    pub(crate) fn prepare_acoustic(&mut self, live: &mut Live) {
        let Some(st) = &self.acoustic else {
            return;
        };
        if !st.wants(live.bus, live.pos) {
            return;
        }
        live.acoustic = Some(AcousticVoice::new(st.settings.max_delay_s));
    }

    /// Renders a just-started sound, now that its voice has its processor.
    pub(crate) fn first_render(&mut self, live: &mut Live) {
        let renders = self.backend.renders_acoustics();
        let (listener, spatial) = (self.listener, self.spatial);
        if let Some(st) = &self.acoustic {
            if let Some(scene) = st.scene.clone() {
                refresh(live, &scene, listener, st, renders, &spatial);
            }
        }
    }

    /// A processor for `voice`, with an HRTF renderer when headphone
    /// rendering is on and loaded.
    pub(crate) fn acoustic_processor(&self, voice: &mut AcousticVoice) -> super::AcousticDsp {
        #[allow(unused_mut)]
        let mut dsp = voice.processor();
        #[cfg(feature = "hrtf")]
        {
            voice.hrtf = None;
            if let Some(set) = self.acoustic.as_ref().and_then(|st| st.hrtf_set()) {
                match set.voice() {
                    Ok((link, h)) => {
                        voice.hrtf = Some(link);
                        dsp.hrtf = Some(h);
                    }
                    Err(e) => log::warn!("audio: no HRTF for a sound ({e})"),
                }
            }
        }
        dsp
    }

    /// Runs one frame through the acoustic model: as [`Audio::update`], but
    /// every positioned sound on an acoustic bus is rendered through
    /// `scene` (its delay, per-band gains, reflections and reverb send),
    /// and other positioned sounds are muffled when `scene` blocks their
    /// line of sight. Turns the path on with default settings if
    /// [`Audio::set_acoustics`] has not.
    ///
    /// Pass the same `Arc` while the scene is unchanged: a sound whose
    /// position, listener and scene are all as they were is not worked out
    /// again. To change the scene, make a new `Arc` (`Arc::make_mut` does,
    /// since the pool holds a clone).
    pub fn update_acoustic(&mut self, scene: &Arc<Scene>) {
        self.poll_open();
        self.backend.update();
        if self.acoustic.is_none() {
            self.acoustic = Some(AcousticState::new(AcousticSettings::default()));
        }
        let renders = self.backend.renders_acoustics();
        let listener = self.listener;
        self.update_room(scene, renders);
        #[cfg(feature = "hrtf")]
        self.load_hrtf(renders);

        let spatial = self.spatial;
        let Some(st) = self.acoustic.as_ref() else {
            return;
        };
        let mut freed = Vec::new();
        let mut sends = Vec::new();
        let lis = to_model(listener.pos);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(live) = slot.live.as_mut() else {
                continue;
            };
            if !live.paused && !self.backend.is_playing(live.channel) {
                freed.push(i as u32);
                continue;
            }
            let Some(pos) = live.pos else {
                continue;
            };
            if live.acoustic.is_some() {
                refresh(live, scene, listener, st, renders, &spatial);
            } else {
                live.acoustic_mix = None;
                live.occluded = !scene.line_of_sight(lis, to_model(pos));
            }
            let gain = crate::pool::bus_gain_of(&self.mixing, live.bus);
            let mix = crate::pool::compute_mix(&listener, &spatial, gain, live);
            if mix != live.sent {
                live.sent = mix;
                sends.push((live.channel, mix));
            }
        }
        for index in freed {
            self.free_slot(index);
        }
        let move_ms = self.fades.move_ms;
        for (channel, mix) in sends {
            self.backend.set_mix(channel, mix, move_ms);
        }
        self.update_sense(scene);
        if let Some(st) = self.acoustic.as_mut() {
            st.scene = Some(scene.clone());
        }
    }

    /// Estimates the room again when the listener moved far enough or the
    /// scene changed, and hands it to the backend's reverb.
    fn update_room(&mut self, scene: &Arc<Scene>, renders: bool) {
        let pos = self.listener.pos;
        let Some(st) = self.acoustic.as_mut() else {
            return;
        };
        if !st.settings.room {
            if st.room.take().is_some() || st.room_at.take().is_some() {
                st.generation += 1;
                self.backend.set_room(None);
            }
            return;
        }
        let stale = match st.room_at {
            Some((at, p)) => {
                p != ptr(scene) || f64::from((at - pos).length()) > st.settings.room_move_m
            }
            None => true,
        };
        if !stale {
            return;
        }
        let room = estimate_reverb(scene, to_model(pos), &st.settings.reverb);
        st.room = Some(room);
        st.room_at = Some((pos, ptr(scene)));
        st.generation += 1;
        if renders {
            self.backend
                .set_room(Some(&room.render(scene.speed_of_sound())));
        }
    }

    #[cfg(feature = "hrtf")]
    fn load_hrtf(&mut self, renders: bool) {
        let rate = self.backend.sample_rate();
        let Some(st) = self.acoustic.as_mut() else {
            return;
        };
        let (true, true, Some(rate)) = (st.settings.hrtf, renders, rate) else {
            return;
        };
        if st
            .hrtf
            .set
            .as_ref()
            .is_some_and(|s| s.sample_rate() == rate)
            || st.hrtf.failed_at == Some(rate)
        {
            return;
        }
        match super::hrtf::HrtfSet::load(&st.hrtf.source, rate) {
            Ok(set) => {
                log::info!("audio: HRTF set loaded at {rate} Hz");
                st.hrtf.set = Some(set);
                st.hrtf.failed_at = None;
            }
            Err(e) => {
                log::warn!("audio: HRTF set not loaded ({e}); sounds stay panned");
                st.hrtf.set = None;
                st.hrtf.failed_at = Some(rate);
            }
        }
        st.generation += 1;
    }

    /// Starts, re-aims or stops the obstacle-sense floor.
    fn update_sense(&mut self, scene: &Arc<Scene>) {
        let listener = self.listener;
        let Some(st) = self.acoustic.as_mut() else {
            return;
        };
        let strength = if st.settings.obstacle_sense.is_finite() {
            st.settings.obstacle_sense.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let radius = st.settings.sense_radius_m;
        let current = st.sense.take();
        if strength == 0.0 {
            if let Some((_, sound)) = current {
                self.stop(sound, 200);
            }
            return;
        }
        let sense = match current {
            Some((sense, sound)) if self.is_playing(sound) => {
                self.set_sense(sense.clone(), sound);
                sense
            }
            _ => {
                let sense = ObstacleSense::new();
                let feed: Arc<dyn crate::feed::Feed> = sense.clone();
                let sound = self.start_feed(feed, None, Bus::AMBIENCE);
                self.set_sense(sense.clone(), sound);
                sense
            }
        };
        let sectors = sense_sectors(scene, listener.pos, listener.facing, radius);
        sense.set(&sectors, FULL_LEVEL * strength);
    }

    fn set_sense(&mut self, sense: Arc<ObstacleSense>, sound: Sound) {
        if let Some(st) = self.acoustic.as_mut() {
            st.sense = Some((sense, sound));
        }
    }
}

/// Takes `live` off the model: its voice passes the sound through and its
/// mix goes back to the plain spatial model.
pub(crate) fn bypass(live: &mut Live) {
    if let Some(voice) = live.acoustic.as_mut() {
        voice.bypass();
    }
    live.acoustic_mix = None;
    live.acoustic_key = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    use hearken_acoustics::BAND_COUNT;

    #[test]
    fn plain_mix_fits_a_lowpass_to_lost_highs() {
        let cfg = SpatialConfig::default();
        let mut r = SoundRender {
            direction: [1.0, 0.0, 0.0],
            ..SoundRender::default()
        };
        r.gains = [0.5; BAND_COUNT];
        let m = plain_mix(&r, &cfg);
        assert!((m.gain - 0.5).abs() < 1e-6);
        assert!((m.pan - 1.0).abs() < 1e-6);
        assert_eq!(m.lowpass_hz, None);
        r.gains = [0.5, 0.5, 0.5, 0.3, 0.1, 0.05, 0.02];
        let m = plain_mix(&r, &cfg);
        let hz = m.lowpass_hz.unwrap();
        assert!((500.0..4000.0).contains(&hz), "{hz}");
        r.gains = [f32::NAN; BAND_COUNT];
        let m = plain_mix(&r, &cfg);
        assert_eq!(m.gain, 0.0);
    }
}
