"""Fetch the BASS runtime the Rust audio backend loads.

BASS is un4seen's and proprietary. A game built on it is free to ship it --
un4seen charge nothing for non-commercial use -- but this repository is
public source, and hosting someone else's binaries there is a different
thing from bundling them in a release. So the libraries are fetched, not
committed.

Where they land is what `crates/hearken-bass-sys` looks for:
`crates/hearken-bass-sys/vendor/<os>-<arch>/`, which its build script stages
next to the test and game binaries. `HEARKEN_BASS_PATH` overrides the lot if
a developer keeps BASS somewhere else, and `--dest` sends this script's own
output somewhere else -- a game vendoring BASS into its own tree rather than
this crate's.

Two libraries, no add-ons: `bass` itself decodes WAV, and `bass_fx` supplies
effects such as a biquad low pass. Codec add-ons (AAC, FLAC, Opus, HLS) are
not fetched here, since nothing in this crate needs them yet; a game that
does can fetch its own alongside these with `--dest`.

Windows, macOS and Linux are all pinned. The macOS downloads are universal
binaries -- one file carrying the Intel and Apple silicon slices -- so the
same bytes are written into `macos-x86_64` and `macos-aarch64`, and a build
targeting either architecture finds its library. Linux takes the x86_64 or
the aarch64 slice of un4seen's `-linux` packages, whichever the machine is.

Every file is checked against a pinned sha256. A silent change upstream --
un4seen ship rolling updates behind stable URLs -- would otherwise walk into
a build with nothing said, and the audio backend is exactly where a quiet
substitution is hardest to notice.

    python tools/fetch_bass.py                # fetch what is missing
    python tools/fetch_bass.py --check        # verify, change nothing
    python tools/fetch_bass.py --force        # re-fetch everything
    python tools/fetch_bass.py --print-hashes # what upstream serves now
    python tools/fetch_bass.py --dest DIR     # fetch into DIR instead
"""

from __future__ import annotations

import argparse
import hashlib
import io
import platform
import shutil
import sys
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
VENDOR = REPO_ROOT / "crates" / "hearken-bass-sys" / "vendor"

# One entry per file: the archive it lives in, the member inside that archive,
# and the sha256 of the exact bytes this build was verified against. Update a
# hash only after listening to the result -- a decoder or filter swap is
# audible before it is visible. `--print-hashes` prints what upstream is
# serving today without touching the pins or the vendor directory.
#
# Pins taken from un4seen on 2026-09-02, the day these bindings were first
# pinned; there is no older build they were verified against.
WINDOWS_X64 = {
    "bass.dll": (
        "https://www.un4seen.com/files/bass24.zip",
        "x64/bass.dll",
        "febb2cf1882d554c3a958280777da0b69f07de6e262df271de11c56e4a54afd4",
    ),
    "bass_fx.dll": (
        "https://www.un4seen.com/files/z/0/bass_fx24.zip",
        "x64/bass_fx.dll",
        "a6e1847eef52d882b4137af514d834c2e220daceb417c821d1e502fb7a34c84a",
    ),
}

# macOS, from un4seen's own osx packages. Every file is a fat binary carrying
# x86_64, i386 and arm64, so one download serves both macOS vendor
# directories.
MACOS_UNIVERSAL = {
    "libbass.dylib": (
        "https://www.un4seen.com/files/bass24-osx.zip",
        "libbass.dylib",
        "e81fb7b4d0009ba6343fbfcd840620704dcb840686d405b9734cd37150d31974",
    ),
    "libbass_fx.dylib": (
        "https://www.un4seen.com/files/z/0/bass_fx24-osx.zip",
        "libbass_fx.dylib",
        "82aa99a433d9e86de7ecbbd65599b2ffe9befebcd420d6368ef3bd1d33e418fe",
    ),
}

# Linux, from un4seen's `-linux` packages, which carry one slice per
# architecture under `libs/`. These depend on nothing beyond libc, so the same
# files run on every glibc distribution. The same archives carry `armhf` and
# `x86` slices, which nothing here builds for.
LINUX_X86_64 = {
    "libbass.so": (
        "https://www.un4seen.com/files/bass24-linux.zip",
        "libs/x86_64/libbass.so",
        "bd0841d0c14f25065a16192be07c20b045e384199a86a00892cc3864474dbc51",
    ),
    "libbass_fx.so": (
        "https://www.un4seen.com/files/z/0/bass_fx24-linux.zip",
        "libs/x86_64/libbass_fx.so",
        "63a41ba9627d4577874fd8f241789834ad058f4ef3bac930bf9bbce54b4cedba",
    ),
}

LINUX_AARCH64 = {
    "libbass.so": (
        "https://www.un4seen.com/files/bass24-linux.zip",
        "libs/aarch64/libbass.so",
        "a05ba3afc880b03f36db8dea50fd1d912cacf2a2c85d785fbaef020f5a93adb2",
    ),
    "libbass_fx.so": (
        "https://www.un4seen.com/files/z/0/bass_fx24-linux.zip",
        "libs/aarch64/libbass_fx.so",
        "7bc424672048e6cb8e44a3e0b5da984ad7996cf77fa4d3e47808bca3c7e793e8",
    ),
}

TARGETS = {
    "windows-x86_64": WINDOWS_X64,
    "macos-x86_64": MACOS_UNIVERSAL,
    "macos-aarch64": MACOS_UNIVERSAL,
    "linux-x86_64": LINUX_X86_64,
    "linux-aarch64": LINUX_AARCH64,
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def target_keys() -> list[str]:
    """Vendor directories to fill for the machine running this script.

    macOS gets two, because the download is universal and the same bytes are
    correct for an Intel target and an Apple silicon one; filling both means
    a cross build does not go looking for a library nobody fetched.
    """
    if sys.platform == "win32":
        return ["windows-x86_64"]
    if sys.platform == "darwin":
        return ["macos-x86_64", "macos-aarch64"]
    if sys.platform.startswith("linux"):
        machine = platform.machine()
        if machine in ("x86_64", "AMD64"):
            return ["linux-x86_64"]
        if machine in ("aarch64", "arm64"):
            return ["linux-aarch64"]
    # Anything else has no pinned build, and guessing a URL for it would be
    # worse than saying so.
    raise SystemExit(
        f"fetch_bass: no pinned BASS build for {sys.platform} {platform.machine()}. "
        "Install BASS yourself and point HEARKEN_BASS_PATH at it."
    )


def download(url: str, vendor: Path) -> bytes:
    """The archive at `url`, or a diagnosis of why it could not be had."""
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            return response.read()
    except (urllib.error.URLError, TimeoutError) as err:
        raise SystemExit(
            f"fetch_bass: could not reach {url} ({err}).\n"
            "Download it by hand and unpack the library into "
            f"{vendor}/<os>-<arch>/."
        ) from err


def member_of(archive: bytes, url: str, member: str) -> bytes:
    """One file out of a downloaded zip, matched case-insensitively.

    un4seen are not consistent about case across platforms -- the Windows
    bass_fx package puts its sources under `C/` and the Linux one under `c/`
    -- so the member paths here are matched case-insensitively.
    """
    with zipfile.ZipFile(io.BytesIO(archive)) as zf:
        names = {n.lower(): n for n in zf.namelist()}
        actual = names.get(member.lower())
        if actual is None:
            raise SystemExit(f"fetch_bass: {url} has no {member}")
        return zf.read(actual)


def from_network(
    url: str, member: str, want: str, archives: dict[str, bytes], vendor: Path
) -> bytes:
    if url not in archives:
        archives[url] = download(url, vendor)
    data = member_of(archives[url], url, member)
    got = digest(data)
    if got != want:
        raise SystemExit(
            f"fetch_bass: {member} from {url} is not the pinned build.\n"
            f"  expected sha256 {want}\n"
            f"  got      sha256 {got}\n"
            "un4seen ship rolling updates behind stable URLs. Listen to the "
            "result, then update the hash in this file if the new build is good."
        )
    return data


def print_hashes() -> int:
    """Print the sha256 of every pinned member as upstream serves it today.

    This is how the table above is filled in: run it, listen to the result,
    then paste the hashes in. It downloads every platform's archive, not just
    this machine's, and writes nothing.
    """
    archives: dict[str, bytes] = {}
    seen: set[tuple[str, str]] = set()
    for key, files in TARGETS.items():
        for name, (url, member, want) in files.items():
            if (url, member) in seen:
                continue
            seen.add((url, member))
            if url not in archives:
                archives[url] = download(url, VENDOR)
            got = digest(member_of(archives[url], url, member))
            mark = "  (pinned)" if got == want else "  (DIFFERS from the pin)"
            print(f"{got}  {key}/{name}  <- {url}!{member}{mark}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify what is on disk and change nothing",
    )
    parser.add_argument(
        "--force", action="store_true", help="re-fetch even when the file is correct"
    )
    parser.add_argument(
        "--print-hashes",
        action="store_true",
        help="print the sha256 upstream serves for every pinned file, and stop",
    )
    parser.add_argument(
        "--dest",
        metavar="DIR",
        help="fetch into DIR/<os>-<arch>/ instead of this crate's vendor directory",
    )
    args = parser.parse_args()

    if args.print_hashes:
        return print_hashes()

    vendor = Path(args.dest) if args.dest else VENDOR

    missing: list[str] = []
    # macOS fills two directories from one universal download, and both
    # libraries of a platform can come out of one archive, so bytes already
    # fetched in this run are reused rather than downloaded twice.
    fetched: dict[str, bytes] = {}
    archives: dict[str, bytes] = {}

    for key in target_keys():
        out_dir = vendor / key
        out_dir.mkdir(parents=True, exist_ok=True)
        for name, (url, member, want) in TARGETS[key].items():
            path = out_dir / name
            if path.is_file() and not args.force:
                if digest(path.read_bytes()) == want:
                    print(f"  ok       {key}/{name}")
                    continue
                print(f"  stale    {key}/{name}")
            if args.check:
                missing.append(f"{key}/{name}")
                print(f"  MISSING  {key}/{name}")
                continue
            data = fetched.get(want)
            source = "already fetched"
            if data is None:
                data = from_network(url, member, want, archives, vendor)
                source = url
            fetched[want] = data
            path.write_bytes(data)
            print(f"  fetched  {key}/{name}  <- {source}")

    if args.check and missing:
        print(
            f"\nfetch_bass: {len(missing)} file(s) missing or stale. "
            "Run: python tools/fetch_bass.py",
            file=sys.stderr,
        )
        return 1

    licence = VENDOR / "licenses" / "README.md"
    if not licence.is_file():
        print(
            "fetch_bass: the licence note is missing from vendor/licenses",
            file=sys.stderr,
        )
    if shutil.which("cargo") is None:
        print("fetch_bass: cargo is not on PATH; the libraries are staged anyway")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
