//! Pair subtitle files with the video they belong to.
//!
//! The common layouts are: a sibling file sharing the video's base name, a
//! `Subs/` folder next to the video, or a lone subtitle in a single-video
//! release folder. Each subtitle is assigned to at most one video.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::scanner::FoundFile;

/// Folders that conventionally hold subtitles for the video one level up.
const SUB_DIRS: &[&str] = &["subs", "subtitles", "sub", "subtitle"];

/// Minimum score for a pairing to be trusted at all.
const MIN_SCORE: f64 = 0.45;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleMatch {
    pub path: PathBuf,
    /// ISO 639-1 where we could work it out.
    pub language: Option<String>,
    pub forced: bool,
    pub sdh: bool,
    pub score: f64,
    /// How the pairing was made, shown in the preview so a wrong guess is
    /// visible before anything moves.
    pub reason: String,
}

static LANGUAGES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    [
        ("en", "en"),
        ("eng", "en"),
        ("english", "en"),
        ("fr", "fr"),
        ("fre", "fr"),
        ("fra", "fr"),
        ("french", "fr"),
        ("es", "es"),
        ("spa", "es"),
        ("spanish", "es"),
        ("espanol", "es"),
        ("de", "de"),
        ("ger", "de"),
        ("deu", "de"),
        ("german", "de"),
        ("it", "it"),
        ("ita", "it"),
        ("italian", "it"),
        ("pt", "pt"),
        ("por", "pt"),
        ("portuguese", "pt"),
        ("nl", "nl"),
        ("dut", "nl"),
        ("dutch", "nl"),
        ("sv", "sv"),
        ("swe", "sv"),
        ("swedish", "sv"),
        ("no", "no"),
        ("nor", "no"),
        ("norwegian", "no"),
        ("da", "da"),
        ("dan", "da"),
        ("danish", "da"),
        ("fi", "fi"),
        ("fin", "fi"),
        ("finnish", "fi"),
        ("pl", "pl"),
        ("pol", "pl"),
        ("polish", "pl"),
        ("ru", "ru"),
        ("rus", "ru"),
        ("russian", "ru"),
        ("ja", "ja"),
        ("jpn", "ja"),
        ("japanese", "ja"),
        ("ko", "ko"),
        ("kor", "ko"),
        ("korean", "ko"),
        ("zh", "zh"),
        ("chi", "zh"),
        ("zho", "zh"),
        ("chinese", "zh"),
        ("ar", "ar"),
        ("ara", "ar"),
        ("arabic", "ar"),
        ("hi", "hi"),
        ("hin", "hi"),
        ("hindi", "hi"),
        ("ta", "ta"),
        ("tam", "ta"),
        ("tamil", "ta"),
        ("si", "si"),
        ("sin", "si"),
        ("sinhala", "si"),
        ("tr", "tr"),
        ("tur", "tr"),
        ("turkish", "tr"),
        ("cs", "cs"),
        ("cze", "cs"),
        ("czech", "cs"),
        ("he", "he"),
        ("heb", "he"),
        ("hebrew", "he"),
        ("th", "th"),
        ("tha", "th"),
        ("thai", "th"),
        ("id", "id"),
        ("ind", "id"),
        ("indonesian", "id"),
        ("vi", "vi"),
        ("vie", "vi"),
        ("vietnamese", "vi"),
    ]
    .into_iter()
    .collect()
});

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Lowercase alphanumerics only, so "Movie.Name-2010" and "movie name 2010"
/// compare equal.
fn squash(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn parent_name_lower(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn in_subs_dir(path: &Path) -> bool {
    SUB_DIRS.contains(&parent_name_lower(path).as_str())
}

/// The directory a subtitle's video is expected to live in.
fn effective_dir(sub: &Path) -> Option<PathBuf> {
    let parent = sub.parent()?;
    if in_subs_dir(sub) {
        parent.parent().map(|p| p.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

/// Pull language/forced/SDH hints out of whatever is left of the subtitle name
/// once the video's own name is removed.
fn describe(sub: &Path, leftover: &str) -> (Option<String>, bool, bool) {
    let mut haystack = leftover.to_lowercase();

    // A subtitle inside Subs/ is often named for the language alone.
    if in_subs_dir(sub) {
        haystack.push(' ');
        haystack.push_str(&stem_of(sub).to_lowercase());
    }

    let parts: Vec<String> = haystack
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let forced = parts.iter().any(|p| p == "forced");
    // Deliberately not treating "hi" as hearing-impaired: it collides with the
    // ISO code for Hindi and the language reading is far more common.
    let sdh = parts.iter().any(|p| p == "sdh" || p == "cc");

    let language = parts
        .iter()
        .find_map(|p| LANGUAGES.get(p.as_str()).map(|c| c.to_string()));

    (language, forced, sdh)
}

/// Score a candidate video/subtitle pairing. Returns None when they are
/// unrelated.
fn score_pair(video: &Path, sub: &Path, videos_in_dir: usize) -> Option<(f64, String, String)> {
    let video_dir = video.parent()?;
    let sub_dir = effective_dir(sub)?;
    if squash(&video_dir.to_string_lossy()) != squash(&sub_dir.to_string_lossy()) {
        return None;
    }

    let v_stem = stem_of(video);
    let s_stem = stem_of(sub);
    let v_squashed = squash(&v_stem);
    let s_squashed = squash(&s_stem);

    if v_squashed.is_empty() {
        return None;
    }

    if v_squashed == s_squashed {
        return Some((1.0, String::new(), "exact name match".to_string()));
    }

    if s_squashed.starts_with(&v_squashed) {
        // "Movie.Name.en.forced.srt" -> leftover "en.forced"
        let leftover = s_stem
            .get(v_stem.len().min(s_stem.len())..)
            .unwrap_or("")
            .to_string();
        return Some((0.9, leftover, "name prefix match".to_string()));
    }

    if in_subs_dir(sub) && videos_in_dir == 1 {
        return Some((
            0.7,
            s_stem.clone(),
            "only video beside a Subs folder".to_string(),
        ));
    }

    if videos_in_dir == 1 {
        return Some((
            0.55,
            s_stem.clone(),
            "only video in the folder".to_string(),
        ));
    }

    None
}

/// Assign every subtitle to its best video.
pub fn match_subtitles(
    videos: &[FoundFile],
    subtitles: &[FoundFile],
) -> HashMap<PathBuf, Vec<SubtitleMatch>> {
    let mut videos_per_dir: HashMap<String, usize> = HashMap::new();
    for video in videos {
        if let Some(dir) = video.path.parent() {
            *videos_per_dir
                .entry(squash(&dir.to_string_lossy()))
                .or_insert(0) += 1;
        }
    }

    let mut out: HashMap<PathBuf, Vec<SubtitleMatch>> = HashMap::new();

    for sub in subtitles {
        let mut best: Option<(f64, &FoundFile, String, String)> = None;

        for video in videos {
            let count = video
                .path
                .parent()
                .and_then(|d| videos_per_dir.get(&squash(&d.to_string_lossy())))
                .copied()
                .unwrap_or(0);

            if let Some((score, leftover, reason)) = score_pair(&video.path, &sub.path, count) {
                let better = match &best {
                    Some((current, _, _, _)) => score > *current,
                    None => true,
                };
                if better {
                    best = Some((score, video, leftover, reason));
                }
            }
        }

        if let Some((score, video, leftover, reason)) = best {
            if score >= MIN_SCORE {
                let (language, forced, sdh) = describe(&sub.path, &leftover);
                out.entry(video.path.clone()).or_default().push(SubtitleMatch {
                    path: sub.path.clone(),
                    language,
                    forced,
                    sdh,
                    score,
                    reason,
                });
            }
        }
    }

    // Strongest match first, then by language for a stable order.
    for matches in out.values_mut() {
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.language.cmp(&b.language))
        });
    }

    out
}

/// Filter to the languages the user asked for. An empty preference list keeps
/// everything, and unknown-language subtitles are always kept - dropping a
/// subtitle we simply failed to label would lose data.
pub fn filter_languages(matches: &[SubtitleMatch], preferred: &[String]) -> Vec<SubtitleMatch> {
    if preferred.is_empty() {
        return matches.to_vec();
    }
    let wanted: Vec<String> = preferred.iter().map(|p| p.to_lowercase()).collect();
    matches
        .iter()
        .filter(|m| match &m.language {
            Some(lang) => wanted.contains(&lang.to_lowercase()),
            None => true,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FoundFile {
        FoundFile {
            path: PathBuf::from(path),
            size: 1000,
        }
    }

    #[test]
    fn pairs_a_sibling_subtitle_by_name() {
        let videos = vec![file(r"D:\dl\Inception.2010\Inception.2010.mkv")];
        let subs = vec![file(r"D:\dl\Inception.2010\Inception.2010.en.srt")];
        let out = match_subtitles(&videos, &subs);
        let matched = &out[&videos[0].path];
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].language.as_deref(), Some("en"));
    }

    #[test]
    fn pairs_from_a_subs_folder() {
        let videos = vec![file(r"D:\dl\Arrival\Arrival.mkv")];
        let subs = vec![file(r"D:\dl\Arrival\Subs\English.srt")];
        let out = match_subtitles(&videos, &subs);
        let matched = &out[&videos[0].path];
        assert_eq!(matched[0].language.as_deref(), Some("en"));
    }

    #[test]
    fn keeps_episodes_apart_in_a_shared_folder() {
        let videos = vec![
            file(r"D:\dl\Show\Show.S01E01.mkv"),
            file(r"D:\dl\Show\Show.S01E02.mkv"),
        ];
        let subs = vec![file(r"D:\dl\Show\Show.S01E02.en.srt")];
        let out = match_subtitles(&videos, &subs);
        assert!(!out.contains_key(&videos[0].path));
        assert_eq!(out[&videos[1].path].len(), 1);
    }

    #[test]
    fn flags_forced_subtitles() {
        let videos = vec![file(r"D:\dl\Dune\Dune.mkv")];
        let subs = vec![file(r"D:\dl\Dune\Dune.en.forced.srt")];
        let out = match_subtitles(&videos, &subs);
        assert!(out[&videos[0].path][0].forced);
    }

    #[test]
    fn language_filter_keeps_unlabelled_subtitles() {
        let matches = vec![
            SubtitleMatch {
                path: PathBuf::from("a.srt"),
                language: Some("fr".into()),
                forced: false,
                sdh: false,
                score: 1.0,
                reason: String::new(),
            },
            SubtitleMatch {
                path: PathBuf::from("b.srt"),
                language: None,
                forced: false,
                sdh: false,
                score: 1.0,
                reason: String::new(),
            },
        ];
        let kept = filter_languages(&matches, &["en".to_string()]);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].language.is_none());
    }
}
