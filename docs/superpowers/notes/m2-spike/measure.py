"""Spike measurement: compare a candidate sound against a reference recording.

Throwaway. Reports third-octave band balance (each band relative to the clip's
own total, so loudness differences drop out), spectral centroid, crest factor,
and for one-shots the decay time; plus modal peaks and harmonic analysis
helpers used to set recipe targets.
"""
import sys

import numpy as np
import soundfile as sf
from scipy.signal import find_peaks, resample_poly, welch

SR = 48000
BANDS = [63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]


def load(path, start=None, dur=None):
    x, sr = sf.read(path, always_2d=True)
    x = x.mean(axis=1)
    if sr != SR:
        x = resample_poly(x, SR, sr)
    if start is not None:
        x = x[int(start * SR):]
    if dur is not None:
        x = x[: int(dur * SR)]
    return x


def band_balance(x):
    f, p = welch(x, SR, nperseg=8192)
    total = p.sum()
    out = []
    for c in BANDS:
        lo, hi = c / 2 ** 0.5, c * 2 ** 0.5
        e = p[(f >= lo) & (f < hi)].sum()
        out.append(10 * np.log10(e / total + 1e-12))
    return np.array(out)


def centroid(x):
    f, p = welch(x, SR, nperseg=8192)
    return float((f * p).sum() / p.sum())


def crest_db(x):
    return float(20 * np.log10(np.abs(x).max() / (np.sqrt((x ** 2).mean()) + 1e-12)))


def rms_env(x, seconds):
    w = max(1, int(seconds * SR))
    c = np.concatenate([[0.0], np.cumsum(x ** 2)])
    e = (c[w:] - c[:-w]) / w
    return np.sqrt(np.maximum(np.pad(e, (w // 2, w - 1 - w // 2), mode="edge"), 0))


def decay_ms(x, drop_db=30.0):
    """Time from the peak until the 5 ms RMS envelope falls drop_db below it."""
    env = rms_env(x, 0.005)
    i = int(env.argmax())
    thr = env[i] * 10 ** (-drop_db / 20)
    after = np.nonzero(env[i:] < thr)[0]
    return 1000.0 * (after[0] if len(after) else len(env) - i) / SR


def peaks(x, lo=200, hi=16000, n=8):
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x)), 1 << 16))
    f = np.fft.rfftfreq(1 << 16, 1 / SR)
    m = (f > lo) & (f < hi)
    db = 20 * np.log10(spec[m] / spec[m].max() + 1e-12)
    idx, _ = find_peaks(db, distance=40, prominence=6)
    top = idx[np.argsort(db[idx])[::-1][:n]]
    return sorted((round(float(f[m][i])), round(float(db[i]), 1)) for i in top)


def report(label, x):
    b = band_balance(x)
    return label, b, centroid(x), crest_db(x), decay_ms(x)


def compare(ref, cands):
    rows = [report("reference", ref)] + [report(k, v) for k, v in cands.items()]
    print("band Hz      " + " ".join(f"{c:>6}" for c in BANDS) + "  centroid  crest  decay")
    rb = rows[0][1]
    for label, b, cen, cr, dec in rows:
        print(f"{label:<12} " + " ".join(f"{v:6.1f}" for v in b)
              + f"  {cen:7.0f}  {cr:5.1f}  {dec:5.0f}ms")
        if label != "reference":
            d = b - rb
            print(f"{'  vs ref':<12} " + " ".join(f"{v:+6.1f}" for v in d)
                  + f"   mean |dev| {np.abs(d[1:]).mean():.1f} dB")


if __name__ == "__main__":
    x = load(sys.argv[1])
    print("peaks", peaks(x))
    print(report(sys.argv[1], x))
