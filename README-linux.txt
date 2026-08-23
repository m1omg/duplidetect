DupliDetect for Linux
=====================

Unpack anywhere and run ./DupliDetect. There is nothing to install, no root
access needed, and no packages to add.

If the file is not already executable:

    chmod +x DupliDetect ddcli

Requirements
------------
Only what a desktop Linux system already has: glibc 2.35 or newer, X11 or
Wayland, and OpenGL. Tested against Linux Mint Debian Edition. Sound preview
additionally uses ALSA, which is standard.

Using it
--------
1. Drag folders onto the window, or click "Choose Folder...".
2. Pick what to look for and how strict matching should be.
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
