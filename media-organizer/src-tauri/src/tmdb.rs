//! TMDB lookup with an on-disk response cache.
//!
//! Identification is deliberately conservative: a candidate is only accepted
//! automatically when the title similarity (and year, when we have one) clears
//! the configured threshold. Everything else goes to the review queue rather
//! than being guessed at.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::parser::{MediaKind, ParsedName};

const API_ROOT: &str = "https://api.themoviedb.org/3";
pub const IMAGE_ROOT: &str = "https://image.tmdb.org/t/p";
/// Cached responses older than this are refetched.
const CACHE_TTL_SECS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: u32,
    pub title: String,
    pub original_title: String,
    pub year: Option<u16>,
    pub overview: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub popularity: f64,
    pub vote_average: f64,
    pub kind: MediaKind,
    /// 0..1 similarity against the parsed filename.
    pub score: f64,
}

impl Candidate {
    pub fn poster_url(&self, size: &str) -> Option<String> {
        self.poster_path
            .as_ref()
            .map(|p| format!("{IMAGE_ROOT}/{size}{p}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeasonInfo {
    pub season: u16,
    pub episodes: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleDetails {
    pub genres: Vec<String>,
    pub runtime_minutes: Option<u32>,
    pub imdb_id: Option<String>,
    pub status: Option<String>,
    pub tagline: Option<String>,
    pub seasons: Vec<SeasonInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identification {
    pub best: Option<Candidate>,
    pub alternatives: Vec<Candidate>,
    pub confidence: f64,
    pub needs_review: bool,
    pub reason: String,
    /// Filled in for TV episodes when the episode exists on TMDB.
    pub episode_title: Option<String>,
}

impl Identification {
    pub fn unidentified(reason: impl Into<String>) -> Self {
        Self {
            best: None,
            alternatives: Vec::new(),
            confidence: 0.0,
            needs_review: true,
            reason: reason.into(),
            episode_title: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Title similarity
// ---------------------------------------------------------------------------

/// Lowercase, drop punctuation and the leading article, normalise "&".
fn normalize_title(input: &str) -> String {
    let lowered = input.to_lowercase().replace('&', " and ");
    let cleaned: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let words = match words.split_first() {
        Some((first, rest)) if matches!(*first, "the" | "a" | "an") && !rest.is_empty() => {
            rest.to_vec()
        }
        _ => words,
    };
    words.join(" ")
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// 0..1 similarity between two titles.
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let na = normalize_title(a);
    let nb = normalize_title(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }

    let ca: Vec<char> = na.chars().collect();
    let cb: Vec<char> = nb.chars().collect();
    let distance = levenshtein(&ca, &cb) as f64;
    let longest = ca.len().max(cb.len()) as f64;
    let edit_ratio = 1.0 - (distance / longest);

    // A parsed title is often a prefix of the real one ("Dune Part Two" vs
    // "Dune: Part Two"), so reward containment as well as raw edit distance.
    let containment = if nb.starts_with(&na) || na.starts_with(&nb) {
        0.9
    } else if nb.contains(&na) || na.contains(&nb) {
        0.8
    } else {
        0.0
    };

    edit_ratio.max(containment).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: i64,
    body: Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    entries: HashMap<String, CacheEntry>,
    #[serde(skip)]
    dirty: bool,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct TmdbClient {
    http: reqwest::Client,
    api_key: String,
    cache: Mutex<Cache>,
    cache_path: PathBuf,
}

impl TmdbClient {
    pub fn new(api_key: impl Into<String>, cache_path: PathBuf) -> Self {
        let cache = std::fs::read_to_string(&cache_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Cache>(&text).ok())
            .unwrap_or_default();

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("Cinefold/0.1")
            .build()
            .unwrap_or_default();

        Self {
            http,
            api_key: api_key.into(),
            cache: Mutex::new(cache),
            cache_path,
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// Persist the cache. Called after a batch rather than per request.
    pub fn save_cache(&self) {
        let mut guard = match self.cache.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !guard.dirty {
            return;
        }
        if let Some(parent) = self.cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&*guard) {
            if std::fs::write(&self.cache_path, text).is_ok() {
                guard.dirty = false;
            }
        }
    }

    fn cache_get(&self, key: &str) -> Option<Value> {
        let guard = self.cache.lock().ok()?;
        let entry = guard.entries.get(key)?;
        let age = chrono::Utc::now().timestamp() - entry.fetched_at;
        if age > CACHE_TTL_SECS {
            return None;
        }
        Some(entry.body.clone())
    }

    fn cache_put(&self, key: &str, body: &Value) {
        if let Ok(mut guard) = self.cache.lock() {
            guard.entries.insert(
                key.to_string(),
                CacheEntry {
                    fetched_at: chrono::Utc::now().timestamp(),
                    body: body.clone(),
                },
            );
            guard.dirty = true;
        }
    }

    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Value> {
        if !self.has_key() {
            return Err(anyhow!("no TMDB API key configured"));
        }

        let key = format!(
            "{path}?{}",
            params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&")
        );
        if let Some(cached) = self.cache_get(&key) {
            return Ok(cached);
        }

        let url = format!("{API_ROOT}{path}");
        let mut query: Vec<(&str, String)> = vec![("api_key", self.api_key.clone())];
        query.extend(params.iter().cloned());

        // One retry, because TMDB answers a burst with 429 and a Retry-After.
        let mut attempt = 0;
        let body = loop {
            attempt += 1;
            let response = self
                .http
                .get(&url)
                .query(&query)
                .send()
                .await
                .with_context(|| format!("requesting {path}"))?;

            if response.status().as_u16() == 429 && attempt < 3 {
                let wait = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2);
                tokio::time::sleep(Duration::from_secs(wait.clamp(1, 10))).await;
                continue;
            }

            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(anyhow!("TMDB rejected the API key"));
            }
            if !response.status().is_success() {
                return Err(anyhow!("TMDB returned {}", response.status()));
            }

            break response
                .json::<Value>()
                .await
                .context("decoding the TMDB response")?;
        };

        self.cache_put(&key, &body);
        Ok(body)
    }

    /// Cheap call used by the settings screen to check the key works.
    pub async fn validate_key(&self) -> Result<bool> {
        let body = self.get("/configuration", &[]).await?;
        Ok(body.get("images").is_some())
    }

    pub async fn search(
        &self,
        kind: MediaKind,
        title: &str,
        year: Option<u16>,
    ) -> Result<Vec<Candidate>> {
        if title.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut params: Vec<(&str, String)> = vec![
            ("query", title.to_string()),
            ("include_adult", "false".to_string()),
        ];
        let path = match kind {
            MediaKind::Movie => {
                if let Some(y) = year {
                    params.push(("year", y.to_string()));
                }
                "/search/movie"
            }
            MediaKind::Tv => {
                if let Some(y) = year {
                    params.push(("first_air_date_year", y.to_string()));
                }
                "/search/tv"
            }
        };

        let mut body = self.get(path, &params).await?;

        // A year filter that returns nothing is usually a wrong year in the
        // filename rather than a missing film - retry without it.
        let empty = body
            .get("results")
            .and_then(|r| r.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true);
        if empty && year.is_some() {
            let retry: Vec<(&str, String)> = vec![
                ("query", title.to_string()),
                ("include_adult", "false".to_string()),
            ];
            body = self.get(path, &retry).await?;
        }

        let results = body
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let mut candidates: Vec<Candidate> = results
            .iter()
            .take(10)
            .filter_map(|item| candidate_from_json(item, kind, title, year))
            .collect();

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.popularity
                        .partial_cmp(&a.popularity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        Ok(candidates)
    }

    /// How many seasons a series has, or None if TMDB does not say.
    pub async fn tv_season_count(&self, tv_id: u32) -> Option<u16> {
        let body = self.get(&format!("/tv/{tv_id}"), &[]).await.ok()?;
        body.get("number_of_seasons")
            .and_then(|n| n.as_u64())
            .map(|n| n as u16)
    }

    /// The extra facts the collection page shows: genres, runtime, seasons.
    pub async fn details(&self, kind: MediaKind, id: u32) -> Option<TitleDetails> {
        let path = match kind {
            MediaKind::Movie => format!("/movie/{id}"),
            MediaKind::Tv => format!("/tv/{id}"),
        };
        let body = self.get(&path, &[]).await.ok()?;

        let genres = body
            .get("genres")
            .and_then(|g| g.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|g| g.get("name").and_then(|n| n.as_str()))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        let runtime_minutes = match kind {
            MediaKind::Movie => body.get("runtime").and_then(|r| r.as_u64()),
            MediaKind::Tv => body
                .get("episode_run_time")
                .and_then(|r| r.as_array())
                .and_then(|a| a.first())
                .and_then(|r| r.as_u64()),
        }
        .map(|r| r as u32)
        .filter(|r| *r > 0);

        let seasons = body
            .get("seasons")
            .and_then(|s| s.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|s| {
                        let number = s.get("season_number")?.as_u64()? as u16;
                        let episodes = s.get("episode_count")?.as_u64()? as u16;
                        // Season 0 is specials; not part of "did I finish it".
                        (number > 0).then_some(SeasonInfo { season: number, episodes })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Some(TitleDetails {
            genres,
            runtime_minutes,
            imdb_id: body
                .get("imdb_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            status: body
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            tagline: body
                .get("tagline")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            seasons,
        })
    }

    pub async fn episode_title(&self, tv_id: u32, season: u16, episode: u16) -> Option<String> {
        let path = format!("/tv/{tv_id}/season/{season}/episode/{episode}");
        let body = self.get(&path, &[]).await.ok()?;
        body.get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Look up a parsed filename and decide whether the top hit can be trusted.
    pub async fn identify(&self, parsed: &ParsedName, threshold: f64) -> Identification {
        if parsed.title.trim().is_empty() {
            return Identification::unidentified("could not read a title from the filename");
        }

        // Try the title as parsed, then each half of an "X AKA Y" title.
        let mut candidates = Vec::new();
        for query in title_variants(&parsed.title) {
            match self.search(parsed.kind, &query, parsed.year).await {
                Ok(found) if !found.is_empty() => {
                    candidates = found;
                    break;
                }
                Ok(_) => continue,
                Err(err) => return Identification::unidentified(err.to_string()),
            }
        }

        if candidates.is_empty() {
            return Identification::unidentified(format!(
                "no TMDB result for \"{}\"",
                parsed.title
            ));
        }

        // Two near-identical scores usually means a popular title sharing its
        // name with something obscure (a short, an unreleased remake). When
        // popularity separates them clearly, trust it; otherwise ask.
        let is_tied = |list: &[Candidate]| match (list.first(), list.get(1)) {
            (Some(top), Some(runner_up)) if top.score - runner_up.score < 0.06 => {
                !(top.popularity > 1.0 && top.popularity >= runner_up.popularity * 3.0)
            }
            _ => false,
        };

        let mut ambiguous = is_tied(&candidates);

        // For a series we know the season of, a namesake that does not have
        // that many seasons cannot be the right one. Check the tied leaders.
        if ambiguous && parsed.kind == MediaKind::Tv {
            if let Some(season) = parsed.season.filter(|s| *s > 0) {
                let leader_score = candidates[0].score;
                let mut kept: Vec<Candidate> = Vec::new();
                for candidate in candidates.iter() {
                    let tied_with_leader = leader_score - candidate.score < 0.06;
                    if !tied_with_leader {
                        kept.push(candidate.clone());
                        continue;
                    }
                    match self.tv_season_count(candidate.id).await {
                        Some(count) if count < season => {} // cannot be it
                        _ => kept.push(candidate.clone()),
                    }
                }
                if !kept.is_empty() {
                    candidates = kept;
                    ambiguous = is_tied(&candidates);
                }
            }
        }

        let best = candidates[0].clone();
        let confidence = best.score;
        let needs_review = confidence < threshold || ambiguous;

        let reason = if confidence < threshold {
            format!("low confidence ({:.0}%)", confidence * 100.0)
        } else if ambiguous {
            "several equally likely matches".to_string()
        } else {
            String::new()
        };

        let episode_title = match (parsed.kind, parsed.season, parsed.episode) {
            (MediaKind::Tv, Some(season), Some(episode)) if !needs_review => {
                self.episode_title(best.id, season, episode).await
            }
            _ => None,
        };

        Identification {
            alternatives: candidates.into_iter().skip(1).take(5).collect(),
            best: Some(best),
            confidence,
            needs_review,
            reason,
            episode_title,
        }
    }

    /// Download bytes (used for posters). Kept here so the cache directory and
    /// HTTP client are shared.
    pub async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("downloading {url}"))?;
        if !response.status().is_success() {
            return Err(anyhow!("download failed with {}", response.status()));
        }
        Ok(response.bytes().await?.to_vec())
    }
}

/// The title as given, then each side of an "A.K.A" split if there is one.
///
/// Releases of non-English series often carry both names in the filename:
/// "When Life Gives You Tangerines A.K.A Pokssak Sogatsuda". TMDB will only
/// ever match one half.
fn title_variants(title: &str) -> Vec<String> {
    static RE_AKA: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\s+(?:a\.?k\.?a\.?|aka)\s+").expect("aka regex"));

    let mut variants = vec![title.to_string()];
    let parts: Vec<&str> = RE_AKA
        .split(title)
        .map(|s| s.trim().trim_end_matches('.'))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() >= 2 {
        variants.extend(parts.into_iter().map(|s| s.to_string()));
    }
    variants
}

fn year_from_date(value: Option<&str>) -> Option<u16> {
    value
        .filter(|s| s.len() >= 4)
        .and_then(|s| s.get(0..4))
        .and_then(|s| s.parse().ok())
}

fn candidate_from_json(
    item: &Value,
    kind: MediaKind,
    query_title: &str,
    query_year: Option<u16>,
) -> Option<Candidate> {
    let id = item.get("id")?.as_u64()? as u32;

    let (title_key, original_key, date_key) = match kind {
        MediaKind::Movie => ("title", "original_title", "release_date"),
        MediaKind::Tv => ("name", "original_name", "first_air_date"),
    };

    let title = item
        .get(title_key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let original_title = item
        .get(original_key)
        .and_then(|v| v.as_str())
        .unwrap_or(&title)
        .to_string();
    let year = year_from_date(item.get(date_key).and_then(|v| v.as_str()));

    let similarity = title_similarity(query_title, &title)
        .max(title_similarity(query_title, &original_title));

    // Year agreement is a strong signal either way. The weights are chosen so
    // that a perfect title with the right year (1.0) sits clearly above the
    // same title with no date on TMDB (0.88) or the wrong date (0.75), which
    // keeps genuine duplicates-by-name from looking ambiguous.
    let score = match (query_year, year) {
        (Some(want), Some(got)) if want == got => similarity * 0.9 + 0.1,
        // Release-year drift of a year is common for festival titles.
        (Some(want), Some(got)) if want.abs_diff(got) == 1 => similarity * 0.9 + 0.05,
        (Some(_), Some(_)) => similarity * 0.75,
        (Some(_), None) => similarity * 0.88,
        (None, _) => similarity,
    };

    Some(Candidate {
        id,
        title,
        original_title,
        year,
        overview: item
            .get("overview")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        poster_path: item
            .get("poster_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        backdrop_path: item
            .get("backdrop_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        popularity: item
            .get("popularity")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        vote_average: item
            .get("vote_average")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        kind,
        score: score.clamp(0.0, 1.0),
    })
}

/// Where the response cache lives.
pub fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("tmdb-cache.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_titles_score_one() {
        assert_eq!(title_similarity("Inception", "Inception"), 1.0);
    }

    #[test]
    fn ignores_punctuation_and_articles() {
        assert!(title_similarity("The Matrix", "Matrix") > 0.95);
        assert!(title_similarity("Dune Part Two", "Dune: Part Two") > 0.9);
    }

    #[test]
    fn unrelated_titles_score_low() {
        assert!(title_similarity("Inception", "Frozen") < 0.4);
    }

    #[test]
    fn year_agreement_lifts_the_score() {
        let item = serde_json::json!({
            "id": 27205,
            "title": "Inception",
            "original_title": "Inception",
            "release_date": "2010-07-15",
            "popularity": 90.0
        });
        let with_year =
            candidate_from_json(&item, MediaKind::Movie, "Inception", Some(2010)).unwrap();
        let wrong_year =
            candidate_from_json(&item, MediaKind::Movie, "Inception", Some(1999)).unwrap();
        assert!(with_year.score > wrong_year.score);
    }

    #[test]
    fn a_dated_match_beats_an_undated_namesake_by_a_clear_margin() {
        let dated = serde_json::json!({
            "id": 1, "name": "Silo", "original_name": "Silo",
            "first_air_date": "2023-05-04", "popularity": 300.0
        });
        let undated = serde_json::json!({
            "id": 2, "name": "Silo", "original_name": "Silo", "popularity": 0.5
        });
        let a = candidate_from_json(&dated, MediaKind::Tv, "Silo", Some(2023)).unwrap();
        let b = candidate_from_json(&undated, MediaKind::Tv, "Silo", Some(2023)).unwrap();
        assert!(a.score - b.score >= 0.06, "{} vs {}", a.score, b.score);
    }

    #[test]
    fn splits_an_aka_title_into_variants() {
        let variants =
            title_variants("When Life Gives You Tangerines A.K.A Pokssak Sogatsuda");
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[1], "When Life Gives You Tangerines");
        assert_eq!(variants[2], "Pokssak Sogatsuda");

        assert_eq!(title_variants("Inception").len(), 1);
    }
}
