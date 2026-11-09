//! A match's rules kept in step with its room: [`Match`] applies the
//! server-ordered turns to a [`Recording`] and sends the player's own.
//!
//! The sync state is explicit ([`Sync`]): `Idle` when the recording has
//! every turn the client has read; `Posting` while the player's turn is on
//! its way; `Behind(n)` while `n` turns have been read but not applied,
//! which only happens when a game applies them one at a time to play
//! sounds for each ([`Match::apply_next`]).

use hearken_replay::{InputCode, Recording, Replayable, Seat, Seated};

use crate::client::{Answer, Rooms, Ticket};
use crate::error::RoomsError;
use crate::protocol::{EventKind, RoomId, RoomInfo};

/// Where the recording stands against the room's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sync {
    /// Every turn read has been applied.
    Idle,
    /// The player's own turn is on its way to the server.
    Posting,
    /// This many turns have been read and wait to be applied.
    Behind(usize),
}

/// One turn applied to the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied<I> {
    pub seq: u64,
    pub seat: Seat,
    /// The inputs, or empty when the seat left.
    pub inputs: Vec<I>,
    pub left: bool,
}

/// Why a match could not be made from a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchError {
    /// The room has not started: no seed yet.
    NotStarted,
    /// The room's context is not one these rules take.
    BadContext,
    /// The recording could not start (a bad rules name or context).
    Format(hearken_replay::FormatError),
}

/// The rules of a match, stepped by the room's turns in server order.
pub struct Match<R: Replayable> {
    recording: Recording<R>,
    applied_seq: u64,
    posting: Option<Ticket>,
    /// The last refusal of the player's turn, until taken.
    refused: Option<RoomsError>,
}

impl<R, I> Match<R>
where
    R: Replayable<Input = Seated<I>>,
    I: InputCode + Clone,
{
    /// A match for `room`, which must have started: the rules begin from
    /// the room's seed and context, with no turn applied yet.
    pub fn new(room: &RoomInfo) -> Result<Self, MatchError> {
        let seed = room.seed().ok_or(MatchError::NotStarted)?;
        let context: R::Context =
            serde_json::from_value(room.context.clone()).map_err(|_| MatchError::BadContext)?;
        let recording = Recording::<R>::new(seed, &context).map_err(MatchError::Format)?;
        Ok(Match {
            recording,
            applied_seq: 0,
            posting: None,
            refused: None,
        })
    }

    /// The rules as they stand, to read from.
    pub fn sim(&self) -> &R {
        self.recording.sim()
    }

    /// The recording, to submit or save.
    pub fn recording(&self) -> &Recording<R> {
        &self.recording
    }

    pub fn finished(&self) -> bool {
        self.recording.finished()
    }

    /// The sequence number of the last turn applied.
    pub fn applied_seq(&self) -> u64 {
        self.applied_seq
    }

    /// Where the recording stands against what the client has read of
    /// `room`.
    pub fn sync(&self, room: &crate::client::Room) -> Sync {
        if self.posting.is_some() {
            return Sync::Posting;
        }
        let waiting = room
            .events_after(self.applied_seq)
            .filter(|e| e.is_move())
            .count();
        if waiting > 0 {
            Sync::Behind(waiting)
        } else {
            Sync::Idle
        }
    }

    /// Sends the player's `inputs` as one turn. The turn is not applied
    /// here: it comes back ordered by the server and is applied with the
    /// rest. Refused while a turn is already on its way.
    pub fn post(
        &mut self,
        rooms: &mut Rooms,
        id: &RoomId,
        inputs: &[I],
    ) -> Result<Ticket, RoomsError> {
        if self.posting.is_some() {
            return Err(RoomsError::Busy);
        }
        let codes: Vec<(u16, i32)> = inputs.iter().map(InputCode::to_code).collect();
        let ticket = rooms.post_turn(id, &codes)?;
        self.posting = Some(ticket);
        Ok(ticket)
    }

    /// Collects the answer to a posted turn. Call once a frame, before
    /// applying turns.
    pub fn update(&mut self, rooms: &mut Rooms) {
        if let Some(ticket) = self.posting {
            match rooms.result(ticket) {
                None if rooms.is_pending(ticket) => {}
                None => self.posting = None,
                Some(Ok(Answer::Seq(_))) | Some(Ok(_)) => self.posting = None,
                Some(Err(e)) => {
                    self.posting = None;
                    self.refused = Some(e);
                }
            }
        }
    }

    /// The server's refusal of the player's last turn, once, if there was
    /// one.
    pub fn take_refusal(&mut self) -> Option<RoomsError> {
        self.refused.take()
    }

    /// Applies every turn read but not yet applied, in order.
    pub fn apply_all(&mut self, room: &crate::client::Room) -> Vec<Applied<I>> {
        let mut out = Vec::new();
        while let Some(applied) = self.apply_next(room) {
            out.push(applied);
        }
        out
    }

    /// Applies the next turn read but not yet applied, for a game that
    /// plays each other seat's turn at its own pace.
    pub fn apply_next(&mut self, room: &crate::client::Room) -> Option<Applied<I>> {
        let event = room
            .events_after(self.applied_seq)
            .find(|e| e.is_move())?
            .clone();
        self.applied_seq = event.seq;
        let seat = event.seat()?;
        let (seated, inputs, left) = match event.kind {
            EventKind::Left => (vec![Seated::Left { seat }], Vec::new(), true),
            _ => {
                let inputs: Vec<I> = event
                    .inputs
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|&(code, value)| I::from_code(code, value))
                    .collect();
                let seated = inputs
                    .iter()
                    .cloned()
                    .map(|input| Seated::new(seat, input))
                    .collect();
                (seated, inputs, false)
            }
        };
        self.recording.step(&seated);
        Some(Applied {
            seq: event.seq,
            seat,
            inputs,
            left,
        })
    }
}
