//! The collection: everything Cinefold has organised, with the user's own
//! watched / rating / notes on top.
//!
//! Entries are keyed by TMDB id so a film seen twice (two resolutions, a
//! re-rip) stays one entry. Entries never disappear because a folder moved -
//! the page shows an "not on disk" badge instead - so viewing history is safe
//! from reorganising.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::parser::MediaKind;
use crate::tmdb::{Candidate, SeasonInfo, TitleDetails};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFile {
    pub path: PathBuf,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub episode_title: Option<String>,
    pub resolution: Option<String>,
    pub size: u64,
}

impl CatalogFile {
    /// "S01E02" for episodes, None for films.
    pub fn episode_key(&self) -> Option<String> {
        Some(format!("S{:02}E{:02}", self.season?, self.episode?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    /// "movie-27205" / "tv-1396".
    pub key: String,
    pub tmdb_id: u32,
    pub kind: MediaKind,
    pub title: String,
    pub original_title: String,
    pub year: Option<u16>,
    pub overview: String,
    pub tagline: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub genres: Vec<String>,
    pub runtime_minutes: Option<u32>,
    pub vote_average: f64,
    pub imdb_id: Option<String>,
    pub status: Option<String>,
    /// Season -> episode count from TMDB, for "you have 7 of 10".
    pub seasons: Vec<SeasonInfo>,
    /// Film folder, or the show folder for a series.
    pub folder: PathBuf,
    pub files: Vec<CatalogFile>,
    pub added_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // --- the user's own data ---------------------------------------------
    pub watched: bool,
    pub watched_at: Option<DateTime<Utc>>,
    /// 1-10, matching TMDB's scale.
    pub rating: Option<u8>,
    #[serde(default)]
    pub note: String,
    /// "S01E02" keys the user has ticked off.
    #[serde(default)]
    pub episodes_watched: Vec<String>,
}

impl CatalogEntry {
    pub fn key_for(kind: MediaKind, tmdb_id: u32) -> String {
        match kind {
            MediaKind::Movie => format!("movie-{tmdb_id}"),
            MediaKind::Tv => format!("tv-{tmdb_id}"),
        }
    }

    #[cfg(test)]
    pub fn has_episode(&self, season: u16, episode: u16) -> bool {
        self.files
            .iter()
            .any(|f| f.season == Some(season) && f.episode == Some(episode))
    }

    /// Whether at least one recorded copy of the film is still on disk. A
    /// remembered entry whose files are gone must not block re-organising.
    pub fn film_on_disk(&self) -> bool {
        self.files.iter().any(|f| f.path.is_file())
    }

    /// Whether that specific episode is still on disk.
    pub fn episode_on_disk(&self, season: u16, episode: u16) -> bool {
        self.files
            .iter()
            .any(|f| f.season == Some(season) && f.episode == Some(episode) && f.path.is_file())
    }

    /// Episode count on disk vs. what TMDB says exists.
    pub fn episode_progress(&self) -> Option<(usize, usize)> {
        if self.kind != MediaKind::Tv {
            return None;
        }
        let have = self
            .files
            .iter()
            .filter(|f| f.episode_key().is_some())
            .count();
        let total: usize = self.seasons.iter().map(|s| s.episodes as usize).sum();
        Some((have, total))
    }
}

/// What the page renders: the entry plus live facts we do not persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub on_disk: bool,
    pub episodes_have: usize,
    pub episodes_total: usize,
}

/// Distinguish "field absent" (None) from "field is null" (Some(None)), which
/// serde folds together for a plain Option<Option<T>>.
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// Fields the user can change from the page. Absent means "leave as is".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPatch {
    pub watched: Option<bool>,
    /// Some(None) clears the rating.
    #[serde(default, deserialize_with = "double_option")]
    pub rating: Option<Option<u8>>,
    pub note: Option<String>,
    pub episodes_watched: Option<Vec<String>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub entries: HashMap<String, CatalogEntry>,
}

impl Catalog {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("catalog.json")
    }

    pub fn load(config_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(config_dir))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = serde_json::to_string_pretty(self)?;
        let path = Self::path(config_dir);
        crate::fsutil::write_atomic(&path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn get(&self, kind: MediaKind, tmdb_id: u32) -> Option<&CatalogEntry> {
        self.entries.get(&CatalogEntry::key_for(kind, tmdb_id))
    }

    /// Add or merge an organised title. Files are merged by path, so a second
    /// resolution of the same film becomes a second file on one entry, and
    /// the user's watched/rating data is never touched.
    pub fn upsert(
        &mut self,
        candidate: &Candidate,
        details: Option<&TitleDetails>,
        folder: PathBuf,
        files: Vec<CatalogFile>,
    ) -> &CatalogEntry {
        let key = CatalogEntry::key_for(candidate.kind, candidate.id);
        let now = Utc::now();

        let entry = self.entries.entry(key.clone()).or_insert_with(|| CatalogEntry {
            key: key.clone(),
            tmdb_id: candidate.id,
            kind: candidate.kind,
            title: candidate.title.clone(),
            original_title: candidate.original_title.clone(),
            year: candidate.year,
            overview: candidate.overview.clone(),
            tagline: None,
            poster_path: candidate.poster_path.clone(),
            backdrop_path: candidate.backdrop_path.clone(),
            genres: Vec::new(),
            runtime_minutes: None,
            vote_average: candidate.vote_average,
            imdb_id: None,
            status: None,
            seasons: Vec::new(),
            folder: folder.clone(),
            files: Vec::new(),
            added_at: now,
            updated_at: now,
            watched: false,
            watched_at: None,
            rating: None,
            note: String::new(),
            episodes_watched: Vec::new(),
        });

        // Metadata refreshes; user data does not.
        entry.title = candidate.title.clone();
        entry.year = candidate.year.or(entry.year);
        if !candidate.overview.is_empty() {
            entry.overview = candidate.overview.clone();
        }
        if candidate.poster_path.is_some() {
            entry.poster_path = candidate.poster_path.clone();
        }
        if candidate.backdrop_path.is_some() {
            entry.backdrop_path = candidate.backdrop_path.clone();
        }
        entry.vote_average = candidate.vote_average;
        entry.folder = folder;
        entry.updated_at = now;

        if let Some(details) = details {
            entry.genres = details.genres.clone();
            entry.runtime_minutes = details.runtime_minutes.or(entry.runtime_minutes);
            entry.imdb_id = details.imdb_id.clone().or(entry.imdb_id.take());
            entry.status = details.status.clone().or(entry.status.take());
            entry.tagline = details.tagline.clone().or(entry.tagline.take());
            if !details.seasons.is_empty() {
                entry.seasons = details.seasons.clone();
            }
        }

        for file in files {
            match entry
                .files
                .iter_mut()
                .find(|f| crate::fsutil::paths_equal(&f.path, &file.path))
            {
                Some(existing) => *existing = file,
                None => entry.files.push(file),
            }
        }
        entry.files.sort_by(|a, b| {
            (a.season, a.episode, a.path.clone()).cmp(&(b.season, b.episode, b.path.clone()))
        });

        entry
    }

    pub fn apply(&mut self, key: &str, patch: CatalogPatch) -> Option<&CatalogEntry> {
        let entry = self.entries.get_mut(key)?;
        if let Some(watched) = patch.watched {
            entry.watched = watched;
            entry.watched_at = watched.then(Utc::now);
        }
        if let Some(rating) = patch.rating {
            entry.rating = rating.filter(|r| (1..=10).contains(r));
        }
        if let Some(note) = patch.note {
            entry.note = note;
        }
        if let Some(list) = patch.episodes_watched {
            entry.episodes_watched = list;
            entry.episodes_watched.sort();
            entry.episodes_watched.dedup();
        }
        entry.updated_at = Utc::now();
        Some(entry)
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Everything, newest first, with the on-disk check done here so the
    /// page never has to touch the filesystem.
    pub fn views(&self) -> Vec<CatalogView> {
        let mut out: Vec<CatalogView> = self
            .entries
            .values()
            .map(|entry| {
                let (have, total) = entry.episode_progress().unwrap_or((0, 0));
                CatalogView {
                    on_disk: entry.folder.is_dir(),
                    episodes_have: have,
                    episodes_total: total,
                    entry: entry.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| b.entry.added_at.cmp(&a.entry.added_at));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: MediaKind, id: u32, title: &str) -> Candidate {
        Candidate {
            id,
            title: title.into(),
            original_title: title.into(),
            year: Some(2010),
            overview: "A dream within a dream.".into(),
            poster_path: Some("/p.jpg".into()),
            backdrop_path: None,
            popularity: 10.0,
            vote_average: 8.4,
            kind,
            score: 1.0,
        }
    }

    fn file(path: &str, season: Option<u16>, episode: Option<u16>) -> CatalogFile {
        CatalogFile {
            path: PathBuf::from(path),
            season,
            episode,
            episode_title: None,
            resolution: Some("1080P".into()),
            size: 1,
        }
    }

    #[test]
    fn upsert_merges_files_and_keeps_user_data() {
        let mut catalog = Catalog::default();
        let c = candidate(MediaKind::Movie, 27205, "Inception");
        catalog.upsert(&c, None, PathBuf::from(r"L:\M\Inception"), vec![file(r"L:\M\Inception\a.mkv", None, None)]);
        catalog
            .apply(
                "movie-27205",
                CatalogPatch {
                    watched: Some(true),
                    rating: Some(Some(9)),
                    ..Default::default()
                },
            )
            .unwrap();

        // Second rip of the same film.
        catalog.upsert(&c, None, PathBuf::from(r"L:\M\Inception"), vec![file(r"L:\M\Inception\b.mkv", None, None)]);

        let entry = catalog.get(MediaKind::Movie, 27205).unwrap();
        assert_eq!(entry.files.len(), 2);
        assert!(entry.watched);
        assert_eq!(entry.rating, Some(9));
        assert_eq!(catalog.entries.len(), 1);
    }

    #[test]
    fn tv_progress_counts_episodes_against_tmdb() {
        let mut catalog = Catalog::default();
        let c = candidate(MediaKind::Tv, 1396, "Breaking Bad");
        let details = TitleDetails {
            seasons: vec![SeasonInfo { season: 1, episodes: 7 }],
            ..Default::default()
        };
        catalog.upsert(
            &c,
            Some(&details),
            PathBuf::from(r"L:\T\Breaking Bad"),
            vec![
                file(r"L:\T\Breaking Bad\S01\e1.mkv", Some(1), Some(1)),
                file(r"L:\T\Breaking Bad\S01\e2.mkv", Some(1), Some(2)),
            ],
        );
        let entry = catalog.get(MediaKind::Tv, 1396).unwrap();
        assert_eq!(entry.episode_progress(), Some((2, 7)));
        assert!(entry.has_episode(1, 2));
        assert!(!entry.has_episode(1, 3));
    }

    #[test]
    fn a_null_rating_in_json_clears_it() {
        let patch: CatalogPatch = serde_json::from_str(r#"{"rating": null}"#).unwrap();
        assert_eq!(patch.rating, Some(None));
        let absent: CatalogPatch = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(absent.rating, None);
    }

    #[test]
    fn a_remembered_film_whose_file_is_gone_does_not_count_as_owned() {
        let dir = std::env::temp_dir().join(format!(
            "cinefold-catalog-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("present.mkv");
        std::fs::write(&present, b"x").unwrap();

        let mut catalog = Catalog::default();
        let c = candidate(MediaKind::Movie, 5, "Gone");
        catalog.upsert(&c, None, dir.clone(), vec![file(dir.join("missing.mkv").to_str().unwrap(), None, None)]);
        assert!(!catalog.get(MediaKind::Movie, 5).unwrap().film_on_disk());

        catalog.upsert(&c, None, dir.clone(), vec![file(present.to_str().unwrap(), None, None)]);
        assert!(catalog.get(MediaKind::Movie, 5).unwrap().film_on_disk());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rating_outside_range_is_dropped() {
        let mut catalog = Catalog::default();
        let c = candidate(MediaKind::Movie, 1, "X");
        catalog.upsert(&c, None, PathBuf::from("x"), vec![]);
        catalog
            .apply(
                "movie-1",
                CatalogPatch {
                    rating: Some(Some(11)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(catalog.get(MediaKind::Movie, 1).unwrap().rating, None);
    }
}
