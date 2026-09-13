//! Subtitle fetching from SubDL.
//!
//! SubDL has no file-hash matching, so the best we can do is pick the
//! subtitle whose release name looks most like ours: same group, same source,
//! same resolution. The parser already extracts those tags, which makes the
//! comparison cheap and usually right.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::parser::{MediaKind, ParsedName};

const API_ROOT: &str = "https://api.subdl.com/api/v1";
const DOWNLOAD_ROOT: &str = "https://dl.subdl.com";

const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "sub", "vtt"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubdlSubtitle {
    pub release_name: String,
    pub name: String,
    /// Path on dl.subdl.com, e.g. "/subtitle/2506211-1689020.zip".
    pub url: String,
    /// ISO 639-1 lowercase where SubDL gives one, else whatever it said.
    pub language: String,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub hearing_impaired: bool,
    pub full_season: bool,
}

/// What to look for. TMDB id is the reliable key; the title is a fallback.
#[derive(Debug, Clone)]
pub struct SubtitleQuery<'a> {
    pub kind: MediaKind,
    pub tmdb_id: Option<u32>,
    pub title: &'a str,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub languages: &'a [String],
}

pub struct SubdlClient {
    http: reqwest::Client,
    api_key: String,
}

fn normalise_language(raw: &str) -> String {
    let lower = raw.trim().to_lowercase().replace('_', "-");
    // "english" -> "en", "brazillian portuguese" -> keep as is; codes pass through.
    match lower.as_str() {
        "english" => "en".into(),
        "french" => "fr".into(),
        "spanish" => "es".into(),
        "german" => "de".into(),
        "italian" => "it".into(),
        "portuguese" => "pt".into(),
        "dutch" => "nl".into(),
        "arabic" => "ar".into(),
        "hindi" => "hi".into(),
        "tamil" => "ta".into(),
        "sinhala" => "si".into(),
        "korean" => "ko".into(),
        "japanese" => "ja".into(),
        "chinese" => "zh".into(),
        "russian" => "ru".into(),
        "turkish" => "tr".into(),
        _ => lower,
    }
}

fn number(value: Option<&Value>) -> Option<u16> {
    let n = value?.as_u64().or_else(|| value?.as_str()?.parse().ok())?;
    (n > 0).then_some(n as u16)
}

impl SubdlClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Cinefold/0.1")
            .build()
            .unwrap_or_default();
        Self {
            http,
            api_key: api_key.into(),
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// A cheap call that proves the key is accepted.
    pub async fn validate_key(&self) -> Result<bool> {
        let body = self
            .raw_search(&[
                ("film_name", "Inception".to_string()),
                ("type", "movie".to_string()),
                ("languages", "EN".to_string()),
                ("subs_per_page", "1".to_string()),
            ])
            .await?;
        Ok(body.get("status").and_then(|s| s.as_bool()).unwrap_or(false))
    }

    async fn raw_search(&self, params: &[(&str, String)]) -> Result<Value> {
        if !self.has_key() {
            return Err(anyhow!("no SubDL API key configured"));
        }
        let mut query: Vec<(&str, String)> = vec![("api_key", self.api_key.clone())];
        query.extend(params.iter().cloned());

        let response = self
            .http
            .get(format!("{API_ROOT}/subtitles"))
            .query(&query)
            .send()
            .await
            .context("contacting SubDL")?;

        match response.status().as_u16() {
            401 | 403 => return Err(anyhow!("SubDL rejected the API key")),
            429 => return Err(anyhow!("SubDL daily limit reached")),
            code if code >= 400 => return Err(anyhow!("SubDL returned {code}")),
            _ => {}
        }
        let body: Value = response.json().await.context("decoding SubDL response")?;
        if let Some(err) = body.get("error").and_then(|e| e.as_str()) {
            return Err(anyhow!("SubDL: {err}"));
        }
        Ok(body)
    }

    pub async fn search(&self, query: &SubtitleQuery<'_>) -> Result<Vec<SubdlSubtitle>> {
        let languages = if query.languages.is_empty() {
            "EN".to_string()
        } else {
            query
                .languages
                .iter()
                .map(|l| l.to_uppercase())
                .collect::<Vec<_>>()
                .join(",")
        };

        let mut params: Vec<(&str, String)> = vec![
            (
                "type",
                match query.kind {
                    MediaKind::Movie => "movie",
                    MediaKind::Tv => "tv",
                }
                .to_string(),
            ),
            ("languages", languages),
            ("subs_per_page", "30".to_string()),
        ];
        match query.tmdb_id {
            Some(id) => params.push(("tmdb_id", id.to_string())),
            None => params.push(("film_name", query.title.to_string())),
        }
        if query.kind == MediaKind::Tv {
            if let Some(s) = query.season {
                params.push(("season_number", s.to_string()));
            }
            if let Some(e) = query.episode {
                params.push(("episode_number", e.to_string()));
            }
        }

        let body = self.raw_search(&params).await?;
        let list = body
            .get("subtitles")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(list
            .iter()
            .filter_map(|item| {
                let url = item.get("url")?.as_str()?.to_string();
                Some(SubdlSubtitle {
                    release_name: item
                        .get("release_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    url,
                    language: normalise_language(
                        item.get("language")
                            .or_else(|| item.get("lang"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("en"),
                    ),
                    season: number(item.get("season")),
                    episode: number(item.get("episode")),
                    hearing_impaired: item.get("hi").and_then(|v| v.as_bool()).unwrap_or(false),
                    full_season: item
                        .get("full_season")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                })
            })
            .collect())
    }

    /// Fetch the subtitle archive.
    pub async fn download(&self, url_path: &str) -> Result<Vec<u8>> {
        let url = if url_path.starts_with("http") {
            url_path.to_string()
        } else {
            format!("{DOWNLOAD_ROOT}{url_path}")
        };
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("downloading {url}"))?;
        if !response.status().is_success() {
            return Err(anyhow!("download failed with {}", response.status()));
        }
        Ok(response.bytes().await?.to_vec())
    }
}

// ---------------------------------------------------------------------------
// Choosing the best subtitle
// ---------------------------------------------------------------------------

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// How well a release name matches our file. Higher is better.
fn score(sub: &SubdlSubtitle, parsed: &ParsedName, want_language: &str) -> i32 {
    if sub.language != want_language {
        return i32::MIN;
    }

    let release = tokens(&sub.release_name);
    let mut points = 0;

    // Episode agreement, where SubDL recorded it. A whole-season pack is
    // usable but a single-episode file is preferred.
    if parsed.kind == MediaKind::Tv {
        match (parsed.season, parsed.episode) {
            (Some(s), Some(e)) => {
                let tag = format!("s{s:02}e{e:02}");
                let in_name = sub.release_name.to_lowercase().contains(&tag);
                let recorded = sub.season == Some(s) && sub.episode == Some(e);
                if recorded || in_name {
                    points += 6;
                } else if sub.full_season || sub.season == Some(s) {
                    points += 2;
                } else if sub.season.is_some() || sub.episode.is_some() {
                    return i32::MIN; // clearly a different episode
                }
            }
            _ => {}
        }
    }

    if let Some(group) = &parsed.group {
        if release.contains(&group.to_lowercase()) {
            points += 4;
        }
    }
    if let Some(source) = &parsed.source {
        let source = source.to_lowercase().replace('-', "");
        if release.iter().any(|t| t.replace('-', "") == source) {
            points += 3;
        }
    }
    if let Some(resolution) = &parsed.resolution {
        if release.contains(&resolution.to_lowercase()) {
            points += 2;
        }
    }
    if let Some(codec) = &parsed.codec {
        if release.contains(&codec.to_lowercase()) {
            points += 1;
        }
    }
    if sub.hearing_impaired {
        points -= 1;
    }
    points
}

/// The most promising subtitle for this file in this language, if any.
pub fn pick_best<'a>(
    candidates: &'a [SubdlSubtitle],
    parsed: &ParsedName,
    language: &str,
) -> Option<&'a SubdlSubtitle> {
    candidates
        .iter()
        .map(|c| (score(c, parsed, language), c))
        .filter(|(s, _)| *s > i32::MIN)
        .max_by_key(|(s, _)| *s)
        .map(|(_, c)| c)
}

// ---------------------------------------------------------------------------
// Unpacking
// ---------------------------------------------------------------------------

/// Pull the subtitle files out of the archive: (filename, bytes).
pub fn extract_subtitles(zip_bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("the download was not a zip")?;
    let mut out = Vec::new();

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)?;
        out.push((name, bytes));
    }
    Ok(out)
}

/// From a possibly multi-file archive, the entry for our episode (or the
/// largest file, for films and packs that do not label episodes).
pub fn choose_entry<'a>(
    entries: &'a [(String, Vec<u8>)],
    parsed: &ParsedName,
) -> Option<&'a (String, Vec<u8>)> {
    if entries.is_empty() {
        return None;
    }
    if let (MediaKind::Tv, Some(s), Some(e)) = (parsed.kind, parsed.season, parsed.episode) {
        let tag = format!("s{s:02}e{e:02}");
        let alt = format!("{s}x{e:02}");
        if let Some(hit) = entries.iter().find(|(name, _)| {
            let lower = name.to_lowercase();
            lower.contains(&tag) || lower.contains(&alt)
        }) {
            return Some(hit);
        }
        if entries.len() > 1 {
            // Several files and none labelled for us: too risky to guess.
            return None;
        }
    }
    entries.iter().max_by_key(|(_, bytes)| bytes.len())
}

// ---------------------------------------------------------------------------
// End to end for one video
// ---------------------------------------------------------------------------

/// Whether a subtitle already sits beside the video (same stem, any language).
pub fn has_subtitle_beside(video: &Path) -> bool {
    let Some(dir) = video.parent() else {
        return false;
    };
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.filter_map(|e| e.ok()).any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                let ext = Path::new(&name)
                    .extension()
                    .map(|x| x.to_string_lossy().to_string())
                    .unwrap_or_default();
                name.starts_with(&stem) && SUBTITLE_EXTENSIONS.contains(&ext.as_str())
            })
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchOutcome {
    pub written: Vec<std::path::PathBuf>,
    /// Languages that had nothing suitable.
    pub missing: Vec<String>,
}

/// Search once, then download the best match for each wanted language and
/// write it beside the video as `<stem>.<lang>.<ext>`. Never overwrites.
pub async fn fetch_for_video(
    client: &SubdlClient,
    video: &Path,
    parsed: &ParsedName,
    tmdb_id: Option<u32>,
    languages: &[String],
    mut on_written: impl FnMut(&Path),
) -> Result<FetchOutcome> {
    let wanted: Vec<String> = if languages.is_empty() {
        vec!["en".to_string()]
    } else {
        languages.iter().map(|l| l.to_lowercase()).collect()
    };

    let candidates = client
        .search(&SubtitleQuery {
            kind: parsed.kind,
            tmdb_id,
            title: &parsed.title,
            season: parsed.season,
            episode: parsed.episode,
            languages: &wanted,
        })
        .await?;

    let dir = video.parent().unwrap_or_else(|| Path::new("."));
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut outcome = FetchOutcome::default();
    for language in &wanted {
        let Some(best) = pick_best(&candidates, parsed, language) else {
            outcome.missing.push(language.clone());
            continue;
        };
        let archive = client.download(&best.url).await?;
        let entries = extract_subtitles(&archive)?;
        let Some((name, bytes)) = choose_entry(&entries, parsed) else {
            outcome.missing.push(language.clone());
            continue;
        };
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "srt".to_string());

        let target = crate::fsutil::unique_destination(&dir.join(format!("{stem}.{language}.{ext}")));
        crate::fsutil::write_atomic(&target, bytes)
            .with_context(|| format!("writing {}", target.display()))?;
        on_written(&target);
        outcome.written.push(target);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse, DEFAULT_TOKENS};

    fn sub(release: &str, lang: &str, season: Option<u16>, episode: Option<u16>) -> SubdlSubtitle {
        SubdlSubtitle {
            release_name: release.into(),
            name: release.into(),
            url: "/subtitle/1-1.zip".into(),
            language: lang.into(),
            season,
            episode,
            hearing_impaired: false,
            full_season: false,
        }
    }

    #[test]
    fn prefers_the_matching_release_group_and_source() {
        let parsed = parse("Inception.2010.1080p.BluRay.x264-SPARKS", &DEFAULT_TOKENS);
        let candidates = vec![
            sub("Inception.2010.720p.WEB-DL.x264-KILLERS", "en", None, None),
            sub("Inception.2010.1080p.BluRay.x264-SPARKS", "en", None, None),
            sub("Inception.2010.1080p.BluRay.x264-SPARKS", "fr", None, None),
        ];
        let best = pick_best(&candidates, &parsed, "en").unwrap();
        assert!(best.release_name.contains("SPARKS"));
        assert_eq!(best.language, "en");
    }

    #[test]
    fn rejects_a_different_episode() {
        let parsed = parse("Silo.S01E02.1080p.WEB.H264-NTb", &DEFAULT_TOKENS);
        let candidates = vec![
            sub("Silo.S01E03.1080p.WEB.H264-NTb", "en", Some(1), Some(3)),
            sub("Silo.S01E02.1080p.WEB.H264-NTb", "en", Some(1), Some(2)),
        ];
        let best = pick_best(&candidates, &parsed, "en").unwrap();
        assert!(best.release_name.contains("S01E02"));
    }

    #[test]
    fn nothing_in_the_wanted_language_means_none() {
        let parsed = parse("Inception.2010.1080p", &DEFAULT_TOKENS);
        let candidates = vec![sub("Inception.2010.1080p", "fr", None, None)];
        assert!(pick_best(&candidates, &parsed, "en").is_none());
    }

    #[test]
    fn picks_the_episode_out_of_a_season_pack() {
        let parsed = parse("Silo.S01E02.1080p", &DEFAULT_TOKENS);
        let entries = vec![
            ("Silo.S01E01.srt".to_string(), vec![1u8; 10]),
            ("Silo.S01E02.srt".to_string(), vec![1u8; 5]),
        ];
        assert_eq!(choose_entry(&entries, &parsed).unwrap().0, "Silo.S01E02.srt");
    }

    #[test]
    fn language_names_become_codes() {
        assert_eq!(normalise_language("English"), "en");
        assert_eq!(normalise_language("EN"), "en");
        assert_eq!(normalise_language("pt_BR"), "pt-br");
    }
}
