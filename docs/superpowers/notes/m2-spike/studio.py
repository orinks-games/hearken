"""Spike (throwaway): Surge XT jingle, hybrid crowd, and the loudness-matched
listening set.

win_medium is re-voiced note for note from Lucky Break's recipe (the same
ladder and final chord from C5), but played by Surge XT -- band-limited
oscillators, FM bells whose brightness decays under a filter envelope -- and
mastered with pedalboard's reverb, compressor and limiter.
"""
import re
import shutil
from pathlib import Path

import mido
import numpy as np
import soundfile as sf
from pedalboard import Compressor, HighpassFilter, Limiter, Pedalboard, Reverb, load_plugin
from scipy.signal import resample_poly

from measure import SR, load, rms_env  # noqa: F401

HERE = Path(__file__).parent
OUT = HERE / "out"
LB = Path(r"C:\Users\joshu\gh-projects\lucky-break\assets\sounds\sfx")
SURGE = str(HERE / "dl/surge/Surge XT.vst3/Contents/x86_64-win/Surge XT.vst3")


def setp(p, name, target):
    """Set a Surge parameter to the valid value nearest `target`."""
    q = p.parameters[name]
    if isinstance(target, str):
        setattr(p, name, target)
        return
    vals = q.valid_values

    def num(v):  # times come back as "435.0 ms" or "1.86 s"; targets are seconds
        m = re.match(r"(-?[\d.]+|-inf)\s*(ms|s)?\b", str(v))
        if not m:
            return np.nan
        x = -1e9 if m.group(1) == "-inf" else float(m.group(1))
        return x / 1000.0 if m.group(2) == "ms" else x

    best = vals[int(np.nanargmin([abs(num(v) - target) for v in vals]))]
    setattr(p, name, best)
    got = num(getattr(p, name))
    assert abs(got - target) <= max(0.05 * abs(target), 0.02), (name, target, got)


def fm_patch(p, ratio_oct, ratio_semis, depth_db, decay, release, cutoff, feg, feg_decay):
    setp(p, "a_osc_1_type", "Sine")
    setp(p, "a_osc_2_type", "Sine")
    setp(p, "a_osc_2_octave", ratio_oct)
    setp(p, "a_osc_2_pitch", ratio_semis)
    p.a_osc_2_mute = True
    p.a_osc_3_mute = True
    setp(p, "a_fm_routing", "2 > 1")
    setp(p, "a_fm_depth", depth_db)
    setp(p, "a_filter_1_type", "LP 12 dB")
    setp(p, "a_filter_1_cutoff", cutoff)
    setp(p, "a_filter_1_feg_mod_amount", feg)
    setp(p, "a_filter_eg_attack", 0.0)
    setp(p, "a_filter_eg_decay", feg_decay)
    setp(p, "a_filter_eg_sustain", 0.0)
    setp(p, "a_amp_eg_attack", 0.0)
    setp(p, "a_amp_eg_decay", decay)
    setp(p, "a_amp_eg_sustain", 0.0)
    setp(p, "a_amp_eg_release", release)
    setp(p, "a_volume", -6.0)


def render(p, notes, seconds):
    msgs = []
    for at, midi, dur, vel in notes:
        msgs.append(mido.Message("note_on", note=midi, velocity=vel, time=at))
        msgs.append(mido.Message("note_off", note=midi, velocity=0, time=at + dur))
    y = p(msgs, duration=seconds, sample_rate=SR, num_channels=2, reset=True)
    return y.mean(axis=0)


def win_medium():
    seconds = 2.5
    ladder = (0, 4, 7, 12, 16, 19, 24)
    mallet = load_plugin(SURGE)
    # ratio 4 modulator: a marimba-like bar, bright strike, quick darkening
    fm_patch(mallet, 2, 0.0, -9.0, decay=0.30, release=0.2, cutoff=1800.0, feg=36.0,
             feg_decay=0.12)
    a = render(mallet, [(0.13 * i, 72 + s, 0.22, 100) for i, s in enumerate(ladder)], seconds)
    bell = load_plugin(SURGE)
    # ratio 3.5 modulator: inharmonic bell partials that mellow as they ring
    fm_patch(bell, 2, -2.31, -3.0, decay=1.4, release=0.9, cutoff=2500.0, feg=40.0,
             feg_decay=0.35)
    b = render(bell, [(0.13 * i, 84 + s, 0.3, 70) for i, s in enumerate(ladder)]
               + [(0.95, 72 + s, 1.3, 105) for s in (12, 16, 19, 24)], seconds)
    mix = a * 0.8 + b * 0.55
    master = Pedalboard([
        HighpassFilter(160.0),
        Reverb(room_size=0.35, damping=0.5, wet_level=0.16, dry_level=0.9, width=1.0),
        Compressor(threshold_db=-20.0, ratio=2.5, attack_ms=4.0, release_ms=120.0),
        Limiter(threshold_db=-1.5, release_ms=80.0),
    ])
    return master(mix.astype(np.float32)[None, :], SR)[0]


def crowd_hybrid():
    """A CC0 walla recording, re-cut into a fresh 3 s swell (never the same
    twice), with the synthesized babble and laugh layered over it."""
    rng = np.random.default_rng(7)
    walla = load(str(HERE / "ref/817149.wav"))[10 * SR: 70 * SR]
    n = 3 * SR
    bed = np.zeros(n)
    for layer in range(2):
        at = -int(rng.uniform(0, 0.3) * SR)
        while at < n:
            g = int(rng.uniform(0.35, 0.8) * SR)
            src = int(rng.uniform(0, len(walla) - g))
            win = np.sin(np.linspace(0, np.pi, g)) ** 0.5  # equal-power-ish
            lo, hi = max(at, 0), min(at + g, n)
            bed[lo:hi] += walla[src + lo - at: src + hi - at] * win[lo - at: hi - at]
            at += g // 2
    t = np.arange(n) / SR
    swell = 0.35 + 0.65 * np.minimum(t / 0.7, 1.0) * np.clip((3.0 - t) / 0.8, 0.0, 1.0)
    synth = load(str(OUT / "crowd_murmur_new_synth.wav"))[:n]
    bed = bed / (np.sqrt((bed ** 2).mean()) + 1e-9)
    synth = synth / (np.sqrt((synth ** 2).mean()) + 1e-9)
    return (bed * 0.8 + synth * 0.45) * swell


def rms(x):
    return float(np.sqrt((x ** 2).mean()))


def write_group(listen, clips):
    """Write clips at one shared RMS: the loudest level every clip in the
    group reaches without peaking over -0.3 dBFS, so nothing wins by volume."""
    clips = {k: np.asarray(v, dtype=np.float64) for k, v in clips.items()}
    level = min(0.97 * rms(x) / np.abs(x).max() for x in clips.values())
    for name, x in clips.items():
        sf.write(listen / f"{name}.wav", x * (level / rms(x)), SR, subtype="PCM_16")


def save_peak(path, x):
    sf.write(path, np.asarray(x) * (0.89 / np.abs(x).max()), SR, subtype="PCM_16")


def main():
    save_peak(OUT / "win_medium_new_surge.wav", win_medium())
    save_peak(OUT / "crowd_murmur_new_hybrid.wav", crowd_hybrid())

    listen = HERE / "listen"
    shutil.rmtree(listen, ignore_errors=True)
    listen.mkdir()
    write_group(listen, {
        "1a chip - current": load(str(LB / "chip_25.wav")),
        "1b chip - new (Faust modal)": load(str(OUT / "chip_25_new.wav")),
        "1c chip - real recording (CC0)": load(str(HERE / "ref/201808.wav")),
    })
    write_group(listen, {
        "2a crowd - current": load(str(LB / "crowd_murmur.wav")),
        "2b crowd - new, pure synthesis (fundsp)": load(str(OUT / "crowd_murmur_new_synth.wav")),
        "2c crowd - new, recording plus synthesis": load(str(OUT / "crowd_murmur_new_hybrid.wav")),
        "2d crowd - real recording (CC0)": load(str(HERE / "ref/817149.wav"))[12 * SR: 15 * SR],
    })
    write_group(listen, {
        "3a win jingle - current": load(str(LB / "win_medium.wav")),
        "3b win jingle - new (Surge XT)": load(str(OUT / "win_medium_new_surge.wav")),
    })
    write_group(listen, {
        "4a jet ski - new (fundsp, throttle-driven)": load(str(OUT / "jetski_ride_new.wav")),
        "4b jet ski - real recording (CC0)": load(str(HERE / "ref/36169.wav"))[4 * SR: 12 * SR],
    })
    print("\n".join(sorted(p.name for p in listen.iterdir())))


if __name__ == "__main__":
    main()
