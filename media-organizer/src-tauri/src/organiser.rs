//! Planning and execution.
//!
//! Planning is pure: it turns identified items into a list of proposed
//! operations and touches nothing. Execution walks that list, writing an undo
//! entry for each step as it completes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::fsutil;
use crate::parser::{MediaKind, ParsedName};
use crate::settings::Settings;
use crate::subtitles::SubtitleMatch;
use crate::tmdb::{Candidate, Identification};
use crate::undo::{RunJournal, UndoOp};

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    /// Identified with enough confidence to move automatically.
    Ready,
    /// Identified poorly or ambiguously - needs a human.
    NeedsReview,
    /// Explicitly excluded by the user.
    Excluded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub source: PathBuf,
    pub size: u64,
    pub parsed: ParsedName,
    pub identification: Identification,
    pub subtitles: Vec<SubtitleMatch>,
    pub status: ItemStatus,
}

impl MediaItem {
    pub fn chosen(&self) -> Option<&Candidate> {
        self.identification.best.as_ref()
    }

    /// The display title, falling back to whatever the parser produced.
    pub fn title(&self) -> String {
        self.chosen()
            .map(|c| c.title.clone())
            .unwrap_or_else(|| self.parsed.title.clone())
    }

    pub fn kind(&self) -> MediaKind {
        self.chosen().map(|c| c.kind).unwrap_or(self.parsed.kind)
    }
}

// ---------------------------------------------------------------------------
// Template rendering
// ---------------------------------------------------------------------------

static RE_PLACEHOLDER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{(\w+)(?::0?(\d+))?\}").expect("placeholder regex"));

static RE_EMPTY_PARENS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\s*\(\s*\)").expect("empty parens regex"));

#[derive(Debug, Clone, Default)]
pub struct TemplateVars {
    pub title: String,
    pub show: String,
    pub year: Option<u16>,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub extra_episodes: Vec<u16>,
    pub episode_title: Option<String>,
    pub edition: Option<String>,
    pub resolution: Option<String>,
}

fn pad(value: u16, width: usize) -> String {
    format!("{:0width$}", value, width = width)
}

/// A title containing a slash ("Face/Off") must not become two folders. The
/// remaining illegal characters are handled per component later on.
fn strip_separators(value: &str) -> String {
    value.replace(['/', '\\'], " ")
}

/// Substitute `{placeholders}` and tidy up whatever the gaps leave behind.
///
/// Unknown or empty values render as nothing, then empty parentheses and
/// dangling " - " separators are removed, so a missing year does not leave
/// "Movie ()" on disk.
pub fn render_template(template: &str, vars: &TemplateVars) -> String {
    let rendered = RE_PLACEHOLDER.replace_all(template, |caps: &regex::Captures| {
        let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let width: usize = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);

        match name {
            "title" => strip_separators(&vars.title),
            "show" => {
                if vars.show.is_empty() {
                    strip_separators(&vars.title)
                } else {
                    strip_separators(&vars.show)
                }
            }
            "year" => vars.year.map(|y| y.to_string()).unwrap_or_default(),
            "season" => vars
                .season
                .map(|s| pad(s, width.max(1)))
                .unwrap_or_default(),
            "episode" => match vars.episode {
                Some(first) => {
                    let mut out = pad(first, width.max(1));
                    // Multi-episode files use the S01E01-E02 convention.
                    for extra in &vars.extra_episodes {
                        out.push_str(&format!("-E{}", pad(*extra, width.max(1))));
                    }
                    out
                }
                None => String::new(),
            },
            "episode_title" => vars
                .episode_title
                .as_deref()
                .map(strip_separators)
                .unwrap_or_default(),
            "edition" => vars
                .edition
                .as_deref()
                .map(strip_separators)
                .unwrap_or_default(),
            "resolution" => vars.resolution.clone().unwrap_or_default(),
            _ => String::new(),
        }
    });

    // An unfilled placeholder leaves "()" or a trailing " - " behind. Tidy each
    // path segment rather than the whole string, so a separator in the middle
    // of a name survives.
    RE_EMPTY_PARENS
        .replace_all(&rendered, "")
        .split('/')
        .map(|part| {
            part.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .trim_matches(|c: char| c == '-' || c.is_whitespace())
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Turn a rendered template into a real path under the library root.
fn destination_path(library_root: &Path, rendered: &str, extension: &str) -> PathBuf {
    let mut path = library_root.to_path_buf();
    let parts: Vec<&str> = rendered.split('/').collect();

    for (index, part) in parts.iter().enumerate() {
        let is_last = index + 1 == parts.len();
        let safe = fsutil::sanitize_component(part, fsutil::MAX_COMPONENT);
        if is_last {
            path.push(format!("{safe}.{extension}"));
        } else {
            path.push(safe);
        }
    }
    path
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpKind {
    Video,
    Subtitle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedOp {
    pub kind: OpKind,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    /// Set when the destination had to be altered, e.g. a name clash.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub item_id: String,
    pub title: String,
    pub kind: MediaKind,
    /// Folder that gets the poster, icon and desktop.ini.
    pub decorate_dir: PathBuf,
    pub destination_dir: PathBuf,
    pub ops: Vec<PlannedOp>,
    pub poster_url: Option<String>,
    pub warnings: Vec<String>,
    /// Unticked entries are carried through the plan but not executed.
    pub enabled: bool,
    /// Set when an identical title already appears earlier in the plan.
    pub duplicate_of: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub library_root: PathBuf,
    pub entries: Vec<PlanEntry>,
    pub warnings: Vec<String>,
    pub total_bytes: u64,
    pub file_count: usize,
    /// True when at least one file has to cross volumes (slow copy+verify).
    pub cross_volume: bool,
}

/// Reserve a destination path, stepping aside from anything already on disk
/// *or* already claimed earlier in this same plan.
///
/// Checking only the disk would let two sources in one run target an identical
/// path, and the second would silently land on a name the first had not
/// written yet.
fn claim_path(path: &Path, claimed: &mut HashSet<String>) -> (PathBuf, bool) {
    let mut candidate = path.to_path_buf();
    let mut moved_aside = false;

    for n in 2..1000u32 {
        let key = candidate.to_string_lossy().to_lowercase();
        if !candidate.exists() && !claimed.contains(&key) {
            claimed.insert(key);
            return (candidate, moved_aside);
        }
        moved_aside = true;
        candidate = fsutil::numbered(path, n);
    }

    claimed.insert(candidate.to_string_lossy().to_lowercase());
    (candidate, moved_aside)
}

fn resolution_rank(parsed: &ParsedName) -> u32 {
    parsed
        .resolution
        .as_deref()
        .and_then(|r| r.trim_end_matches(['p', 'P', 'i', 'I']).parse::<u32>().ok())
        .unwrap_or(0)
}

/// Template variables for an identified item.
fn template_vars(item: &MediaItem, candidate: &Candidate) -> TemplateVars {
    TemplateVars {
        title: candidate.title.clone(),
        show: candidate.title.clone(),
        year: candidate.year.or(item.parsed.year),
        season: item.parsed.season,
        episode: item.parsed.episode,
        extra_episodes: item.parsed.extra_episodes.clone(),
        episode_title: item
            .identification
            .episode_title
            .clone()
            .or_else(|| item.parsed.episode.map(|e| format!("Episode {e:02}"))),
        edition: item.parsed.edition.clone(),
        resolution: item.parsed.resolution.clone(),
    }
}

/// Where an identified item's video belongs, before any collision handling.
/// None when the item has no accepted match or no library is configured.
pub fn ideal_destination(item: &MediaItem, settings: &Settings) -> Option<PathBuf> {
    if settings.library_root.trim().is_empty() {
        return None;
    }
    let candidate = item.chosen()?;
    let vars = template_vars(item, candidate);
    let template = match item.kind() {
        MediaKind::Movie => &settings.movie_template,
        MediaKind::Tv => &settings.tv_template,
    };
    let rendered = render_template(template, &vars);
    let extension = item
        .source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "mkv".to_string());
    Some(destination_path(
        Path::new(settings.library_root.trim()),
        &rendered,
        &extension,
    ))
}

/// True when the file is already exactly where the current template would
/// put it - nothing to do, and certainly not a rename to " (2)".
pub fn already_organised(item: &MediaItem, settings: &Settings) -> bool {
    ideal_destination(item, settings)
        .map(|dest| fsutil::paths_equal(&dest, &item.source))
        .unwrap_or(false)
}

/// Build the list of proposed operations. Nothing is touched here.
pub fn build_plan(items: &[MediaItem], settings: &Settings, catalog: &Catalog) -> Plan {
    let library_root = PathBuf::from(&settings.library_root);
    let mut plan = Plan {
        library_root: library_root.clone(),
        ..Default::default()
    };

    if settings.library_root.trim().is_empty() {
        plan.warnings
            .push("No library folder is set - choose one in Settings.".to_string());
        return plan;
    }

    // Destinations claimed inside this plan, so two sources cannot collide on
    // one path before either has been written.
    let mut claimed: HashSet<String> = HashSet::new();
    // title|year|season|episode -> (entry index, resolution rank)
    let mut seen: HashMap<String, (usize, u32)> = HashMap::new();

    for item in items {
        if item.status != ItemStatus::Ready {
            continue;
        }
        let Some(candidate) = item.chosen() else {
            continue;
        };

        let kind = item.kind();
        let mut warnings: Vec<String> = Vec::new();
        let vars = template_vars(item, candidate);

        if kind == MediaKind::Tv && (vars.season.is_none() || vars.episode.is_none()) {
            warnings.push(
                "Season or episode number is missing, so the name will be incomplete.".to_string(),
            );
        }

        let Some(ideal_dest) = ideal_destination(item, settings) else {
            continue;
        };

        // Already in place: skip rather than step aside to " (2)".
        if fsutil::paths_equal(&ideal_dest, &item.source) {
            continue;
        }

        // Never overwrite: step aside from anything already on disk or already
        // claimed by an earlier entry in this plan.
        let (video_dest, moved_aside) = claim_path(&ideal_dest, &mut claimed);
        let note = moved_aside.then(|| {
            format!(
                "a file already exists there, saving as \"{}\"",
                video_dest
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            )
        });

        if !fsutil::within_max_path(&video_dest) {
            warnings.push(
                "The destination path is over 260 characters; some tools will not open it."
                    .to_string(),
            );
        }

        let destination_dir = video_dest
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| library_root.clone());

        // Movies get the icon on their own folder; a series gets it on the show
        // folder rather than each season.
        let decorate_dir = match kind {
            MediaKind::Movie => destination_dir.clone(),
            MediaKind::Tv => destination_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| destination_dir.clone()),
        };

        let mut ops = vec![PlannedOp {
            kind: OpKind::Video,
            source: item.source.clone(),
            destination: video_dest.clone(),
            size: item.size,
            note,
        }];

        // Subtitles sit beside the video and share its stem.
        let video_stem = video_dest
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        for sub in &item.subtitles {
            let sub_ext = sub
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_else(|| "srt".to_string());

            let mut suffix = String::new();
            if let Some(lang) = &sub.language {
                suffix.push('.');
                suffix.push_str(lang);
            }
            if sub.forced {
                suffix.push_str(".forced");
            }
            if sub.sdh {
                suffix.push_str(".sdh");
            }

            let ideal_sub = destination_dir.join(format!("{video_stem}{suffix}.{sub_ext}"));
            let (sub_dest, sub_moved) = claim_path(&ideal_sub, &mut claimed);
            let sub_note = sub_moved.then(|| "renamed to avoid an existing subtitle".to_string());

            ops.push(PlannedOp {
                kind: OpKind::Subtitle,
                source: sub.path.clone(),
                destination: sub_dest,
                size: 0,
                note: sub_note,
            });
        }

        if item.subtitles.is_empty() {
            warnings.push("No subtitle found next to this file.".to_string());
        }

        if !fsutil::same_volume(&item.source, &library_root) {
            plan.cross_volume = true;
        }

        plan.total_bytes += item.size;
        plan.file_count += ops.len();

        // Already in the collection from an earlier run *and still on disk*?
        // Films: any surviving copy counts. Series: only that exact episode.
        // An entry whose files are gone is history, not a duplicate.
        let mut enabled = true;
        if let Some(existing) = catalog.get(kind, candidate.id) {
            let owned = match kind {
                MediaKind::Movie => existing.film_on_disk(),
                MediaKind::Tv => match (vars.season, vars.episode) {
                    (Some(s), Some(e)) => existing.episode_on_disk(s, e),
                    _ => false,
                },
            };
            if owned {
                enabled = false;
                warnings.push(format!(
                    "Already in your collection at \"{}\" - unticked. Tick it to add this copy anyway.",
                    existing.folder.display()
                ));
            }
        }

        let entry = PlanEntry {
            item_id: item.id.clone(),
            title: item.title(),
            kind,
            decorate_dir,
            destination_dir,
            ops,
            poster_url: candidate.poster_url("w500"),
            warnings,
            enabled,
            duplicate_of: None,
        };

        // Duplicate detection: same episode or film twice, usually at two
        // different resolutions. Keep the better one enabled.
        let dup_key = format!(
            "{}|{:?}|{:?}|{:?}",
            candidate.id, vars.year, vars.season, vars.episode
        );
        let rank = resolution_rank(&item.parsed);
        let index = plan.entries.len();

        match seen.get(&dup_key).copied() {
            Some((previous_index, previous_rank)) => {
                let mut entry = entry;
                if rank > previous_rank {
                    // This one is better - demote the earlier entry instead.
                    let previous_title = plan.entries[previous_index].title.clone();
                    plan.entries[previous_index].enabled = false;
                    plan.entries[previous_index].duplicate_of = Some(item.id.clone());
                    plan.entries[previous_index]
                        .warnings
                        .push("A higher resolution copy of this is also in the queue.".to_string());
                    entry.warnings.push(format!(
                        "Duplicate of \"{previous_title}\" - this is the higher resolution copy."
                    ));
                    seen.insert(dup_key, (index, rank));
                } else {
                    entry.enabled = false;
                    entry.duplicate_of = Some(plan.entries[previous_index].item_id.clone());
                    entry
                        .warnings
                        .push("Already queued at the same or better quality.".to_string());
                }
                plan.entries.push(entry);
            }
            None => {
                seen.insert(dup_key, (index, rank));
                plan.entries.push(entry);
            }
        }
    }

    if plan.entries.is_empty() {
        plan.warnings
            .push("Nothing is ready to move. Resolve the review queue first.".to_string());
    }

    plan
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub label: String,
    pub item_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    pub moved: usize,
    pub copied: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
    pub decorated: usize,
    /// Icon problems are cosmetic and reported separately from move failures.
    pub decorate_failures: Vec<String>,
    /// Every video that landed, with its final path, for the collection.
    pub placed: Vec<PlacedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacedFile {
    pub item_id: String,
    pub destination: PathBuf,
    pub folder: PathBuf,
    /// A subtitle was part of this entry, so nothing needs fetching.
    pub has_subtitle: bool,
}

enum Transfer {
    Renamed,
    CopiedAndDeleted,
    CopiedOnly,
}

/// Move or copy one file, verifying before anything is removed.
fn transfer(
    source: &Path,
    destination: &Path,
    copy_only: bool,
    deep_verify: bool,
) -> anyhow::Result<Transfer> {
    let src = fsutil::long_path(source);
    let dst = fsutil::long_path(destination);

    if !copy_only {
        // Same-volume fast path: a rename is instant and atomic.
        if std::fs::rename(&src, &dst).is_ok() {
            return Ok(Transfer::Renamed);
        }
    }

    std::fs::copy(&src, &dst)?;

    // Verify before the source is touched. Size always; contents when asked.
    let source_size = fsutil::file_size(source)?;
    let dest_size = fsutil::file_size(destination)?;
    if source_size != dest_size {
        let _ = std::fs::remove_file(&dst);
        anyhow::bail!(
            "copy was truncated ({dest_size} of {source_size} bytes); destination removed"
        );
    }

    if deep_verify {
        let source_hash = fsutil::sha256_file(source)?;
        let dest_hash = fsutil::sha256_file(destination)?;
        if source_hash != dest_hash {
            let _ = std::fs::remove_file(&dst);
            anyhow::bail!("copy did not match the source checksum; destination removed");
        }
    }

    if copy_only {
        return Ok(Transfer::CopiedOnly);
    }

    std::fs::remove_file(&src)?;
    Ok(Transfer::CopiedAndDeleted)
}

/// Create a directory tree, journalling only the levels we actually created.
fn create_dirs(path: &Path, journal: &mut RunJournal) -> std::io::Result<()> {
    let mut missing: Vec<PathBuf> = Vec::new();
    let mut cursor = Some(path);
    while let Some(current) = cursor {
        if current.exists() {
            break;
        }
        missing.push(current.to_path_buf());
        cursor = current.parent();
    }

    fsutil::ensure_dir(path)?;

    for created in missing.into_iter().rev() {
        journal.record(UndoOp::CreatedDir { path: created });
    }
    Ok(())
}

/// Execute the file operations in a plan.
///
/// Each step is journalled as it completes, so an interrupted run is still
/// fully reversible. A failure on one file does not stop the rest.
///
/// `decorate` is called once per entry *before* its files move, with the
/// target folder already created. It returns Ok(true) when it wrote an icon,
/// Ok(false) when that folder was already handled this run.
pub fn execute(
    plan: &Plan,
    settings: &Settings,
    journal: &mut RunJournal,
    mut progress: impl FnMut(ProgressEvent),
    mut decorate: impl FnMut(&PlanEntry, &mut RunJournal) -> Result<bool, String>,
) -> ExecReport {
    let mut report = ExecReport::default();

    let total: usize = plan
        .entries
        .iter()
        .filter(|e| e.enabled)
        .map(|e| e.ops.len())
        .sum();
    let mut current = 0;

    for entry in plan.entries.iter().filter(|e| e.enabled) {
        // Icon first, so Explorer's first look at the folder already finds it.
        // (The old per-entry "ok" flag is gone: icons no longer depend on the
        // moves succeeding, since they are written beforehand.)
        if settings.set_folder_icons {
            match create_dirs(&entry.decorate_dir, journal)
                .map_err(|e| e.to_string())
                .and_then(|_| decorate(entry, journal))
            {
                Ok(true) => report.decorated += 1,
                Ok(false) => {}
                Err(err) => report
                    .decorate_failures
                    .push(format!("{}: {err}", entry.title)),
            }
        }

        for op in &entry.ops {
            current += 1;

            let label = op
                .destination
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            progress(ProgressEvent {
                phase: "move".to_string(),
                current,
                total,
                label: label.clone(),
                item_id: Some(entry.item_id.clone()),
                error: None,
            });

            if !op.source.exists() {
                report.skipped += 1;
                report
                    .failures
                    .push(format!("{} is no longer there", op.source.display()));
                continue;
            }

            if let Some(parent) = op.destination.parent() {
                if let Err(err) = create_dirs(parent, journal) {
                    report.skipped += 1;
                    report
                        .failures
                        .push(format!("could not create {}: {err}", parent.display()));
                    continue;
                }
            }

            // Re-check at the last moment: the plan may have been sitting on
            // screen while something else wrote to the library.
            let destination = if op.destination.exists() {
                fsutil::unique_destination(&op.destination)
            } else {
                op.destination.clone()
            };

            match transfer(
                &op.source,
                &destination,
                settings.copy_instead_of_move,
                settings.deep_verify && !fsutil::same_volume(&op.source, &destination),
            ) {
                Ok(Transfer::Renamed) | Ok(Transfer::CopiedAndDeleted) => {
                    report.moved += 1;
                    if op.kind == OpKind::Video {
                        report.placed.push(PlacedFile {
                            item_id: entry.item_id.clone(),
                            destination: destination.clone(),
                            folder: entry.decorate_dir.clone(),
                            has_subtitle: entry.ops.iter().any(|o| o.kind == OpKind::Subtitle),
                        });
                    }
                    journal.record(UndoOp::Moved {
                        from: op.source.clone(),
                        to: destination,
                    });
                }
                Ok(Transfer::CopiedOnly) => {
                    report.copied += 1;
                    if op.kind == OpKind::Video {
                        report.placed.push(PlacedFile {
                            item_id: entry.item_id.clone(),
                            destination: destination.clone(),
                            folder: entry.decorate_dir.clone(),
                            has_subtitle: entry.ops.iter().any(|o| o.kind == OpKind::Subtitle),
                        });
                    }
                    journal.record(UndoOp::Copied {
                        from: op.source.clone(),
                        to: destination,
                    });
                }
                Err(err) => {
                    journal.record_failure();
                    let message = format!("{}: {err}", op.source.display());
                    progress(ProgressEvent {
                        phase: "move".to_string(),
                        current,
                        total,
                        label,
                        item_id: Some(entry.item_id.clone()),
                        error: Some(message.clone()),
                    });
                    report.failures.push(message);
                }
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_vars() -> TemplateVars {
        TemplateVars {
            title: "Inception".into(),
            show: "Inception".into(),
            year: Some(2010),
            ..Default::default()
        }
    }

    #[test]
    fn renders_the_movie_template() {
        let out = render_template("Movies/{title} ({year})/{title} ({year})", &movie_vars());
        assert_eq!(out, "Movies/Inception (2010)/Inception (2010)");
    }

    #[test]
    fn drops_empty_parentheses_when_the_year_is_unknown() {
        let vars = TemplateVars {
            year: None,
            ..movie_vars()
        };
        let out = render_template("Movies/{title} ({year})/{title} ({year})", &vars);
        assert_eq!(out, "Movies/Inception/Inception");
    }

    #[test]
    fn pads_season_and_episode() {
        let vars = TemplateVars {
            title: "Breaking Bad".into(),
            show: "Breaking Bad".into(),
            season: Some(1),
            episode: Some(2),
            episode_title: Some("Cat's in the Bag...".into()),
            ..Default::default()
        };
        let out = render_template(
            "TV Shows/{show}/Season {season:02}/{show} - S{season:02}E{episode:02} - {episode_title}",
            &vars,
        );
        assert_eq!(
            out,
            "TV Shows/Breaking Bad/Season 01/Breaking Bad - S01E02 - Cat's in the Bag..."
        );
    }

    #[test]
    fn joins_multi_episode_files() {
        let vars = TemplateVars {
            show: "Firefly".into(),
            title: "Firefly".into(),
            season: Some(1),
            episode: Some(1),
            extra_episodes: vec![2],
            episode_title: Some("Serenity".into()),
            ..Default::default()
        };
        let out = render_template("{show} - S{season:02}E{episode:02} - {episode_title}", &vars);
        assert_eq!(out, "Firefly - S01E01-E02 - Serenity");
    }

    #[test]
    fn removes_a_dangling_separator_when_the_episode_title_is_missing() {
        let vars = TemplateVars {
            show: "Show".into(),
            title: "Show".into(),
            season: Some(2),
            episode: Some(3),
            episode_title: None,
            ..Default::default()
        };
        let out = render_template("{show} - S{season:02}E{episode:02} - {episode_title}", &vars);
        assert_eq!(out, "Show - S02E03");
    }

    #[test]
    fn sanitises_illegal_characters_in_the_destination() {
        let vars = TemplateVars {
            title: "Face/Off".into(),
            show: "Face/Off".into(),
            year: Some(1997),
            ..Default::default()
        };
        let rendered = render_template("Movies/{title} ({year})/{title} ({year})", &vars);
        let path = destination_path(Path::new(r"D:\Library"), &rendered, "mkv");
        assert!(path.to_string_lossy().contains("Face Off (1997)"));
        assert!(path.to_string_lossy().ends_with(".mkv"));
    }
}
