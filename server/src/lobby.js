// The lobby: who is here across a whole game (a collection, for a game
// like Wicket's), invites to a match, and friends. It sits on the rooms
// store and routes (rooms.js) and adds three state machines, each named
// here with its transitions in one place:
//
// Presence, one per player, kept in memory only and rebuilt from pulses:
//   away    -> here | playing | table   a pulse arrives
//   here    <-> playing <-> table        each pulse says where the player is
//   any     -> away                      no pulse for `awayMs`, or a pulse
//                                        saying the player is leaving
// The client works out where the player is; the server only keeps the last
// word and when it came. Who may read it is a filter (`show`), not a state.
//
// Invite, one per pair of players per kind of room, kept in the store:
//   sent     -> heard      the invited player's client took the notice
//   sent     -> accepted | declined   answered before the notice was taken
//   heard    -> accepted   they said yes: a room exists with both seated
//   heard    -> declined   they said no
//   sent | heard -> withdrawn   the sender took it back
//   sent | heard -> lapsed      `inviteMs` passed with no answer
// accepted, declined, withdrawn and lapsed are final. The sender hears a
// decline, a lapse or an acceptance once (`toldFrom`).
//
// Friends are one-sided, like following: adding someone needs nothing from
// them. A player whose `show` is "friends" is seen only by the players they
// have added themselves.

export const PRESENCE = ["away", "here", "playing", "table"];
export const VISIBILITY = ["everyone", "friends", "nobody"];
export const INVITE_STATES = ["sent", "heard", "accepted", "declined", "withdrawn", "lapsed"];
export const FINAL_INVITE_STATES = ["accepted", "declined", "withdrawn", "lapsed"];
export const INVITE_ANSWERS = ["accept", "decline", "withdraw"];

/** The invite changes a store may make. Anything else is a bug, not a state. */
const INVITE_TRANSITIONS = {
  sent: ["heard", "accepted", "declined", "withdrawn", "lapsed"],
  heard: ["accepted", "declined", "withdrawn", "lapsed"],
  accepted: [],
  declined: [],
  withdrawn: [],
  lapsed: [],
};

/** `invite` moved to `state`, or throws. */
export function inviteTransition(invite, state, now) {
  if (!INVITE_TRANSITIONS[invite.state]?.includes(state)) {
    throw new Error(`an invite cannot go from ${invite.state} to ${state}`);
  }
  invite.state = state;
  invite.changedAt = now;
  return invite;
}

/** The presence a pulse reports: one of PRESENCE, "here" when unknown. */
export function presenceOf(text) {
  return PRESENCE.includes(text) ? text : "here";
}

/** Who may see a player: one of VISIBILITY, "everyone" when unknown. */
export function visibilityOf(text) {
  return VISIBILITY.includes(text) ? text : "everyone";
}

/** The lobby's limits; a deployment overrides any of them. */
export const LOBBY_LIMITS = {
  // A player is away once no pulse has come for this long.
  awayMs: 2 * 60_000,
  // An invite nobody answers lapses after three days.
  inviteMs: 3 * 86_400_000,
  // A finished invite is forgotten after a week.
  keepInviteMs: 7 * 86_400_000,
  pulsesPerMinute: 20,
  invitesPerHour: 30,
  // Invites a player may have waiting on others at once.
  outstandingInvites: 20,
  friendsPerPlayer: 200,
  // The most players a pulse lists.
  hereListed: 200,
  // The most open tables a pulse lists.
  openListed: 20,
  // Seconds between pulses, as the server asks for them.
  pulseEvery: 60,
};

/** The presence of every player who has pulsed lately, in memory. */
export function memoryPresence() {
  const players = new Map(); // player id -> entry
  return {
    set(entry) {
      players.set(entry.id, entry);
    },
    get(id) {
      return players.get(id) ?? null;
    },
    drop(id) {
      players.delete(id);
    },
    /** Every entry pulsed within `awayMs` of `now`; older ones are dropped. */
    present(now, awayMs) {
      const out = [];
      for (const [id, entry] of players) {
        if (now - entry.at > awayMs) players.delete(id);
        else out.push(entry);
      }
      return out;
    },
    size() {
      return players.size;
    },
  };
}

/** Whether `viewer` may see `entry`: their setting, and no block either way. */
export function visibleTo(entry, viewer, relations) {
  if (!entry || entry.id === viewer.id) return false;
  if (entry.show === "nobody") return false;
  if (entry.show === "friends" && !entry.friends.has(viewer.handle)) return false;
  if (relations.blocked.has(entry.handle) || relations.blockedBy.has(entry.handle)) return false;
  return true;
}

/** An invite as either player sees it: handles and names, never account ids. */
export function publicInvite(invite, viewer) {
  const side = (p) => ({ handle: p.handle, name: p.name, guest: Boolean(p.guest) });
  return {
    id: invite.id,
    kind: invite.kind,
    from: side(invite.from),
    to: side(invite.to),
    state: invite.state,
    at: invite.at,
    changed_at: invite.changedAt ?? invite.at,
    mine: invite.from.id === viewer.id,
    room: invite.room ?? null,
  };
}

/** A player in a pulse's lists. */
export function publicPresence(entry, friend, muted = false) {
  return {
    handle: entry.handle,
    name: entry.name,
    guest: Boolean(entry.guest),
    state: entry.state,
    game: entry.game ?? null,
    place: entry.place ?? null,
    friend: Boolean(friend),
    muted: Boolean(muted),
  };
}
