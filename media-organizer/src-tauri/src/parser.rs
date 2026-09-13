//! Filename to metadata guesser.
//!
//! Built as a layered cleanup rather than one giant regex: pull out bracketed
//! groups, normalise separators, find the season/episode marker and the year,
//! then treat whatever sits before the earliest of those as the title. The
//! vocabulary of release junk lives in `tokens.json` so it can be extended
//! without touching this file.

use std::collections::HashSet;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

const EMBEDDED_TOKENS: &str = include_str!("../tokens.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Movie,
    Tv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleSource {
    Filename,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedName {
    pub raw: String,
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    /// Populated for multi-episode files such as S01E01E02.
    pub extra_episodes: Vec<u16>,
    /// Anime and similar use a running episode number with no season.
    pub absolute_episode: Option<u16>,
    pub kind: MediaKind,
    pub edition: Option<String>,
    pub resolution: Option<String>,
    pub source: Option<String>,
    pub codec: Option<String>,
    pub group: Option<String>,
    pub title_source: TitleSource,
}

impl ParsedName {
    fn empty(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            title: String::new(),
            year: None,
            season: None,
            episode: None,
            extra_episodes: Vec::new(),
            absolute_episode: None,
            kind: MediaKind::Movie,
            edition: None,
            resolution: None,
            source: None,
            codec: None,
            group: None,
            title_source: TitleSource::Filename,
        }
    }

    /// A rough "did the parse work" signal used to sort the review queue.
    pub fn looks_usable(&self) -> bool {
        self.title.chars().filter(|c| c.is_alphanumeric()).count() >= 2
    }
}

// ---------------------------------------------------------------------------
// Token vocabulary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenLists {
    #[serde(default)]
    pub resolution: Vec<String>,
    #[serde(default)]
    pub source: Vec<String>,
    #[serde(default)]
    pub codec: Vec<String>,
    #[serde(default)]
    pub audio: Vec<String>,
    #[serde(default)]
    pub hdr: Vec<String>,
    #[serde(default)]
    pub edition: Vec<String>,
    #[serde(default)]
    pub language: Vec<String>,
    #[serde(default)]
    pub misc: Vec<String>,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Tokens {
    resolution: HashSet<String>,
    source: HashSet<String>,
    codec: HashSet<String>,
    edition: HashSet<String>,
    groups: HashSet<String>,
    junk: HashSet<String>,
}

impl Default for Tokens {
    fn default() -> Self {
        let lists: TokenLists =
            serde_json::from_str(EMBEDDED_TOKENS).expect("embedded tokens.json is valid");
        Self::from_lists(lists)
    }
}

impl Tokens {
    pub fn from_lists(lists: TokenLists) -> Self {
        fn set(v: &[String]) -> HashSet<String> {
            v.iter().map(|s| s.to_lowercase()).collect()
        }
        let mut junk = HashSet::new();
        for group in [
            &lists.resolution,
            &lists.source,
            &lists.codec,
            &lists.audio,
            &lists.hdr,
            &lists.edition,
            &lists.language,
            &lists.misc,
            &lists.groups,
        ] {
            junk.extend(group.iter().map(|s| s.to_lowercase()));
        }
        Self {
            resolution: set(&lists.resolution),
            source: set(&lists.source),
            codec: set(&lists.codec),
            edition: set(&lists.edition),
            groups: set(&lists.groups),
            junk,
        }
    }

    /// Merge a user-supplied `tokens.json` over the embedded defaults.
    pub fn load_override(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let extra: TokenLists = serde_json::from_str(&text).ok()?;
        let mut base: TokenLists =
            serde_json::from_str(EMBEDDED_TOKENS).expect("embedded tokens.json is valid");
        base.resolution.extend(extra.resolution);
        base.source.extend(extra.source);
        base.codec.extend(extra.codec);
        base.audio.extend(extra.audio);
        base.hdr.extend(extra.hdr);
        base.edition.extend(extra.edition);
        base.language.extend(extra.language);
        base.misc.extend(extra.misc);
        base.groups.extend(extra.groups);
        Some(Self::from_lists(base))
    }

    fn is_junk(&self, token: &str) -> bool {
        let t = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '.');
        if t.is_empty() {
            return false;
        }
        let lower = t.to_lowercase();
        if self.junk.contains(&lower) {
            return true;
        }
        if RE_JUNK_SHAPE.is_match(&lower) {
            return true;
        }
        // "x264-RARBG" and friends: junk prefix, group suffix.
        if let Some((head, _tail)) = lower.split_once('-') {
            if self.junk.contains(head) || RE_JUNK_SHAPE.is_match(head) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
pub static DEFAULT_TOKENS: Lazy<Tokens> = Lazy::new(Tokens::default);

// ---------------------------------------------------------------------------
// Regexes
// ---------------------------------------------------------------------------

static RE_BRACKET: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[[^\]]*\]|\{[^}]*\}").expect("bracket regex"));

static RE_SE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bs(\d{1,2})\s*e(\d{1,3})((?:\s*-?\s*e\d{1,3})*)").expect("s01e01 regex")
});

static RE_X: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{1,3})\b").expect("1x02 regex"));

static RE_SEASON_EPISODE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bseason\s*(\d{1,2})\s*(?:episode|ep)\s*(\d{1,3})\b").expect("season regex")
});

static RE_SEASON_ONLY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bseason\s*(\d{1,2})\b").expect("season-only regex"));

static RE_YEAR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(19\d{2}|20\d{2})\b").expect("year regex"));

static RE_ANIME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*\[[^\]]+\]\s*(.+?)\s+-\s+(\d{1,3})(?:v\d)?\s*(?:\[|\(|$)")
        .expect("anime regex")
});

static RE_EPISODE_NUM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)e(\d{1,3})").expect("episode number regex"));

static RE_JUNK_SHAPE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)^(?:
            \d{3,4}p
          | x26[45]
          | h\.?26[45]
          | \d{1,2}bit
          | \d+(?:\.\d+)?ch
          | ddp?\+?\d?(?:\.\d)?
          | aac\d?(?:\.\d)?
          | dts(?:-?hd)?(?:-?ma)?
          | [a-f0-9]{8}
        )$",
    )
    .expect("junk shape regex")
});

// ---------------------------------------------------------------------------
// Tokenising
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Tok {
    text: String,
    start: usize,
}

fn strip_brackets(raw: &str) -> String {
    RE_BRACKET.replace_all(raw, " ").to_string()
}

/// Flatten release-style separators into spaces.
fn normalize(raw: &str) -> String {
    let mut s = strip_brackets(raw);
    s = s.replace(['(', ')'], " ");
    s = s.replace(['.', '_', '+'], " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokenize(norm: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, ch) in norm.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                out.push(Tok {
                    text: norm[s..i].to_string(),
                    start: s,
                });
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(Tok {
            text: norm[s..].to_string(),
            start: s,
        });
    }
    out
}

fn token_index_at(toks: &[Tok], byte: usize) -> usize {
    toks.iter()
        .position(|t| t.start >= byte)
        .unwrap_or(toks.len())
}

/// "S W A T" came from "S.W.A.T." - put it back together.
fn rejoin_acronyms(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<String> = Vec::new();

    for word in words {
        let is_single_letter = word.chars().count() == 1 && word.chars().all(|c| c.is_alphabetic());
        if is_single_letter {
            run.push(word.clone());
            continue;
        }
        if run.len() >= 3 {
            out.push(
                run.iter()
                    .map(|c| c.to_uppercase())
                    .collect::<Vec<_>>()
                    .join("."),
            );
        } else {
            out.extend(run.iter().cloned());
        }
        run.clear();
        out.push(word.clone());
    }

    if run.len() >= 3 {
        out.push(
            run.iter()
                .map(|c| c.to_uppercase())
                .collect::<Vec<_>>()
                .join("."),
        );
    } else {
        out.extend(run);
    }
    out
}

fn tidy_title(words: &[String]) -> String {
    let joined = rejoin_acronyms(words).join(" ");
    joined
        .trim_matches(|c: char| c == '-' || c == ',' || c.is_whitespace())
        .to_string()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a bare filename stem.
pub fn parse(stem: &str, tokens: &Tokens) -> ParsedName {
    let mut out = ParsedName::empty(stem);

    // Anime fansub shape: [Group] Show Name - 07 [1080p][ABCD1234]
    if let Some(caps) = RE_ANIME.captures(stem) {
        let show = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let number = caps
            .get(2)
            .and_then(|m| m.as_str().parse::<u16>().ok())
            .unwrap_or(0);
        // A four digit "episode" is almost certainly a year, so ignore those.
        if number > 0 && !show.trim().is_empty() {
            out.title = tidy_title(
                &normalize(show)
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
            );
            out.absolute_episode = Some(number);
            out.episode = Some(number);
            out.kind = MediaKind::Tv;
            annotate_tags(&mut out, stem, tokens);
            return out;
        }
    }

    let norm = normalize(stem);
    let toks = tokenize(&norm);
    if toks.is_empty() {
        return out;
    }

    // --- season / episode -------------------------------------------------
    let mut marker_idx: Option<usize> = None;

    if let Some(caps) = RE_SE.captures(&norm) {
        out.season = caps.get(1).and_then(|m| m.as_str().parse().ok());
        out.episode = caps.get(2).and_then(|m| m.as_str().parse().ok());
        if let Some(tail) = caps.get(3) {
            for extra in RE_EPISODE_NUM.captures_iter(tail.as_str()) {
                if let Some(n) = extra.get(1).and_then(|m| m.as_str().parse::<u16>().ok()) {
                    out.extra_episodes.push(n);
                }
            }
        }
        marker_idx = Some(token_index_at(&toks, caps.get(0).map(|m| m.start()).unwrap_or(0)));
    } else if let Some(caps) = RE_SEASON_EPISODE.captures(&norm) {
        out.season = caps.get(1).and_then(|m| m.as_str().parse().ok());
        out.episode = caps.get(2).and_then(|m| m.as_str().parse().ok());
        marker_idx = Some(token_index_at(&toks, caps.get(0).map(|m| m.start()).unwrap_or(0)));
    } else if let Some(caps) = RE_X.captures(&norm) {
        // Guard against resolutions written as 1920x1080.
        let season: u16 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let episode: u16 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        if season > 0 && season < 50 && episode > 0 {
            out.season = Some(season);
            out.episode = Some(episode);
            marker_idx = Some(token_index_at(&toks, caps.get(0).map(|m| m.start()).unwrap_or(0)));
        }
    }

    if out.episode.is_some() {
        out.kind = MediaKind::Tv;
    } else if let Some(caps) = RE_SEASON_ONLY.captures(&norm) {
        // Season pack with no episode marker in the name.
        out.season = caps.get(1).and_then(|m| m.as_str().parse().ok());
        out.kind = MediaKind::Tv;
        marker_idx = Some(token_index_at(&toks, caps.get(0).map(|m| m.start()).unwrap_or(0)));
    }

    // --- year -------------------------------------------------------------
    // Take the last match that is not the very first token, so
    // "Blade Runner 2049 2017" resolves to 2017 and "2012" stays a title.
    let mut year_idx: Option<usize> = None;
    for caps in RE_YEAR.captures_iter(&norm) {
        let whole = match caps.get(0) {
            Some(m) => m,
            None => continue,
        };
        let idx = token_index_at(&toks, whole.start());
        if idx == 0 {
            continue;
        }
        // A year after the season marker belongs to the release, not the title.
        if let Some(m) = marker_idx {
            if idx > m {
                continue;
            }
        }
        out.year = caps.get(1).and_then(|c| c.as_str().parse().ok());
        year_idx = Some(idx);
    }

    // --- first piece of release junk --------------------------------------
    let junk_idx = toks.iter().position(|t| tokens.is_junk(&t.text));

    // --- title ------------------------------------------------------------
    let cut = [year_idx, marker_idx, junk_idx]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(toks.len());

    let words: Vec<String> = toks
        .iter()
        .take(cut)
        .map(|t| t.text.clone())
        .filter(|w| !w.is_empty())
        .collect();

    out.title = tidy_title(&words);

    // Nothing before the marker - fall back to everything that is not junk.
    if out.title.is_empty() {
        let fallback: Vec<String> = toks
            .iter()
            .filter(|t| !tokens.is_junk(&t.text))
            .map(|t| t.text.clone())
            .collect();
        out.title = tidy_title(&fallback);
    }

    annotate_tags(&mut out, stem, tokens);
    out
}

/// Record the quality/source/group tags we recognised, for display only.
fn annotate_tags(out: &mut ParsedName, stem: &str, tokens: &Tokens) {
    let norm = normalize(stem).to_lowercase();
    for tok in norm.split_whitespace() {
        let clean = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '.');
        if clean.is_empty() {
            continue;
        }
        if out.resolution.is_none()
            && (tokens.resolution.contains(clean) || RE_JUNK_SHAPE.is_match(clean))
            && clean.ends_with('p')
        {
            out.resolution = Some(clean.to_uppercase());
        }
        if out.source.is_none() && tokens.source.contains(clean) {
            out.source = Some(clean.to_uppercase());
        }
        if out.codec.is_none() && tokens.codec.contains(clean) {
            out.codec = Some(clean.to_lowercase());
        }
        if out.edition.is_none() && tokens.edition.contains(clean) {
            let mut label = clean.to_string();
            if label == "director" || label == "directors" {
                label = "Directors Cut".to_string();
            }
            out.edition = Some(title_case(&label));
        }
        if let Some((_, tail)) = clean.rsplit_once('-') {
            if out.group.is_none() && tokens.groups.contains(tail) {
                out.group = Some(tail.to_uppercase());
            }
        }
        if out.group.is_none() && tokens.groups.contains(clean) {
            out.group = Some(clean.to_uppercase());
        }
    }

    // Bracketed leading tag is conventionally the group.
    if out.group.is_none() {
        if let Some(m) = RE_BRACKET.find(stem) {
            if m.start() == 0 {
                let inner = m
                    .as_str()
                    .trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}')
                    .trim();
                if !inner.is_empty() && inner.len() <= 24 {
                    out.group = Some(inner.to_string());
                }
            }
        }
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

static RE_SEASON_FOLDER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:season|series|s)\s*_?\s*(\d{1,2})\s*$|^\s*specials?\s*$")
        .expect("season folder regex")
});

static RE_LOOSE_NUMBER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(\d{1,3})\b").expect("loose number regex"));

/// A directory that holds episodes but never the show name, e.g. "Season 02".
fn season_folder_number(name: &str) -> Option<Option<u16>> {
    let caps = RE_SEASON_FOLDER.captures(name)?;
    Some(caps.get(1).and_then(|m| m.as_str().parse().ok()))
}

/// Placeholder filenames that rips and disc dumps leave behind. These carry no
/// information, so the release folder above them is a better source.
const GENERIC_STEMS: &[&str] = &[
    "video", "movie", "film", "main", "title", "index", "playback", "output", "encode", "default",
    "untitled", "new", "download", "file",
];

fn is_generic_stem(stem: &str) -> bool {
    let lower = stem.trim().to_lowercase();
    if lower.is_empty() {
        return true;
    }
    if GENERIC_STEMS.contains(&lower.as_str()) {
        return true;
    }
    // VTS_01_1, title00, part1, cd1 and bare numbers.
    lower.chars().all(|c| c.is_ascii_digit())
        || lower.starts_with("vts_")
        || lower.starts_with("title0")
        || lower.starts_with("video_ts")
}

/// Last-resort episode number for names like "05 - Gray Matter".
fn loose_episode(stem: &str) -> Option<u16> {
    RE_LOOSE_NUMBER
        .captures(stem)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .filter(|n| *n > 0)
}

/// Parse a full path, borrowing context from the parent folders when the
/// filename alone is uninformative.
///
/// Handles two very common shapes: a generic `video.mkv` inside a well-named
/// release folder, and `Show/Season 02/05 - Title.mkv`, where the show name
/// and season number only exist on the directories.
pub fn parse_path(path: &Path, tokens: &Tokens) -> ParsedName {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut parsed = parse(&stem, tokens);

    let parent = path.parent();
    let parent_name = parent
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string());

    if let Some(parent_name) = parent_name.as_deref() {
        if let Some(folder_season) = season_folder_number(parent_name) {
            // Inside a season folder the filename holds the episode title at
            // best, so the show name always comes from the level above.
            if let Some(grandparent) = parent
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
            {
                let show = parse(&grandparent, tokens);
                if show.looks_usable() {
                    parsed.title = show.title;
                    parsed.year = parsed.year.or(show.year);
                    parsed.title_source = TitleSource::Folder;
                }
            }
            parsed.season = parsed.season.or(folder_season);
            if parsed.episode.is_none() {
                parsed.episode = loose_episode(&stem);
            }
            parsed.kind = MediaKind::Tv;
        } else if !parsed.looks_usable() || is_generic_stem(&stem) {
            // Generic filename (VIDEO_TS, video.mkv) in a named release folder.
            let from_folder = parse(parent_name, tokens);
            if from_folder.looks_usable() {
                let season = parsed.season.or(from_folder.season);
                let episode = parsed.episode.or(from_folder.episode);
                let kind = if episode.is_some() {
                    MediaKind::Tv
                } else {
                    from_folder.kind
                };
                parsed = ParsedName {
                    season,
                    episode,
                    kind,
                    title_source: TitleSource::Folder,
                    raw: stem.clone(),
                    ..from_folder
                };
            }
        }

        if parsed.season.is_none() {
            if let Some(caps) = RE_SEASON_ONLY.captures(parent_name) {
                parsed.season = caps.get(1).and_then(|m| m.as_str().parse().ok());
            }
        }
    }

    if parsed.episode.is_some() || parsed.season.is_some() {
        parsed.kind = MediaKind::Tv;
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> ParsedName {
        parse(name, &DEFAULT_TOKENS)
    }

    #[test]
    fn parses_a_standard_movie_release() {
        let r = p("Inception.2010.1080p.BluRay.x264-RARBG");
        assert_eq!(r.title, "Inception");
        assert_eq!(r.year, Some(2010));
        assert_eq!(r.kind, MediaKind::Movie);
        assert_eq!(r.resolution.as_deref(), Some("1080P"));
        assert_eq!(r.group.as_deref(), Some("RARBG"));
    }

    #[test]
    fn parses_season_and_episode() {
        let r = p("Breaking.Bad.S01E01.Pilot.1080p.WEB-DL");
        assert_eq!(r.title, "Breaking Bad");
        assert_eq!(r.season, Some(1));
        assert_eq!(r.episode, Some(1));
        assert_eq!(r.kind, MediaKind::Tv);
    }

    #[test]
    fn parses_the_x_form() {
        let r = p("The Office 3x07 Branch Closing");
        assert_eq!(r.title, "The Office");
        assert_eq!(r.season, Some(3));
        assert_eq!(r.episode, Some(7));
    }

    #[test]
    fn keeps_a_numeric_title() {
        let r = p("2012.2009.1080p.BluRay");
        assert_eq!(r.title, "2012");
        assert_eq!(r.year, Some(2009));
    }

    #[test]
    fn prefers_the_release_year_over_a_year_in_the_title() {
        let r = p("Blade.Runner.2049.2017.2160p.UHD.BluRay");
        assert_eq!(r.title, "Blade Runner 2049");
        assert_eq!(r.year, Some(2017));
    }

    #[test]
    fn collects_multi_episode_files() {
        let r = p("Firefly.S01E01E02.720p");
        assert_eq!(r.episode, Some(1));
        assert_eq!(r.extra_episodes, vec![2]);
    }

    #[test]
    fn handles_anime_absolute_numbering() {
        let r = p("[SubsPlease] Frieren - 12 [1080p][A1B2C3D4]");
        assert_eq!(r.title, "Frieren");
        assert_eq!(r.absolute_episode, Some(12));
        assert_eq!(r.kind, MediaKind::Tv);
    }

    #[test]
    fn rebuilds_dotted_acronyms() {
        let r = p("S.W.A.T.2017.1080p.WEB");
        assert_eq!(r.title, "S.W.A.T");
        assert_eq!(r.year, Some(2017));
    }

    #[test]
    fn detects_an_edition() {
        let r = p("Blade.Runner.1982.Extended.1080p.BluRay");
        assert_eq!(r.title, "Blade Runner");
        assert_eq!(r.edition.as_deref(), Some("Extended"));
    }

    #[test]
    fn takes_the_season_from_the_parent_folder() {
        let path = Path::new(r"D:\dl\Breaking Bad\Season 02\episode 05.mkv");
        let r = parse_path(path, &DEFAULT_TOKENS);
        assert_eq!(r.season, Some(2));
        assert_eq!(r.episode, Some(5));
        assert_eq!(r.title, "Breaking Bad");
        assert_eq!(r.kind, MediaKind::Tv);
    }

    #[test]
    fn borrows_the_title_from_a_release_folder() {
        let path = Path::new(r"D:\dl\Arrival.2016.1080p.BluRay.x264\video.mkv");
        let r = parse_path(path, &DEFAULT_TOKENS);
        assert_eq!(r.title, "Arrival");
        assert_eq!(r.year, Some(2016));
        assert_eq!(r.title_source, TitleSource::Folder);
    }
}
