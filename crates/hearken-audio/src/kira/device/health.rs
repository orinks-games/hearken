//! Whether a running stream is still playing, judged from the errors it
//! reports and from whether it keeps calling back for sound.
//!
//! The watcher feeds one [`Event`] at a time to [`Health::next`] and acts on
//! the [`Verdict`]; nothing else moves this state. It touches no device, so
//! the table at the bottom tests it without one.

use cpal::ErrorKind;

/// Checks with no callback before a stream counts as stalled: 2 s at the
/// watcher's 500 ms. Each reopen in one episode doubles it, up to
/// [`MAX_BACKOFF`] doublings, so a device that is slow to start gets more
/// room each time and is never reopened in a tight loop. A suspect error
/// cuts only the first wait of an episode to one check.
const STALL_CHECKS: u32 = 4;
const MAX_BACKOFF: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Health {
    /// The callback count seen at the last check.
    last_beats: u64,
    /// Checks in a row with no callback.
    quiet_checks: u32,
    /// An error that may have ended the stream arrived; one quiet check
    /// settles it, unless this episode already reopened.
    suspect: bool,
    /// Reopens for silence since sound last played.
    stalls: u32,
    /// A suspect error was logged; later ones go to debug until sound
    /// plays again after a stall.
    warned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Event {
    /// A new stream started; its callback count starts at zero.
    Opened,
    /// The stream reported an error.
    Error(ErrorKind),
    /// The watcher's periodic look at the callback count.
    Check { beats: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Keep,
    /// Keep, but log this error at warn: it may have ended the stream.
    Suspect,
    /// Keep, and log that sound came back after a stall.
    Resumed,
    /// Close the stream and open the default device again.
    Reopen(Why),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Why {
    /// The device reported it was gone.
    Lost,
    /// The stream stopped calling back.
    Stalled,
}

impl Health {
    pub(super) fn next(&mut self, event: Event) -> Verdict {
        match event {
            Event::Opened => {
                self.last_beats = 0;
                self.quiet_checks = 0;
                self.suspect = false;
                Verdict::Keep
            }
            Event::Error(ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated) => {
                Verdict::Reopen(Why::Lost)
            }
            // The stream carries on after these.
            Event::Error(
                ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged,
            ) => Verdict::Keep,
            Event::Error(_) => {
                self.suspect = true;
                if self.warned {
                    Verdict::Keep
                } else {
                    self.warned = true;
                    Verdict::Suspect
                }
            }
            Event::Check { beats } if beats != self.last_beats => {
                self.last_beats = beats;
                self.quiet_checks = 0;
                self.suspect = false;
                if self.stalls > 0 {
                    self.stalls = 0;
                    self.warned = false;
                    Verdict::Resumed
                } else {
                    Verdict::Keep
                }
            }
            Event::Check { .. } => {
                self.quiet_checks = self.quiet_checks.saturating_add(1);
                let limit = if self.suspect && self.stalls == 0 {
                    1
                } else {
                    STALL_CHECKS << self.stalls.min(MAX_BACKOFF)
                };
                if self.quiet_checks >= limit {
                    self.quiet_checks = 0;
                    self.stalls = self.stalls.saturating_add(1);
                    Verdict::Reopen(Why::Stalled)
                } else {
                    Verdict::Keep
                }
            }
        }
    }

    /// Reopens for silence since sound last played: 1 on the first.
    pub(super) fn stalls(&self) -> u32 {
        self.stalls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(last_beats: u64, quiet_checks: u32, suspect: bool, stalls: u32, warned: bool) -> Health {
        Health {
            last_beats,
            quiet_checks,
            suspect,
            stalls,
            warned,
        }
    }

    const fn check(beats: u64) -> Event {
        Event::Check { beats }
    }

    const LOST: Verdict = Verdict::Reopen(Why::Lost);
    const STALLED: Verdict = Verdict::Reopen(Why::Stalled);

    #[test]
    fn transitions() {
        use ErrorKind::*;
        use Verdict::{Keep, Resumed, Suspect};
        #[rustfmt::skip]
        let table: &[(&str, Health, Event, Verdict, Health)] = &[
            ("a lost device reopens", h(9, 2, false, 0, false), Event::Error(DeviceNotAvailable), LOST, h(9, 2, false, 0, false)),
            ("an invalidated stream reopens", h(9, 0, true, 1, true), Event::Error(StreamInvalidated), LOST, h(9, 0, true, 1, true)),
            ("an underrun keeps playing", h(9, 1, false, 0, false), Event::Error(Xrun), Keep, h(9, 1, false, 0, false)),
            ("no realtime priority keeps playing", h(9, 0, false, 0, false), Event::Error(RealtimeDenied), Keep, h(9, 0, false, 0, false)),
            ("a rerouted stream keeps playing", h(9, 0, false, 0, false), Event::Error(DeviceChanged), Keep, h(9, 0, false, 0, false)),
            ("another error is suspect", h(9, 0, false, 0, false), Event::Error(BackendError), Suspect, h(9, 0, true, 0, true)),
            ("a second one is not logged again", h(9, 0, true, 0, true), Event::Error(DeviceBusy), Keep, h(9, 0, true, 0, true)),
            ("a suspect stream reopens on one quiet check", h(9, 0, true, 0, true), check(9), STALLED, h(9, 0, true, 1, true)),
            ("a suspect stream that calls back is cleared", h(9, 0, true, 0, true), check(12), Keep, h(12, 0, false, 0, true)),
            ("three quiet checks keep", h(9, 2, false, 0, false), check(9), Keep, h(9, 3, false, 0, false)),
            ("the fourth reopens as stalled", h(9, 3, false, 0, false), check(9), STALLED, h(9, 0, false, 1, false)),
            ("after one stall it waits 8", h(0, 6, false, 1, false), check(0), Keep, h(0, 7, false, 1, false)),
            ("and reopens on the 8th", h(0, 7, false, 1, false), check(0), STALLED, h(0, 0, false, 2, false)),
            ("after two it waits 16", h(0, 14, false, 2, false), check(0), Keep, h(0, 15, false, 2, false)),
            ("and reopens on the 16th", h(0, 15, false, 2, false), check(0), STALLED, h(0, 0, false, 3, false)),
            ("after three it waits 32", h(0, 30, false, 3, false), check(0), Keep, h(0, 31, false, 3, false)),
            ("and reopens on the 32nd", h(0, 31, false, 3, false), check(0), STALLED, h(0, 0, false, 4, false)),
            ("the wait stops growing at 32", h(0, 31, false, 9, false), check(0), STALLED, h(0, 0, false, 10, false)),
            ("a suspect stream after a reopen waits 8", h(0, 0, true, 1, true), check(0), Keep, h(0, 1, true, 1, true)),
            ("and reopens on the 8th", h(0, 7, true, 1, true), check(0), STALLED, h(0, 0, true, 2, true)),
            ("a suspect stream keeps the cap of 32", h(0, 31, true, 3, true), check(0), STALLED, h(0, 0, true, 4, true)),
            ("and does not reopen sooner", h(0, 30, true, 3, true), check(0), Keep, h(0, 31, true, 3, true)),
            ("callbacks after stalls report sound back", h(0, 5, false, 3, true), check(4), Resumed, h(4, 0, false, 0, false)),
            ("callbacks with no stall say nothing", h(4, 1, false, 0, false), check(8), Keep, h(8, 0, false, 0, false)),
            ("a new stream starts its count over", h(500, 3, true, 2, true), Event::Opened, Keep, h(0, 0, false, 2, true)),
        ];
        for (name, start, event, verdict, end) in table {
            let mut health = *start;
            assert_eq!(health.next(*event), *verdict, "{name}: verdict");
            assert_eq!(health, *end, "{name}: state");
        }
    }

    #[test]
    fn an_error_then_silence_reopens_on_the_next_check() {
        let mut health = Health::default();
        assert_eq!(health.next(check(10)), Verdict::Keep);
        // Callbacks that ran before the error are judged first ...
        assert_eq!(health.next(check(20)), Verdict::Keep);
        assert_eq!(
            health.next(Event::Error(ErrorKind::BackendError)),
            Verdict::Suspect
        );
        // ... and the following interval decides.
        assert_eq!(health.next(check(20)), STALLED);
        assert_eq!(health.next(Event::Opened), Verdict::Keep);
        assert_eq!(health.next(check(3)), Verdict::Resumed);
        assert_eq!(health, h(3, 0, false, 0, false));
    }
}
