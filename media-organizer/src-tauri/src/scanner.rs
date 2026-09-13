//! Directory walk: collect video files worth organising, plus every subtitle
//! sitting near them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

use crate::fsutil;
use crate::settings::Settings;

/// Directories that never contain the main feature.
const SKIP_DIRS: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    "extras",
    "featurettes",
    "behind the scenes",
    "deleted scenes",
    "trailers",
    "sample",
    "samples",
    "proof",
    "screens",
    "node_modules",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundFile {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub path: PathBuf,
    pub size: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub videos: Vec<FoundFile>,
    pub subtitles: Vec<FoundFile>,
    pub skipped: Vec<SkippedFile>,
    pub files_seen: usize,
    pub errors: Vec<String>,
    /// True when the library lives inside the source and was left alone.
    pub library_excluded: bool,
}

fn is_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|name| name.starts_with('.') && name.len() > 1)
        .unwrap_or(false)
}

fn is_skipped_dir(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|name| SKIP_DIRS.contains(&name.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Files a release group ships beside the feature that are never the feature.
const JUNK_FILENAMES: &[&str] = &["rarbg.mp4", "rarbg.txt", "sample.mkv", "sample.mp4", "sample.avi"];

/// A sample or trailer masquerading as the feature.
///
/// The size threshold is the real signal. Name markers only apply to files
/// that are still smallish, so a full-length "Trailer Park Boys" episode or a
/// film from the RARBG group is never mistaken for a promo clip.
fn sample_reason(path: &Path, size: u64, settings: &Settings) -> Option<String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if settings.skip_samples {
        if JUNK_FILENAMES.contains(&name.as_str()) {
            return Some("release group extra".to_string());
        }

        // A marker counts only as a whole word, and only when the file is
        // too small to plausibly be the feature (4x the minimum size).
        let small = size < settings.min_bytes().saturating_mul(4).max(200 * 1024 * 1024);
        if small {
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
            let words: Vec<&str> = stem
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            for marker in ["sample", "trailer", "teaser"] {
                if words.contains(&marker) {
                    return Some(format!("looks like a {marker}"));
                }
            }
        }
    }

    if size < settings.min_bytes() {
        return Some(format!(
            "smaller than {} MB",
            settings.min_file_size_mb
        ));
    }

    None
}

/// Walk `root` and bucket everything into videos, subtitles and skips.
///
/// If the organised library sits *inside* the source (a common setup: one
/// big media drive), the library subtree is skipped so already-organised files
/// are not offered up for organising again. When the source *is* the library,
/// nothing is excluded - the planner then recognises files already in place.
///
/// `progress` is called every so often with the running file count so the UI
/// can show something during a slow network-drive scan.
pub fn scan(
    root: &Path,
    library_root: Option<&Path>,
    settings: &Settings,
    mut progress: impl FnMut(usize),
) -> Scan {
    let mut out = Scan::default();

    let exclude: Option<&Path> = library_root
        .filter(|lib| fsutil::is_within(lib, root) && !fsutil::paths_equal(lib, root));
    out.library_excluded = exclude.is_some();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |e| {
            if is_hidden(e) || is_skipped_dir(e) {
                return false;
            }
            match exclude {
                Some(lib) => !fsutil::paths_equal(e.path(), lib),
                None => true,
            }
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                out.errors.push(err.to_string());
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        out.files_seen += 1;
        if out.files_seen % 250 == 0 {
            progress(out.files_seen);
        }

        let path = entry.path().to_path_buf();
        let ext = match path.extension().map(|e| e.to_string_lossy().to_string()) {
            Some(e) => e,
            None => continue,
        };

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        if settings.is_subtitle_ext(&ext) {
            out.subtitles.push(FoundFile { path, size });
            continue;
        }

        if !settings.is_video_ext(&ext) {
            continue;
        }

        match sample_reason(&path, size, settings) {
            Some(reason) => out.skipped.push(SkippedFile { path, size, reason }),
            None => out.videos.push(FoundFile { path, size }),
        }
    }

    progress(out.files_seen);

    // Biggest first: the feature tends to matter more than the extras.
    out.videos.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_files_under_the_size_threshold() {
        let settings = Settings::default();
        let reason = sample_reason(Path::new("C:/dl/movie.mkv"), 1024, &settings);
        assert!(reason.unwrap().contains("smaller than"));
    }

    #[test]
    fn skips_a_smallish_sample_by_name() {
        let settings = Settings::default();
        let smallish = 120 * 1024 * 1024;
        let reason = sample_reason(Path::new("C:/dl/movie-sample.mkv"), smallish, &settings);
        assert!(reason.is_some());
    }

    #[test]
    fn a_large_file_is_never_a_sample_whatever_its_name() {
        let settings = Settings::default();
        let big = 4 * 1024 * 1024 * 1024;
        assert!(sample_reason(Path::new("C:/dl/Trailer.Park.Boys.S01E01.mkv"), big, &settings).is_none());
        assert!(sample_reason(Path::new("C:/dl/Rocky.1976.x265-RARBG.mp4"), big, &settings).is_none());
    }

    #[test]
    fn the_rarbg_promo_file_is_junk_by_exact_name() {
        let settings = Settings::default();
        let reason = sample_reason(Path::new("C:/dl/RARBG.mp4"), 900 * 1024 * 1024, &settings);
        assert_eq!(reason.as_deref(), Some("release group extra"));
    }

    #[test]
    fn keeps_a_normal_feature() {
        let settings = Settings::default();
        let big = 4 * 1024 * 1024 * 1024;
        assert!(sample_reason(Path::new("C:/dl/Inception.mkv"), big, &settings).is_none());
    }
}
