use std::fs::File;
use std::path::PathBuf;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn main() {
    for arg in std::env::args().skip(1) {
        let path = PathBuf::from(&arg);
        println!("--- {}", path.file_name().unwrap().to_string_lossy());
        let file = File::open(&path).unwrap();
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = match symphonia::default::get_probe().format(
            &hint, mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        ) {
            Ok(p) => p,
            Err(e) => { println!("   probe FAILED: {e}"); continue }
        };
        let mut format = probed.format;
        println!("   tracks: {}", format.tracks().len());
        for t in format.tracks() {
            println!("     id={} codec={:?} rate={:?} ch={:?} frames={:?} bits={:?} delay={:?} padding={:?} start_ts={:?}",
                     t.id, t.codec_params.codec, t.codec_params.sample_rate,
                     t.codec_params.channels.map(|c| c.count()),
                     t.codec_params.n_frames, t.codec_params.bits_per_sample,
                     t.codec_params.delay, t.codec_params.padding,
                     t.codec_params.start_ts);
        }
        let Some(track) = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL) else {
            println!("   no usable track"); continue
        };
        let track_id = track.id;
        let params = track.codec_params.clone();
        let mut decoder = match symphonia::default::get_codecs()
            .make(&params, &Default::default()) {
            Ok(d) => d,
            Err(e) => { println!("   make decoder FAILED: {e}"); continue }
        };
        let mut packets = 0; let mut frames = 0usize; let mut errs = vec![];
        for _ in 0..2000 {
            match format.next_packet() {
                Ok(p) => {
                    if p.track_id() != track_id { continue }
                    packets += 1;
                    match decoder.decode(&p) {
                        Ok(b) => frames += b.frames(),
                        Err(e) => { if errs.len() < 3 { errs.push(format!("{e}")) } }
                    }
                }
                Err(e) => { errs.push(format!("next_packet: {e}")); break }
            }
        }
        println!("   packets={packets} decodedFrames={frames} errors={errs:?}");
    }
}
