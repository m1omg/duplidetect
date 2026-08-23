DupliDetect for Linux
=====================

There are two downloads; either is fine, and neither installs anything or
needs root access.

  DupliDetect-x86_64.AppImage   one file. Make it executable and run it:
                                    chmod +x DupliDetect-x86_64.AppImage
                                    ./DupliDetect-x86_64.AppImage
                                Most desktops will also let you just
                                double-click it once it is executable.

  DupliDetect-linux-x86_64      a folder with the plain binaries, if you would
                                rather not use an AppImage:
                                    chmod +x DupliDetect ddcli
                                    ./DupliDetect

The AppImage is the simpler of the two: one file, nothing to unpack. It needs
libfuse2, which Linux Mint already has. On a system without it, either install
libfuse2 or run the AppImage with --appimage-extract-and-run, or just use the
plain binaries instead.

Requirements
------------
Only what a desktop Linux system already has: glibc 2.35 or newer, X11 or
Wayland, and OpenGL. Tested against Linux Mint Debian Edition. Sound preview
additionally uses ALSA, which is standard.

Using it
--------
1. Drag folders onto the window, or click "Choose Folder...".
2. Pick what to look for and how strict matching should be. The default,
   "Perfect match", groups only files that are the same recording from start to
   finish -- true 1:1 duplicates. Looser levels also group excerpts and heavily
   re-encoded copies; review those before deleting anything.
3. Click "Scan for Duplicates".

Files you mark are moved to the desktop Trash, never deleted permanently, so a
mistake is always recoverable. A group will never let you mark every copy --
one always stays.

Checking the build
------------------
ddcli is a command-line version of the same engine. Run:

    ./ddcli selftest

It verifies that the audio analysis on your machine matches the reference
implementation exactly. You can also scan without the interface:

    ./ddcli scan ~/Music

If the window fails to open on a machine without graphics acceleration, try:

    LIBGL_ALWAYS_SOFTWARE=1 ./DupliDetect

Adding it to your menu
----------------------
Copy DupliDetect.desktop to ~/.local/share/applications/ and edit its Exec line
to the full path of the DupliDetect binary.

Formats
-------
Decoded and audio-matched: WAV, AIFF, MP3, M4A/AAC, Apple Lossless, FLAC,
CAF (uncompressed), Ogg Vorbis.

Found only as exact byte-for-byte copies: Opus, WMA, WavPack, Monkey's Audio,
Matroska Audio, RealAudio, AC-3, AMR, and AAC stored inside a .caf container.
DupliDetect tells you when it skips a file and why.
