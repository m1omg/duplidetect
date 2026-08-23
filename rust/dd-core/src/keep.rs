//! Which copy to keep when auto-selecting duplicates for removal.

use crate::model::{quality_cmp, AudioFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepRule {
    BestQuality,
    LargestFile,
    SmallestFile,
    Oldest,
    Newest,
    ShortestPath,
}

impl KeepRule {
    pub const ALL: [KeepRule; 6] = [
        KeepRule::BestQuality,
        KeepRule::LargestFile,
        KeepRule::SmallestFile,
        KeepRule::Oldest,
        KeepRule::Newest,
        KeepRule::ShortestPath,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            KeepRule::BestQuality => "Best quality",
            KeepRule::LargestFile => "Largest file",
            KeepRule::SmallestFile => "Smallest file",
            KeepRule::Oldest => "Oldest file",
            KeepRule::Newest => "Newest file",
            KeepRule::ShortestPath => "Shallowest folder",
        }
    }

    pub fn explanation(&self) -> &'static str {
        match self {
            KeepRule::BestQuality => "Lossless first, then sample rate and bitrate — not file size",
            KeepRule::LargestFile => "Keeps the copy that takes the most space",
            KeepRule::SmallestFile => "Keeps the copy that takes the least space",
            KeepRule::Oldest => "Keeps the copy that was modified longest ago",
            KeepRule::Newest => "Keeps the most recently modified copy",
            KeepRule::ShortestPath => "Keeps the copy nearest the top of your folders",
        }
    }

    /// Picks the single file to keep out of a group.
    pub fn keeper<'a>(&self, files: &'a [AudioFile]) -> Option<&'a AudioFile> {
        match self {
            KeepRule::BestQuality => {
                // Never trade away audio for fidelity: a copy missing a chunk of
                // the recording is disqualified no matter how good it sounds.
                // Silent padding does not count, so a merely padded file still
                // competes on equal terms.
                let longest = files
                    .iter()
                    .filter_map(|f| f.content_duration)
                    .fold(0.0f64, f64::max);
                let complete: Vec<&AudioFile> = if longest > 0.0 {
                    files
                        .iter()
                        .filter(|f| f.content_duration.unwrap_or(longest) >= longest * 0.95)
                        .collect()
                } else {
                    Vec::new()
                };
                let pool: Vec<&AudioFile> =
                    if complete.is_empty() { files.iter().collect() } else { complete };
                pool.into_iter().max_by(|a, b| quality_cmp(a, b))
            }
            KeepRule::LargestFile => files.iter().max_by_key(|f| f.byte_size),
            KeepRule::SmallestFile => files.iter().min_by_key(|f| f.byte_size),
            KeepRule::Oldest => files.iter().min_by_key(|f| f.modified),
            KeepRule::Newest => files.iter().max_by_key(|f| f.modified),
            KeepRule::ShortestPath => files.iter().min_by(|a, b| {
                let (ap, bp) = (a.path.display().to_string(), b.path.display().to_string());
                a.path
                    .components()
                    .count()
                    .cmp(&b.path.components().count())
                    .then_with(|| ap.len().cmp(&bp.len()))
                    // Equally shallow paths of equal length would otherwise be
                    // a tie decided by scan order, which is not reproducible.
                    .then_with(|| ap.cmp(&bp))
            }),
        }
    }
}
