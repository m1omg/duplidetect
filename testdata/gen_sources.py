#!/usr/bin/env python3
"""Deterministically regenerates the PCM sources for the test fixtures.

Only the uncompressed sources are generated. Every lossy/encoded fixture is
produced once and committed, because re-encoding with a different encoder build
would feed different audio to each platform and make parity tests meaningless.
"""
import math, os, random, struct, sys, wave

RATE = 44100

def write(path, fn, seconds, channels=2):
    with wave.open(path, "wb") as w:
        w.setnchannels(channels); w.setsampwidth(2); w.setframerate(RATE)
        data = bytearray()
        for i in range(int(RATE * seconds)):
            v = max(-1.0, min(1.0, fn(i / RATE)))
            s = struct.pack("<h", int(v * 30000))
            data += s * channels
        w.writeframes(bytes(data))

def arpeggio(notes, tempo, seed, noise=0.015):
    rng = random.Random(seed)
    def fn(t):
        f = notes[int(t * tempo) % len(notes)]
        phase = (t * tempo) % 1.0
        env = math.exp(-3.0 * phase)
        v = sum(a * math.sin(2 * math.pi * f * h * t) for h, a in ((1, 1.0), (2, 0.45), (3, 0.22)))
        return v * env / 1.8 + noise * rng.uniform(-1, 1)
    return fn

def main(out):
    os.makedirs(out, exist_ok=True)
    write(f"{out}/musicA.wav", arpeggio([261.63, 329.63, 392.0, 523.25, 392.0, 329.63], 4, 1), 4.0)
    write(f"{out}/musicB.wav", arpeggio([220.0, 277.18, 349.23, 440.0, 349.23, 277.18], 4, 2), 4.0)
    # A held two-tone chord: stationary, so it exercises the shape-template path.
    write(f"{out}/toneA.wav",
          lambda t: 0.4 * math.sin(2 * math.pi * 220 * t) + 0.2 * math.sin(2 * math.pi * 330 * t), 4.0)
    write(f"{out}/toneB.wav",
          lambda t: 0.4 * math.sin(2 * math.pi * 523 * t) + 0.2 * math.sin(2 * math.pi * 784 * t), 4.0)
    # Same audio as musicA with two seconds of silence in front: tests that
    # padding does not count as content. The generator is built once, so the
    # dither sequence matches musicA rather than restarting on every sample.
    base = arpeggio([261.63, 329.63, 392.0, 523.25, 392.0, 329.63], 4, 1)
    write(f"{out}/musicA_shifted.wav", lambda t: 0.0 if t < 2.0 else base(t - 2.0), 6.0)
    print(f"generated PCM sources in {out}")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "testdata/fixtures")
