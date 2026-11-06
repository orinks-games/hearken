# Bundled HRTF set

`mit_kemar_normal_pinna.sofa` is the MIT KEMAR dummy-head measurement
(normal pinna), in the SOFA format, copied unchanged from libmysofa's
`share/MIT_KEMAR_normal_pinna.sofa` (the file libmysofa's `default.sofa`
points to): <https://github.com/hoene/libmysofa/tree/main/share>.

- Source data: Bill Gardner and Keith Martin, "HRTF Measurements of a KEMAR
  Dummy-Head Microphone", MIT Media Lab Perceptual Computing Technical
  Report #280, May 1994. <https://sound.media.mit.edu/resources/KEMAR.html>
- SOFA conversion: the SOFA conventions project (sofaconventions.org,
  Piotr Majdak), as credited in libmysofa's README.
- SHA-256:
  `2768ac841213a7ae11d1ea7fd0f25a69b39216102dc5dd913ea6ba0f0dc57e28`.

## Licence

The measurement's own documentation (`hrtfdoc.txt`, checked on
2026-11-06 at <https://sound.media.mit.edu/resources/KEMAR/hrtfdoc.txt>)
says:

> This HRTF data is Copyright 1994 by the MIT Media Lab. It is provided
> without any usage restrictions. We request that you cite the authors when
> using this data for research or commercial applications.

So the data may be shipped in games, free or paid. A game that ships it
should credit Bill Gardner and Keith Martin, MIT Media Lab, for example in
its credits or `help.md`.

The SOFA file's own `License` attribute reads "No license provided, ask the
author for permission". That field was filled in by the conversion; the
measurement itself carries the terms above, which put no restriction on
use. A game that wants no doubt at all can pass its own SOFA file instead
(`Audio::set_hrtf`).
