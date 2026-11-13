//! When the main voice falls silent, by estimate and by the voice's word.
//!
//! The main channel (menus, `say`, `say_queued`) has no pacer: its lines
//! interrupt or queue and nothing decides their fate. A game that waits for
//! the player to hear a line still needs to know when it ends.
//! [`SpeechProjection`] keeps that one number, the projected clear time, by
//! the same rules the event pacer uses for its backlog: an interrupting line
//! starts it over at now plus the line's estimated duration
//! ([`PacerConfig::duration_s`]), a queued line extends it, and a stop
//! clears it. A voice that can say whether it is speaking corrects it
//! ([`SpeechProjection::observe_voice`]) by the pacer's rules, and one
//! more: while the voice's latest report says it is speaking and is no
//! older than `voice_report_fresh_s`, the channel reads busy whatever the
//! estimate says, so the gap between two reports never reads as silence.
//!
//! The caller passes the time to every call, so the projection runs on
//! whatever clock its owner does.

use crate::pacer::PacerConfig;

/// The projected clear time of one speech channel.
#[derive(Clone, Debug)]
pub struct SpeechProjection {
    config: PacerConfig,
    /// When the channel falls silent; `NEG_INFINITY` when nothing was said
    /// since the last clear.
    clear_at: f64,
    /// When a line was last handed to the voice.
    last_submit_at: f64,
    /// When the newest voice report was taken.
    reported_at: f64,
    /// Whether that report said the voice is speaking.
    voice_speaking: bool,
}

impl Default for SpeechProjection {
    fn default() -> Self {
        Self::new(PacerConfig::DEFAULT)
    }
}

impl SpeechProjection {
    /// A projection timed by `config`'s speaking rate and voice-report
    /// tuning.
    pub fn new(config: PacerConfig) -> Self {
        SpeechProjection {
            config,
            clear_at: f64::NEG_INFINITY,
            last_submit_at: f64::NEG_INFINITY,
            reported_at: f64::NEG_INFINITY,
            voice_speaking: false,
        }
    }

    /// The tuning it was made with.
    pub fn config(&self) -> &PacerConfig {
        &self.config
    }

    /// See [`PacerConfig::wide_char_weight`].
    pub fn set_wide_char_weight(&mut self, weight: f64) {
        self.config.wide_char_weight = weight;
    }

    /// An interrupting line: what was speaking is cut, so the channel is
    /// busy for this line alone.
    pub fn restart(&mut self, now: f64, text: &str) {
        self.clear_at = f64::NEG_INFINITY;
        self.extend(now, text);
    }

    /// A queued line: the channel is busy for it after what it holds.
    pub fn extend(&mut self, now: f64, text: &str) {
        self.last_submit_at = now;
        self.clear_at = now.max(self.clear_at) + self.config.duration_s(text);
    }

    /// The channel was silenced.
    pub fn clear(&mut self) {
        self.clear_at = f64::NEG_INFINITY;
        self.voice_speaking = false;
    }

    /// When the channel is projected to fall silent; `None` when nothing was
    /// said since it was last cleared.
    pub fn clear_at(&self) -> Option<f64> {
        self.clear_at.is_finite().then_some(self.clear_at)
    }

    /// Whether the channel is speaking at `now`: projected to be, or the
    /// voice said so in a report still fresh.
    pub fn busy(&self, now: f64) -> bool {
        now < self.clear_at || self.voice_speaking_at(now)
    }

    /// Whether the voice's latest report says it is speaking and is no
    /// older than `voice_report_fresh_s` at `now`.
    pub fn voice_speaking_at(&self, now: f64) -> bool {
        self.voice_speaking && now - self.reported_at <= self.config.voice_report_fresh_s
    }

    /// Whether a report from the voice would help at `now`: the projection
    /// runs, or ran out within `voice_linger_s`. Outside that window asking
    /// costs a screen-reader call for nothing. See
    /// [`crate::EventSpeechPacer::wants_voice_report`].
    pub fn wants_voice_report(&self, now: f64) -> bool {
        now < self.clear_at + self.config.voice_linger_s
    }

    /// Correct the projection with the voice's report, taken `age_s`
    /// seconds before `now`, by the pacer's rules
    /// ([`crate::EventSpeechPacer::observe_voice`]): silence after the
    /// start grace ends it now, and speech past its end holds it
    /// `voice_hold_s` past the report, so the voice is asked again. A
    /// speaking report also keeps [`Self::busy`] true for as long as it is
    /// fresh. A report older than the last one taken is ignored.
    pub fn observe_voice(&mut self, now: f64, speaking: bool, age_s: f64) {
        let observed_at = now - age_s.max(0.0);
        if observed_at < self.reported_at || !self.clear_at.is_finite() {
            return;
        }
        self.reported_at = observed_at;
        self.voice_speaking = speaking;
        if speaking {
            if now >= self.clear_at {
                self.clear_at = self.clear_at.max(observed_at + self.config.voice_hold_s);
            }
        } else if now < self.clear_at
            && observed_at >= self.last_submit_at + self.config.voice_start_grace_s
        {
            self.clear_at = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "Three customers wait at the counter, the first one in a hurry.";

    fn grace() -> f64 {
        PacerConfig::DEFAULT.voice_start_grace_s
    }

    #[test]
    fn an_interrupt_starts_over_and_a_queued_line_extends() {
        let mut p = SpeechProjection::default();
        let one = p.config().duration_s(LINE);
        p.restart(10.0, LINE);
        assert_eq!(p.clear_at(), Some(10.0 + one));
        p.extend(10.5, "Next.");
        let next = p.config().duration_s("Next.");
        assert_eq!(p.clear_at(), Some(10.0 + one + next));
        p.restart(11.0, "Next.");
        assert_eq!(p.clear_at(), Some(11.0 + next));
    }

    #[test]
    fn a_queued_line_after_silence_starts_from_now() {
        let mut p = SpeechProjection::default();
        p.extend(10.0, "Hi.");
        p.extend(50.0, "Hi.");
        assert_eq!(p.clear_at(), Some(50.0 + p.config().duration_s("Hi.")));
    }

    #[test]
    fn a_clear_ends_it_and_stops_asking_the_voice() {
        let mut p = SpeechProjection::default();
        assert!(!p.wants_voice_report(0.0), "nothing said, nothing to ask");
        p.restart(10.0, LINE);
        assert!(p.busy(10.5));
        assert!(p.wants_voice_report(10.5));
        p.clear();
        assert!(!p.busy(10.5));
        assert!(!p.wants_voice_report(10.5));
    }

    #[test]
    fn it_asks_the_voice_only_until_the_linger_runs_out() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, "Hi.");
        let end = p.clear_at().unwrap();
        assert!(p.wants_voice_report(end + 1.0));
        assert!(!p.wants_voice_report(end + p.config().voice_linger_s));
    }

    #[test]
    fn silence_after_the_grace_ends_it_and_inside_the_grace_does_not() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, LINE);
        p.observe_voice(10.0 + grace() / 2.0, false, 0.0);
        assert!(p.busy(10.3), "the voice may not have started yet");
        p.observe_voice(11.0, false, 0.0);
        assert!(!p.busy(11.0));
    }

    #[test]
    fn speaking_past_the_estimate_holds_it_while_the_report_is_fresh() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, "Hi.");
        let end = p.clear_at().unwrap();
        p.observe_voice(end + 1.0, true, 0.0);
        let fresh = p.config().voice_report_fresh_s;
        assert!(p.busy(end + 1.0 + fresh));
        assert!(p.wants_voice_report(end + 1.0 + fresh));
        assert!(
            !p.busy(end + 1.0 + fresh + 0.01),
            "reports stopped: the estimate"
        );
    }

    #[test]
    fn old_speaking_reports_in_a_row_never_read_as_silence() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, "Hi.");
        let mut now = 10.0;
        while now < 40.0 {
            if p.wants_voice_report(now) {
                p.observe_voice(now, true, 0.3);
            }
            assert!(p.busy(now), "not busy at {now}");
            assert!(p.busy(now + 0.05), "not busy between reports at {now}");
            now += 0.1;
        }
        p.observe_voice(now, false, 0.3);
        assert!(!p.busy(now));
    }

    #[test]
    fn a_clear_forgets_the_voice_speaking() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, LINE);
        p.observe_voice(11.0, true, 0.0);
        p.clear();
        assert!(!p.busy(11.0));
    }

    #[test]
    fn an_older_report_than_the_last_is_ignored() {
        let mut p = SpeechProjection::default();
        p.restart(10.0, LINE);
        p.observe_voice(11.0, true, 0.0);
        p.observe_voice(11.1, false, 0.5);
        assert!(p.busy(11.1));
    }

    #[test]
    fn a_report_with_nothing_said_changes_nothing() {
        let mut p = SpeechProjection::default();
        p.observe_voice(10.0, true, 0.0);
        assert!(!p.busy(10.0));
        assert_eq!(p.clear_at(), None);
    }
}
