# Sounds

Put the game's sounds here. A sound's key is its path in this folder
without the extension, with `/` between folders: `sfx/door.ogg` is played
as `ctx.audio.play("sfx/door", Bus::EFFECTS)`. Ogg, Opus, WAV, FLAC and MP3
all work.

A release (`hearken release`) ships this folder beside the program, or packs
it into an encrypted `sounds.pak` when `pack = true` is set under
`[package.metadata.hearken]` in Cargo.toml.
