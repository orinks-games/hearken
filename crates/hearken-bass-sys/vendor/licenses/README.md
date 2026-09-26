# BASS is not in this repository

BASS and bass_fx are proprietary libraries by un4seen Developments. They
are fetched by `tools/fetch_bass.py`, checked against pinned sha256 hashes,
and ignored by git.

BASS is free for non-commercial use. A game sold or otherwise used
commercially needs its own licence from https://www.un4seen.com/. The
licence covers finished products, not components, so Hearken can never
bundle it: each game fetches it for itself.

Point `HEARKEN_BASS_PATH` at a file or folder to use a copy installed
somewhere else.
