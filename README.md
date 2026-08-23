# DupliDetect

An app that finds duplicate audio files — including copies saved in
*different formats*. The same recording stored as a WAV, an MP3, and an
Ogg Vorbis file is recognised as one piece of audio, not three unrelated files.

Runs on **macOS, Windows and Linux**. The macOS build is a native SwiftUI app
(universal, Apple Silicon and Intel). Windows and Linux are served by a Rust
port that shares the algorithm exactly — see [Cross-platform](#cross-platform).

## What it finds

DupliDetect runs two passes over your folders.

**Identical files** — files that are byte-for-byte the same. Files are grouped
by size first, so only files that could possibly match are ever hashed (SHA-256).

**Same audio, different bytes** — the interesting case. Every file is decoded to
mono audio and reduced to an *acoustic fingerprint*, then fingerprints are
compared to each other. This catches:

- the same song as MP3, WAV, M4A, FLAC, AIFF, CAF and Ogg Vorbis
- the same song at different bitrates (320 kbps vs 64 kbps)
- a held tone, drone or test signal, which the main fingerprint cannot see
- copies with different tags, artwork or filenames
- copies at different volumes
- copies with extra silence at the start, or trimmed differently
- a short clip taken from a longer recording

## Supported formats

| Decoded and audio-matched | Exact copies only |
|---|---|
| WAV, AIFF/AIFC, MP3, M4A/AAC, Apple Lossless, FLAC, CAF, AU, AC-3, AMR, **Ogg Vorbis** | Opus, WMA, WavPack, Monkey's Audio, Matroska Audio, RealAudio |

macOS ships no decoder for the formats in the right-hand column, so their audio
cannot be compared — but byte-identical copies of them are still found, and the
app tells you exactly why a file was only partially checked.

Ogg Vorbis works because DupliDetect bundles its own decoder
([stb_vorbis](https://github.com/nothings/stb), public domain); AVFoundation
cannot open Ogg files on macOS.

## Cross-platform

`Sources/` is the macOS app. `rust/` is a port of the same algorithm that builds
for all three platforms and ships the Windows and Linux versions:

```
rust/dd-core/     the algorithm: fingerprinting, matching, quality ranking
rust/dd-app/      decoding, scanning, the egui interface, platform shims
  src/bin/ddcli.rs   headless scanner and --selftest
testdata/         committed fixtures and the reference vectors
```

Windows gets a portable `DupliDetect.exe` — no installer, no admin rights, no
VC++ redistributable. Linux gets a tarball containing a single binary that needs
only glibc 2.35+, X11 or Wayland, and OpenGL: nothing to install. Both are built
and tested on real Windows and Linux by GitHub Actions.

### Proving the port matches

The port is not trusted because it looks right; it is measured against the macOS
implementation. `Tools/build_parity.sh` compiles a *copy* of `Sources/` with a
few members widened, so reference values come from the real algorithm code while
`Sources/` stays untouched. Measured results:

| Check | Result |
|---|---|
| Hann window peak, band edges, `shape`, `shapeDistance`, `median` | exact |
| Band energies | worst relative error 2e-7 |
| Fingerprints from identical PCM | 1 bit differs in 32,448 (0.003%) |
| Spectral shape template | 0 LSB difference on every file |
| Decoder + resampler drift, end to end | worst bit-error rate 0.0012 |
| Grouping and keeper decisions | identical at every sensitivity |

The matching threshold is 22% of bits, so the residual differences are three
orders of magnitude away from changing any decision.

Two deliberate divergences are documented in the code. `make_window` uses
`2·sin²(x/2)` rather than reproducing vDSP's f32 `1 − cos(x)`, which loses most
of its digits to cancellation; matching vDSP bit-for-bit would amplify
platform `cosf` differences and make Windows and Linux disagree with each other.
And `decode.rs` discards AAC's priming frame, which Symphonia does not report —
without it, `.m4a` files land half a fingerprint frame out of alignment and match
measurably worse (bit-error rate 0.108 against 0.060).

Anyone can check a build on their own machine:

```sh
ddcli selftest      # verifies the analysis matches the reference
ddcli scan ~/Music  # scan without the interface
```

### Format differences off macOS

The Rust build decodes WAV, AIFF, MP3, M4A/AAC, Apple Lossless, FLAC,
uncompressed CAF and Ogg Vorbis. Relative to the macOS app it loses AC-3, AMR,
Sound Designer II and AAC-inside-CAF, which have no pure-Rust decoder; those
join Opus, WMA, WavPack, Monkey's Audio and Matroska in the exact-copies-only
column, and the app says so when it skips one.

## Building

### macOS

Requires only the Xcode Command Line Tools — no Xcode installation needed.

```sh
./build.sh          # universal (arm64 + x86_64) release build
./build.sh --fast   # host architecture only, for quick iteration
```

The result is `build/DupliDetect.app`. It is ad-hoc signed so it launches
without a developer certificate.

Because the app is not notarised, the first launch needs
**right-click → Open** (or *System Settings → Privacy & Security → Open Anyway*).

### Windows and Linux

```sh
cd rust && cargo build --release      # builds duplidetect and ddcli
cd rust && cargo test --release       # parity, decoder and interface tests
python3 Tools/compare_scans.py testdata   # macOS-only: Swift vs Rust decisions
```

CI builds both on every push and attaches the packaged results as artifacts.

## Using it

1. Add folders: drag them onto the sidebar, click **Choose Folder…**, use the
   **+** button, or **File ▸ Add Folder…** (⌘O). Remove one with the **×** on
   its row, or clear them all with **Remove All**.
2. Choose what to look for and how strict matching should be.
3. Click **Scan for Duplicates**.

You can also drop a folder onto the app icon, or launch it with folders to scan
straight away:

```sh
open -a DupliDetect --args ~/Music ~/Downloads
```

Each result group shows one row per copy with its format, bitrate, duration and
size. Click ▶ to hear the first 30 seconds of any copy — including Ogg Vorbis.

**Nothing is ever deleted permanently.** Marked files are moved to the Trash, so
a mistake is always recoverable.

### Choosing what to keep

The **Which copy to keep** rule marks everything *except* one file in each group:

| Rule | Keeps |
|---|---|
| Best quality | Truly lossless first, then sample rate, bit depth and bitrate |
| Largest / Smallest file | The biggest or smallest copy |
| Oldest / Newest file | By modification date |
| Shallowest folder | The copy nearest the top of your folders |

You can override any of it by clicking the icon beside a file. A group will
never let you mark every copy — one always stays.

**Losslessness comes from the codec, never the file extension.** `.caf` and
`.m4a` are containers that can hold either kind of audio: a `.caf` may be raw
PCM or a 98 kbps AAC, and an `.m4a` may be AAC or Apple Lossless. DupliDetect
reads the codec it actually decoded, so those files are ranked — and labelled —
by what is really inside them. The extension is consulted only for files macOS
cannot decode at all, and where even that says nothing (a container name) the
value is treated as unknown rather than guessed.

**Best quality deliberately ignores file size.** A FLAC and a WAV made from the
same master hold identical audio, so the WAV being three times larger does not
make it better — DupliDetect keeps the FLAC. Size only breaks a tie once
losslessness, sample rate, bit depth and channel count are all equal, and then
the *smaller* file wins. Attributes that a format does not report (compressed
lossless formats publish no bit depth) are treated as unknown rather than zero.

The rule also refuses to trade away audio: a copy holding meaningfully less of
the recording is never chosen as the keeper. That comparison uses the *content*
duration — silent padding at either end does not count — so a file with two
seconds of leading silence still competes on equal terms with an unpadded copy
rather than winning simply for being longer.

### Match strictness

Only affects the audio-matching pass. **Perfect match is the default.**

| Level | Groups |
|---|---|
| **Perfect match** | The same recording end to end — true 1:1 duplicates |
| Very strict | Near-identical recordings, including a clip taken from a longer one |
| Strict | Near-identical recordings, with a little more tolerance |
| Relaxed | Also heavily re-encoded copies — review before deleting |
| Very relaxed | Loosely similar audio — expect false positives |

**Perfect match is not simply a tighter threshold**, because a tighter threshold
cannot express what "1:1" means. Measured across the test corpus, genuine
duplicates of one master span a bit-error rate of 0.000 (any lossless
conversion) up to 0.086 (Ogg Vorbis q5), while a four-second excerpt of a
ten-second recording of the same tune scores 0.101 — the two populations very
nearly touch, so no threshold separates them reliably.

So Perfect match adds a *structural* requirement instead: the matching region
must cover essentially the whole of **both** files, judged against the longer
one. An excerpt aligns perfectly over the whole of its own length, which is
exactly why measuring against the shorter file cannot exclude it. Silent padding
still does not count — a copy with two seconds of leading silence is trimmed
before comparison and remains a 1:1 duplicate.

The audio tolerance at Perfect match is the same as Very strict; the structural
rule does the work. Every format conversion above still groups: FLAC, AIFF, CAF,
ALAC, 192 kbps MP3, 96 kbps MP3, 128 kbps AAC and Ogg Vorbis all match their
source.

## How the fingerprinting works

Each file is decoded to mono at 11 025 Hz and cut into overlapping 4096-sample
frames. For every frame, the spectrum is folded into 33 logarithmically spaced
bands between 300 Hz and 3 kHz, and 32 bits are derived — one per adjacent band
pair — recording whether that band's energy rose or fell *relative to the
previous frame*.

Because every bit comes from a difference of differences, the fingerprint is
inherently invariant to volume and largely unaffected by lossy-codec artefacts.
This is the Haitsma-Kalker robust hash.

Matching indexes each 32-bit value by both of its 16-bit halves, so a pair of
files becomes a candidate if either half survives re-encoding intact. Candidates
vote on a time offset — which is what makes alignment robust to added silence —
and the winning offset is scored by bit-error rate over the overlap. Unrelated
audio sits near 0.5 (pure chance); the same master re-encoded typically lands
below 0.15.

### Stationary audio

That scheme has one blind spot, and DupliDetect handles it separately. Because
every bit records a *change*, audio that does not change carries no signal: for
a held tone or a drone the difference is essentially zero, so the sign of each
bit is decided by rounding noise. Two encodes of one tone then score a bit-error
rate of about 0.46 — indistinguishable from two unrelated recordings — and the
duplicate is missed.

So each file also gets a **spectral shape template**: the mean energy of every
band relative to the loudest band, clamped to an 80 dB window and quantised.
Subtracting the peak makes it volume-invariant, and the clamp puts every file's
noise floor on the same value, so one codec's idea of silence does not read as a
difference in the audio. Files whose median frame-to-frame change falls below a
threshold are matched on this template instead, weighted towards the bands that
actually carry sound.

The two populations are far apart, so nothing has to be guessed at: measured
across the test material, stationary audio ranges from 0.004 to 0.058 and music
from 0.223 to 0.414, with the threshold at 0.10 between them. On the template
itself, encodes of one tone stay within 0.13 of each other while different tones
sit above 0.60.

Stationary files are also the one case where length is checked directly. Normal
recordings may differ in length as much as they like, because matching a short
excerpt against the full recording is a real result worth reporting. A held tone
is different: every second of it sounds like every other, so a ten-second tone
aligns perfectly inside a ten-minute one and "excerpt of" stops carrying any
information. When both files are stationary they must therefore be of comparable
length — within 10%, or half a second, whichever is larger — no matter which of
the two passes found them.

## Layout

```
Sources/CVorbis/        Ogg Vorbis decoding (stb_vorbis + a small C shim)
Sources/DupliDetect/    The app
  AudioDecoder.swift    Decoding, resampling, format naming
  Fingerprint.swift     FFT, the acoustic fingerprint and the shape template
  Matcher.swift         Indexing, offset alignment, scoring, grouping
  Scanner.swift         Folder walking and the two-pass scan
  FileHasher.swift      Streaming SHA-256
  AppModel.swift        View model, keep rules, trashing
  PreviewPlayer.swift   Audition playback through AVAudioEngine
  *View.swift           SwiftUI interface
Tools/MakeIcon.swift    Generates the app icon artwork
build.sh                Universal build + .app assembly
```

## Limitations

- Opus and WMA cannot be audio-matched on macOS (no system decoder).
- Only the first 120 seconds of each file are fingerprinted; two tracks that are
  identical for two minutes and diverge later will be grouped.
- Stationary audio is matched on timbre plus length, so two takes of one steady
  tone are grouped only when their durations are close. Two tones of the same
  pitch and character but clearly different lengths are reported as separate
  files, since nothing in the audio distinguishes an excerpt from the whole.
- Matching is content-based, so genuinely different recordings of the same piece
  (a live take vs the studio take) are correctly *not* matched.
