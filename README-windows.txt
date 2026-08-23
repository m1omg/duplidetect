DupliDetect for Windows
=======================

Just run DupliDetect.exe. There is no installer, nothing to set up, and it does
not need administrator rights. You can keep it anywhere -- a folder, a USB
stick, wherever suits you.

First run
---------
Windows will probably show a blue "Windows protected your PC" box, because this
program is not code-signed. Click "More info", then "Run anyway". It only has to
be done once.

Using it
--------
1. Drag folders onto the window, or click "Choose Folder...".
2. Pick what to look for and how strict matching should be. The default,
   "Perfect match", groups only files that are the same recording from start to
   finish -- true 1:1 duplicates. Looser levels also group excerpts and heavily
   re-encoded copies; review those before deleting anything.
3. Click "Scan for Duplicates".

Files you mark are moved to the Recycle Bin, never deleted permanently, so a
mistake is always recoverable. A group will never let you mark every copy --
one always stays.

Checking the build
------------------
ddcli.exe is a command-line version of the same engine. Run:

    ddcli.exe selftest

It verifies that the audio analysis on your machine matches the reference
implementation exactly. You can also scan without the interface:

    ddcli.exe scan "C:\Users\you\Music"

Formats
-------
Decoded and audio-matched: WAV, AIFF, MP3, M4A/AAC, Apple Lossless, FLAC,
CAF (uncompressed), Ogg Vorbis.

Found only as exact byte-for-byte copies: Opus, WMA, WavPack, Monkey's Audio,
Matroska Audio, RealAudio, AC-3, AMR, and AAC stored inside a .caf container.
DupliDetect tells you when it skips a file and why.
