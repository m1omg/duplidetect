//! Plays a short excerpt so you can hear which copy to keep.
//!
//! Everything goes through DupliDetect's own decoder, so every format the
//! scanner understands can also be auditioned.

use dd_core::fingerprint::Fingerprint;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};

pub struct Preview {
    playing: Option<PathBuf>,
    command: Option<Sender<Option<PathBuf>>>,
    pub last_error: Option<String>,
}

impl Default for Preview {
    fn default() -> Self {
        Preview { playing: None, command: None, last_error: None }
    }
}

impl Preview {
    pub fn playing(&self) -> Option<&Path> {
        self.playing.as_deref()
    }

    pub fn toggle(&mut self, path: &Path) {
        if self.playing.as_deref() == Some(path) {
            self.stop();
        } else {
            self.play(path);
        }
    }

    pub fn stop(&mut self) {
        self.playing = None;
        if let Some(tx) = &self.command {
            let _ = tx.send(None);
        }
    }

    pub fn play(&mut self, path: &Path) {
        let _ = Fingerprint::empty();
        if self.command.is_none() {
            let (tx, rx) = channel::<Option<PathBuf>>();
            std::thread::spawn(move || audio_thread(rx));
            self.command = Some(tx);
        }
        self.playing = Some(path.to_path_buf());
        if let Some(tx) = &self.command {
            if tx.send(Some(path.to_path_buf())).is_err() {
                self.last_error = Some("audio output is unavailable".into());
                self.playing = None;
            }
        }
    }
}

fn audio_thread(rx: std::sync::mpsc::Receiver<Option<PathBuf>>) {
    use rodio::buffer::SamplesBuffer;
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else { return };
    let Ok(sink) = rodio::Sink::try_new(&handle) else { return };
    while let Ok(message) = rx.recv() {
        sink.stop();
        let Some(path) = message else { continue };
        // 30 seconds at playback quality is enough to compare two copies.
        if let Ok(audio) = crate::decode::decode_mono(&path, 44100.0, Some(30.0)) {
            sink.append(SamplesBuffer::new(1, 44100, audio.samples));
            sink.play();
        }
    }
}
