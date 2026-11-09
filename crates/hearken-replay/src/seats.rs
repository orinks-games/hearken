//! Several players in one run: a [`Seat`] for each, and [`Seated`], which
//! puts the seat into an input's code so the run file needs no new field.
//!
//! A match played online is one run whose inputs come from more than one
//! player. The server orders them and stamps each with the seat of the
//! player who sent it, in the top four bits of the 16-bit input code, so a
//! player cannot move for another. Rules that want multiplayer use
//! `Seated<Move>` as their input type and read the seat off each input;
//! `Replayable`, `Recording` and the run format do not change.
//!
//! The seat takes 4 bits, so a game's own codes stay below 4,096 and a run
//! holds up to 16 seats. Code 4,095 of each seat is reserved: it says the
//! seat left the match ([`Seated::Left`]), which the server adds when a
//! player leaves or runs out of time, and the rules decide what that means.

use crate::rules::InputCode;

/// How far the seat is shifted into an input code.
pub const SEAT_SHIFT: u32 = 12;
/// The bits of a code that are the game's own.
pub const CODE_MASK: u16 = 0x0fff;
/// The game code that means "this seat left the match".
pub const LEFT_CODE: u16 = CODE_MASK;
/// The largest code a game's own inputs may use under [`Seated`].
pub const MAX_SEATED_CODE: u16 = CODE_MASK - 1;

/// One player's place in a match, 0 to 15, in order of joining.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Seat(u8);

impl Seat {
    /// The most seats a run can hold.
    pub const COUNT: usize = 16;
    /// The first seat, the one that made the room.
    pub const FIRST: Seat = Seat(0);

    /// Seat `n`, for `n` below 16.
    pub const fn new(n: u8) -> Option<Seat> {
        if (n as usize) < Seat::COUNT {
            Some(Seat(n))
        } else {
            None
        }
    }

    /// The seat's number, 0 upward.
    pub const fn number(self) -> u8 {
        self.0
    }

    /// The seat as an index into a list with one entry per seat.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The seat after this one in a round of `seats`, wrapping to the first.
    pub const fn next_of(self, seats: u8) -> Seat {
        if seats == 0 {
            return Seat::FIRST;
        }
        Seat((self.0 + 1) % seats)
    }
}

impl std::fmt::Display for Seat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "seat {}", self.0 + 1)
    }
}

/// An input with the seat it came from, or a seat leaving the match.
///
/// `Seated<I>` is an [`InputCode`] whenever `I` is one, as long as `I`'s
/// codes stay at or below [`MAX_SEATED_CODE`]. The seat goes in the top
/// four bits of the code; the game's code keeps the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Seated<I> {
    /// `seat` made the input `input`.
    Input { seat: Seat, input: I },
    /// `seat` left the match: the player left, forfeited, or ran out of
    /// time. The rules decide what that means, as they decide everything.
    Left { seat: Seat },
}

impl<I> Seated<I> {
    /// `input` from `seat`.
    pub fn new(seat: Seat, input: I) -> Self {
        Seated::Input { seat, input }
    }

    /// The seat the input came from.
    pub fn seat(&self) -> Seat {
        match self {
            Seated::Input { seat, .. } | Seated::Left { seat } => *seat,
        }
    }

    /// The game's own input, unless the seat left.
    pub fn input(&self) -> Option<&I> {
        match self {
            Seated::Input { input, .. } => Some(input),
            Seated::Left { .. } => None,
        }
    }

    /// Whether this is a seat leaving.
    pub fn is_left(&self) -> bool {
        matches!(self, Seated::Left { .. })
    }
}

/// The code the server stamps: `seat`'s bits over the game's `code`.
pub const fn stamp(seat: Seat, code: u16) -> u16 {
    ((seat.0 as u16) << SEAT_SHIFT) | (code & CODE_MASK)
}

/// The seat and the game's own code in a stamped code.
pub const fn unstamp(code: u16) -> (Seat, u16) {
    (Seat((code >> SEAT_SHIFT) as u8), code & CODE_MASK)
}

impl<I: InputCode> InputCode for Seated<I> {
    fn to_code(&self) -> (u16, i32) {
        match self {
            Seated::Input { seat, input } => {
                let (code, value) = input.to_code();
                debug_assert!(
                    code <= MAX_SEATED_CODE,
                    "a seated input's code ({code}) must stay at or below {MAX_SEATED_CODE}"
                );
                (stamp(*seat, code), value)
            }
            Seated::Left { seat } => (stamp(*seat, LEFT_CODE), 0),
        }
    }

    fn from_code(code: u16, value: i32) -> Option<Self> {
        let (seat, own) = unstamp(code);
        if own == LEFT_CODE {
            return (value == 0).then_some(Seated::Left { seat });
        }
        let input = I::from_code(own, value)?;
        // A game code that would spill into the seat bits has no one
        // encoding, so it is not an input.
        if input.to_code().0 > MAX_SEATED_CODE {
            return None;
        }
        Some(Seated::Input { seat, input })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Take(u8);

    impl InputCode for Take {
        fn to_code(&self) -> (u16, i32) {
            (1, i32::from(self.0))
        }
        fn from_code(code: u16, value: i32) -> Option<Self> {
            (code == 1 && (1..=3).contains(&value)).then_some(Take(value as u8))
        }
    }

    #[test]
    fn seats_stop_at_sixteen() {
        assert_eq!(Seat::new(15).map(Seat::number), Some(15));
        assert_eq!(Seat::new(16), None);
        assert_eq!(Seat::new(1).unwrap().next_of(2), Seat::FIRST);
        assert_eq!(Seat::FIRST.next_of(3), Seat::new(1).unwrap());
        assert_eq!(Seat::new(2).unwrap().to_string(), "seat 3");
    }

    #[test]
    fn a_seated_input_round_trips_through_its_code() {
        let seat = Seat::new(5).unwrap();
        let input = Seated::new(seat, Take(2));
        let (code, value) = input.to_code();
        assert_eq!(code, 5 << 12 | 1);
        assert_eq!(value, 2);
        assert_eq!(Seated::<Take>::from_code(code, value), Some(input));
        assert_eq!(unstamp(code), (seat, 1));
    }

    #[test]
    fn leaving_is_its_own_code_and_nothing_else_reads_as_it() {
        let left = Seated::<Take>::Left {
            seat: Seat::new(1).unwrap(),
        };
        assert_eq!(left.to_code(), (1 << 12 | LEFT_CODE, 0));
        assert_eq!(
            Seated::<Take>::from_code(1 << 12 | LEFT_CODE, 0),
            Some(left)
        );
        assert_eq!(Seated::<Take>::from_code(1 << 12 | LEFT_CODE, 1), None);
        assert!(left.is_left());
        assert_eq!(left.input(), None);
        assert_eq!(left.seat().number(), 1);
    }

    #[test]
    fn a_code_the_game_does_not_know_is_refused() {
        assert_eq!(Seated::<Take>::from_code(2, 1), None);
        assert_eq!(Seated::<Take>::from_code(1, 9), None);
    }
}
