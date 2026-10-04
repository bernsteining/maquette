#!/usr/bin/env python3
import math
import wave
import numpy as np
import scipy.signal as sg

SR, BPM, DUR = 44100, 120, 27.0
BEAT = 60 / BPM
STEP = BEAT / 4
N = int(SR * DUR)
rng = np.random.default_rng(7)


S2, COMPILE, SCRUB0, SCRUB_MID, SCRUB1, SSS, CHIPS2 = 3.0, 4.0, 4.5, 5.25, 5.95, 6.0, 6.75
TYPING = (3.3, 3.9)
BLITZ, CUT, NCUTS = 8.0, 1.5, 6
SCAD = BLITZ + NCUTS * CUT
GLTF, EVERY, OUTRO = SCAD + 2.5, SCAD + 5.0, SCAD + 7.0
FINAL = OUTRO + 2.0
WIPES = [S2, GLTF, EVERY, OUTRO]

hz = lambda m: 440 * 2 ** ((m - 69) / 12)
Am, F, C, G, Dm, E = (57, 60, 64), (53, 57, 60), (55, 60, 64), (55, 59, 62), (50, 53, 57), (52, 56, 59)
ROOT = {Am: 33, F: 29, C: 36, G: 31, Dm: 38, E: 28}
CUT_CHORDS = [Am, F, C, G, Dm, E]
BELLS = [69, 72, 74, 76, 79, 81, 84, 88]


def clamp(x, a=0.0, b=1.0):
    return min(b, max(a, x))


def prog(t, a, b):
    return clamp((t - a) / (b - a))


def ease_in_out(p):
    return 4 * p**3 if p < 0.5 else 1 - (-2 * p + 2) ** 3 / 2


def ease_out_cubic(p):
    return 1 - (1 - p) ** 3


def azimuth(t):
    if t < SCRUB0:
        return 180.0
    if t < SCRUB_MID:
        return 180 + 48 * ease_in_out(prog(t, SCRUB0, SCRUB_MID))
    if t < SCRUB1:
        return 228 - 78 * ease_in_out(prog(t, SCRUB_MID, SCRUB1))
    return 150 + 35 * ease_out_cubic(prog(t, SCRUB1, BLITZ))


def hole(t):
    a = SCAD
    if t < a + 0.6:
        return 25.0
    if t < a + 1.15:
        return 25 - 12 * ease_in_out(prog(t, a + 0.6, a + 1.15))
    if t < a + 1.8:
        return 13 + 18 * ease_in_out(prog(t, a + 1.15, a + 1.8))
    return 31 - 6 * ease_in_out(prog(t, a + 1.8, a + 2.2))


def tt(d):
    return np.arange(int(SR * d)) / SR


def filt(x, kind, fc, order=2):
    sos = sg.butter(order, np.asarray(fc) / (SR / 2), kind, output="sos")
    return sg.sosfilt(sos, x, axis=0)


def noise(d):
    return rng.standard_normal(int(SR * d))


def svf_lowpass(x, cutoff, res):

    f = (2 * np.sin(np.pi * np.clip(cutoff, 20, SR / 6) / SR)).tolist()
    q = 1 - res
    low = band = 0.0
    out = [0.0] * len(x)
    for i, xi in enumerate(x.tolist()):
        low += f[i] * band
        band = math.tanh(band + f[i] * (xi - low - q * band))
        out[i] = low
    return np.array(out)


class Bus:
    def __init__(self):
        self.b = np.zeros((N, 2))

    def add(self, sig, t0, gain=1.0, pan=0.0):
        i = int(round(t0 * SR))
        if i >= N or i < 0:
            return
        sig = sig[: N - i]
        l, r = np.cos((pan + 1) * np.pi / 4), np.sin((pan + 1) * np.pi / 4)
        self.b[i : i + len(sig), 0] += sig * gain * l
        self.b[i : i + len(sig), 1] += sig * gain * r


def saw(f, d, detune=0.0):
    t = tt(d)
    return sg.sawtooth(2 * np.pi * f * (1 + detune) * t + rng.uniform(0, 6.28))


def env_ad(t, a, dcy):
    return np.minimum(1, t / a) * np.exp(-t * dcy)


def kick():
    t = tt(0.4)
    f = 48 + 120 * np.exp(-t * 32)
    body = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 7)
    click = filt(noise(0.4), "bandpass", [1500, 5000]) * np.exp(-t * 400) * 0.35
    return np.tanh((body + click) * 1.4)


def clap():
    t = tt(0.35)
    n = filt(noise(0.35), "bandpass", [900, 4500])
    env = sum(np.where(t >= o, np.exp(-(t - o) * 220), 0) for o in (0, 0.012, 0.024)) + np.where(t >= 0.03, np.exp(-(t - 0.03) * 14) * 0.5, 0)
    return n * env


def hat(open_=False):
    d = 0.2 if open_ else 0.05
    t = tt(d)
    return filt(noise(d), "highpass", 7500, 4) * np.exp(-t * (14 if open_ else 90))


def bass(m, d=BEAT * 0.45):
    t = tt(d)
    f = hz(m)
    x = saw(f, d, -0.004) + saw(f, d, 0.004) + 0.8 * np.sin(2 * np.pi * f * t)
    x = filt(x, "lowpass", 420) + filt(x, "lowpass", 1400) * np.exp(-t * 20) * 0.5
    return np.tanh(x * 0.9) * np.minimum(1, t / 0.004) * np.minimum(1, (d - t) / 0.02)


def sub(m, d):
    t = tt(d)
    return np.sin(2 * np.pi * hz(m) * t) * np.minimum(1, t / 0.02) * np.minimum(1, (d - t) / 0.25)


def supersaw(chord, d, cutoff=2600, attack=0.25, release=0.5):
    t = tt(d)
    voices = [saw(hz(m + o), d, dt) for m in chord for o in (0, 12) for dt in (-0.012, -0.006, 0.0, 0.006, 0.012)]
    x = filt(sum(voices) / len(voices) * 2.2, "lowpass", cutoff)
    return x * np.minimum(1, t / attack) * np.minimum(1, (d - t) / release)


def pluck(m, d=0.32, bright=3200):
    t = tt(d)
    x = saw(hz(m), d, -0.005) + saw(hz(m), d, 0.005)
    x = filt(x, "lowpass", bright) * 0.7 + filt(x, "lowpass", 900) * 0.3
    return x * env_ad(t, 0.002, 11) * 0.55


def bell(m, d=1.2):
    t = tt(d)
    f = hz(m)
    x = np.sin(2 * np.pi * f * t + 1.6 * np.sin(2 * np.pi * f * 3.5 * t) * np.exp(-t * 6))
    return x * env_ad(t, 0.002, 3.5) * 0.5


def whoosh(center, width=0.6):
    d = width * 1.6
    t = tt(d)
    p = t / d
    peak = width / d
    env = np.where(p < peak, (p / peak) ** 2.5, np.exp(-(p - peak) * 10))
    x = filt(noise(d), "bandpass", [400, 2000]) * (1 - p) + filt(noise(d), "bandpass", [1500, 7000]) * p
    return x * env, center - width


def riser(d):
    t = tt(d)
    p = t / d
    x = filt(noise(d), "bandpass", [600, 2400]) * (1 - p) + filt(noise(d), "highpass", 4000) * p
    return x * p**2


def crash(d=2.0):
    t = tt(d)
    return filt(filt(noise(d), "highpass", 3000, 2), "lowpass", 9000) * np.exp(-t * 2.6)


def reverse_crash(d=1.0):
    return crash(d)[::-1] * np.linspace(0, 1, int(SR * d)) ** 2


def boom(d=2.0):
    t = tt(d)
    f = 34 + 40 * np.exp(-t * 7)
    return np.sin(2 * np.pi * np.cumsum(f) / SR) * env_ad(t, 0.003, 2.2)


def tick():
    t = tt(0.01)
    return filt(noise(0.01), "highpass", 3000) * np.exp(-t * 500)


drums, low, pads, arp, fx, send = Bus(), Bus(), Bus(), Bus(), Bus(), Bus()
K, CL, HC, HO, TK = kick(), clap(), hat(), hat(True), tick()
kicks = []


def beat(t0, t1, hats=True, claps=True):

    for b, tb in enumerate(np.arange(t0, t1 - 1e-6, BEAT)):
        drums.add(K, tb, 0.85)
        kicks.append(tb)
        if claps and b % 2 == 1:
            drums.add(CL, tb, 0.32)
            send.add(CL, tb, 0.25)
        if hats:
            drums.add(HO, tb + BEAT / 2, 0.12, pan=0.25)
            for k in (1, 3):
                drums.add(HC, tb + k * STEP, 0.05, pan=-0.25)


def offbeat_bass(t0, t1, chord_at):
    for tb in np.arange(t0, t1 - 1e-6, BEAT):
        r = ROOT[chord_at(tb)]
        low.add(bass(r + 12), tb + BEAT / 2, 0.42)
        low.add(sub(r, BEAT * 0.45), tb + BEAT / 2, 0.3)


def pad(chord, t0, d, cutoff=2600, gain=0.5, attack=0.25):
    p = supersaw(chord, d + 0.3, cutoff, attack)
    pads.add(p, t0, gain)
    send.add(p, t0, gain * 0.5)


def arpeggio(chord, t0, t1, gain=0.32, cutoff=None):

    pattern = [0, 1, 2, 1, 0, 2, 1, 2]
    notes = [chord[i] + 12 for i in range(3)] + [chord[0] + 24]
    for s, ts in enumerate(np.arange(t0, t1 - 1e-6, STEP)):
        m = notes[pattern[s % 8]] if s % 8 != 7 else notes[3]
        bright = cutoff(ts) if cutoff else 3200
        arp.add(pluck(m, 0.3, bright), ts, gain, pan=0.35 if s % 2 else -0.35)


PROG = [Am, F, C, G]
prog_at = lambda t0: (lambda t: PROG[int((t - t0) // (4 * BEAT)) % 4])


fx.add(boom(), 0.05, 0.45)
drums.add(K, 0.05, 0.8)
fx.add(crash(2.0), 0.05, 0.18)
pad(Am, 0.05, S2 + 1.0, 1400, 0.45, attack=1.2)
for m in (57, 64, 69):
    fx.add(bell(m, 1.8), 0.05, 0.22)
for i, m in enumerate(BELLS):
    fx.add(bell(m, 0.9), 0.38 + i * 0.045, 0.16, pan=-0.5 + i / 7)
fx.add(bell(81, 1.2), 1.0, 0.22)
fx.add(bell(76, 1.2), 1.5, 0.2)
for t0 in np.arange(1.5, S2 - 0.1, 0.25):
    drums.add(HC, t0, 0.03 + 0.04 * (t0 - 1.5), pan=0.25)


for w in WIPES:
    sig, start = whoosh(w, 0.45)
    fx.add(sig, start, 0.22, pan=-0.3)
    fx.add(crash(1.6), w, 0.3)
    if w == GLTF:
        drums.add(K, w, 0.85)
        drums.add(CL, w, 0.3)


for t0 in np.arange(TYPING[0], TYPING[1], 0.03):
    fx.add(TK, t0, rng.uniform(0.05, 0.1), pan=rng.uniform(-0.5, -0.1))
drums.add(K, S2, 0.7)
kicks.append(S2)
arpeggio(Am, S2, COMPILE - 0.1, 0.22, cutoff=lambda t: 500 + 2500 * prog(t, S2, COMPILE) ** 2)
fx.add(riser(1.0), COMPILE - 1.0, 0.25)
fx.add(reverse_crash(0.8), COMPILE - 0.8, 0.22)
for k, ts in enumerate(np.arange(COMPILE - BEAT, COMPILE - 1e-6, STEP / 2)):
    drums.add(CL, ts, 0.06 + 0.03 * k)
fx.add(boom(), COMPILE, 0.6)
fx.add(crash(), COMPILE, 0.3)
drums.add(CL, COMPILE, 0.4)
drop = prog_at(COMPILE)
beat(COMPILE, BLITZ)
offbeat_bass(COMPILE, BLITZ, drop)
for bar in range(2):
    t0 = COMPILE + bar * 4 * BEAT
    pad(drop(t0), t0, 4 * BEAT)
scrub_cut = lambda t: 900 + (azimuth(t) - 150) * 45 if t < BLITZ else 3200
arpeggio(Am, COMPILE, COMPILE + 4 * BEAT, 0.3, cutoff=scrub_cut)
arpeggio(F, COMPILE + 4 * BEAT, BLITZ, 0.3, cutoff=scrub_cut)


fx.add(bell(76, 1.5), SSS, 0.32)
fx.add(crash(1.4), SSS, 0.14)
fx.add(bell(81, 1.5), SSS + 0.02, 0.14)
for i, m in enumerate((81, 84, 88)):
    fx.add(bell(m, 0.8), CHIPS2 + i * 0.14, 0.16, pan=-0.3 + 0.3 * i)


for i, ch in enumerate(CUT_CHORDS):
    t0 = BLITZ + i * CUT
    beat(t0, t0 + CUT)
    offbeat_bass(t0, t0 + CUT, lambda t, ch=ch: ch)
    pad(ch, t0, CUT, 3000, 0.5, attack=0.05)
    arpeggio(ch, t0, t0 + CUT, 0.32)
    fx.add(crash(1.2), t0, 0.16)
    drums.add(CL, t0, 0.3)
    for m in ch:
        fx.add(pluck(m + 12, 0.5, 4200), t0, 0.22)
    fx.add(bell(ch[2] + 24, 0.8), t0, 0.16)


beat(SCAD, GLTF - 0.25, hats=False)
fx.add(crash(1.6), SCAD, 0.26)
drums.add(CL, SCAD, 0.35)
for m in Am:
    fx.add(pluck(m + 12, 0.6, 4200), SCAD, 0.24)
offbeat_bass(SCAD, GLTF - 0.25, lambda t: Am)
d = GLTF - SCAD + 0.3
src = supersaw(Am, d, 9000, 0.1)
cut = np.array([300 + (hole(SCAD + k / SR) - 10) * 160 for k in range(len(src))])
pp = svf_lowpass(src, cut, 0.6)
pads.add(pp, SCAD, 0.55)
send.add(pp, SCAD, 0.3)
arpeggio(Am, SCAD, GLTF - 0.25, 0.18, cutoff=lambda t: 400 + (hole(t) - 10) * 140)


pad(F, GLTF, 4 * BEAT, 2200, 0.55, attack=0.6)
pad(G, GLTF + 4 * BEAT, EVERY - GLTF - 4 * BEAT, 2200, 0.5, attack=0.3)
for st, m, ln in [(0, 72, 1.0), (1.0, 71, 0.5), (1.5, 69, 0.5), (2.0, 67, 1.0), (3.0, 69, 1.0), (4.0, 71, 0.9)]:
    fx.add(pluck(m, ln * BEAT * 2, 2200), GLTF + st * BEAT, 0.3)
    send.add(pluck(m, ln * BEAT * 2, 2200), GLTF + st * BEAT, 0.3)
for i in range(6):
    fx.add(bell(76 + [0, 3, 5, 7, 5, 3][i], 0.6), GLTF + 0.62 + i * 0.07, 0.12, pan=-0.4 + 0.16 * i)
fx.add(riser(1.6), EVERY - 1.6, 0.28)
fx.add(reverse_crash(1.0), EVERY - 1.0, 0.2)


fx.add(boom(), EVERY, 0.5)
fx.add(crash(), EVERY, 0.28)
again = prog_at(EVERY)
beat(EVERY, OUTRO)
offbeat_bass(EVERY, OUTRO, again)
pad(again(EVERY), EVERY, OUTRO - EVERY)
arpeggio(Am, EVERY, OUTRO, 0.3)
for i in range(5):
    fx.add(bell([81, 84, 86, 88, 93][i], 0.7), EVERY + 0.5 + i * 0.125, 0.28, pan=-0.4 + 0.2 * i)
    fx.add(pluck([69, 72, 74, 76, 81][i], 0.25, 5000), EVERY + 0.5 + i * 0.125, 0.3, pan=-0.4 + 0.2 * i)
fx.add(bell(88, 1.4), EVERY + 1.25, 0.18)


fx.add(boom(), OUTRO, 0.45)
beat(OUTRO, FINAL)
offbeat_bass(OUTRO, FINAL, lambda t: F if t < OUTRO + 1.0 else G)
pad(F, OUTRO, 1.0, 2200, 0.45, attack=0.2)
pad(G, OUTRO + 1.0, 1.0, 2600, 0.45, attack=0.2)
for i, m in enumerate(reversed(BELLS)):
    fx.add(bell(m, 0.8), OUTRO + 0.02 + i * 0.03, 0.14, pan=0.5 - i / 7)
fx.add(bell(81, 1.2), OUTRO + 0.5, 0.3)
fx.add(bell(76, 1.0), OUTRO + 1.0, 0.2)
fx.add(bell(79, 1.0), OUTRO + 1.25, 0.18)
drums.add(CL, OUTRO + 0.5, 0.45)
for m in G:
    fx.add(pluck(m + 12, 0.5, 4200), OUTRO + 0.5, 0.24)
fx.add(crash(1.4), OUTRO + 0.5, 0.14)
arpeggio(F, OUTRO, OUTRO + 1.0, 0.24, cutoff=lambda t: 900 + 2300 * prog(t, OUTRO, FINAL) ** 2)
arpeggio(G, OUTRO + 1.0, FINAL, 0.26, cutoff=lambda t: 900 + 2300 * prog(t, OUTRO, FINAL) ** 2)
fx.add(riser(1.4), FINAL - 1.4, 0.24)
for k, ts in enumerate(np.arange(FINAL - BEAT, FINAL - 1e-6, STEP / 2)):
    drums.add(CL, ts, 0.06 + 0.03 * k)
drums.add(K, FINAL, 1.0)
kicks.append(FINAL)
fx.add(boom(2.5), FINAL, 0.6)
fx.add(crash(2.5), FINAL, 0.32)
drums.add(CL, FINAL, 0.4)
pad(Am, FINAL, DUR - FINAL - 0.1, 3200, 0.6, attack=0.02)
low.add(sub(33, DUR - FINAL - 0.1), FINAL, 0.5)
for m in (69, 76, 81):
    fx.add(bell(m, 2.5), FINAL, 0.16)


t = np.arange(N) / SR
pump = np.ones(N)
for k in kicks:
    i = int(k * SR)
    seg = t[i:] - k
    pump[i:] = np.minimum(pump[i:], 1 - 0.6 * np.exp(-seg * 9))
dl = int(0.375 * SR)
echo = np.zeros_like(arp.b)
echo[dl:, 0] += arp.b[:-dl, 1] * 0.32
echo[2 * dl :, 1] += arp.b[: -2 * dl, 0] * 0.18
send.b += arp.b * 0.3
ir_t = tt(3.0)
ir = np.stack([filt(filt(noise(3.0), "highpass", 200), "lowpass", 6000) * np.exp(-ir_t * 1.8) for _ in range(2)], 1)
wet = np.stack([sg.fftconvolve(send.b[:, ch], ir[:, ch])[:N] for ch in range(2)], 1) * 0.012

mix = drums.b + (low.b + pads.b + arp.b + echo) * pump[:, None] + wet + fx.b
gap = np.ones(N)
for g in (COMPILE, EVERY):
    a0, a1 = int((g - 0.1) * SR), int(g * SR)
    gap[a0:a1] = 0.0
    gap[a0 - 300 : a0] = np.linspace(1, 0, 300)
mix *= gap[:, None]
mix *= 10 ** (-14.5 / 20) / np.sqrt((mix**2).mean())
lim = 0.8
mag = np.abs(mix)
mix = np.where(mag > lim, np.sign(mix) * (lim + 0.19 * np.tanh((mag - lim) / 0.19)), mix)
mix *= np.minimum(1, (DUR - t) / 0.5)[:, None]

with wave.open("music.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((np.clip(mix, -1, 1) * 32767).astype("<i2").tobytes())
peak, rms = 20 * np.log10(np.abs(mix).max()), 20 * np.log10(np.sqrt((mix**2).mean()))
print(f"music.wav  {DUR:.1f}s  peak {peak:.1f} dBFS  rms {rms:.1f} dBFS  crest {peak - rms:.1f} dB")
