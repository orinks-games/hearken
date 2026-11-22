//! A band for audio games: a new song for every run.
//!
//! A game writes a [`Style`] (which instruments, scales, tempos, bass
//! patterns and grooves fit it) and records a [`Bank`] of single notes and
//! drum hits for the instruments it names. [`Song::compose`] then writes a
//! song from a seed: its own key, scale, tempo, chords, tune, bass line and
//! groove, with a section form it goes round for as long as play lasts.
//! [`Band`] plays it on the music bus a note at a time, each note placed on
//! its sample of the audio clock, and its layers come and go with play as
//! a dynamic score's stems do ([`Feel`], [`layer_level`]).
//!
//! The same seed writes the same song, so a game picks what a song is
//! tied to: a new seed every run, the day's date for a daily challenge
//! that everyone plays alike, a garden's own number.
//!
//! Everything the band plays is an ordinary clip start on [`Bus::MUSIC`],
//! so a run's take records it and its export sounds as the run did.
//!
//! [`Bus::MUSIC`]: hearken_audio::Bus::MUSIC

mod band;
mod compose;
pub mod rng;
mod style;
pub mod theory;

pub use band::{layer_level, Band, BandState, Feel, LOOKAHEAD_S, RAMP_S, TARGET};
pub use compose::{Chosen, Event, Note, Section, SectionKind, Song, BEATS, FORM};
pub use style::{Bank, Instrument, Kit, Part, Play, Role, Style, BASS_PATTERNS, GROOVES};
pub use theory::{Chord, Scale};
