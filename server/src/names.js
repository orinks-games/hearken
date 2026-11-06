// Guest names: a word from each list and a number, such as "Copper Fox 12".
// Players never type their own, so nobody can post a name the boards should
// not show. Every word is plain, easy to hear from a synthesized voice, and
// harmless next to any word in the other list.

export const GUEST_ADJECTIVES = Object.freeze([
  "Amber", "Autumn", "Blue", "Brave", "Bright", "Calm", "Cedar", "Clever",
  "Copper", "Coral", "Cosy", "Crimson", "Dawn", "Dusty", "Early", "Emerald",
  "Evening", "Gentle", "Golden", "Green", "Happy", "Hazel", "Honey", "Jolly",
  "Kind", "Lucky", "Maple", "Merry", "Misty", "Mossy", "Night", "Olive",
  "Patient", "Plum", "Quick", "Quiet", "Rainy", "Ruby", "Rusty", "Sandy",
  "Silver", "Snowy", "Sunny", "Swift", "Thistle", "Tidy", "Velvet", "Winter",
]);

export const GUEST_NOUNS = Object.freeze([
  "Badger", "Beacon", "Bell", "Bramble", "Brook", "Candle", "Comet", "Cricket",
  "Falcon", "Fern", "Finch", "Fox", "Harbor", "Hare", "Heron", "Kettle",
  "Kite", "Lantern", "Lark", "Meadow", "Moth", "Otter", "Owl", "Pebble",
  "Pine", "Puffin", "Rabbit", "Raven", "Ribbon", "River", "Robin", "Rook",
  "Sparrow", "Spruce", "Squirrel", "Star", "Teapot", "Thrush", "Tortoise", "Trail",
  "Violin", "Walrus", "Whistle", "Willow", "Wren", "Yak", "Acorn", "Anchor",
]);

/** The pattern every guest name follows. */
export const GUEST_NAME_PATTERN = /^[A-Z][a-z]+ [A-Z][a-z]+ [1-9][0-9]?$/;

function pick(list, bytes, at) {
  return list[((bytes[at] << 8) | bytes[at + 1]) % list.length];
}

/** A new guest name, chosen at random. */
export function guestName() {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  const number = 1 + (((bytes[4] << 8) | bytes[5]) % 99);
  return `${pick(GUEST_ADJECTIVES, bytes, 0)} ${pick(GUEST_NOUNS, bytes, 2)} ${number}`;
}

/** Whether `name` is one `guestName` could have made. */
export function isGuestName(name) {
  if (typeof name !== "string" || !GUEST_NAME_PATTERN.test(name)) return false;
  const [adjective, noun] = name.split(" ");
  return GUEST_ADJECTIVES.includes(adjective) && GUEST_NOUNS.includes(noun);
}
