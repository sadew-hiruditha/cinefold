//! Operation log and rollback.
//!
//! Every run writes a journal to disk as it goes, entry by entry, so a crash
//! mid-run still leaves a complete record of what had already happened. Undo
//! replays that journal backwards.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fsutil;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UndoOp {
    /// Source was moved; undo puts it back.
    Moved { from: PathBuf, to: PathBuf },
    /// Source was copied and left in place; undo deletes the copy.
    Copied { from: PathBuf, to: PathBuf },
    /// Directory we created; undo removes it if it is still empty.
    CreatedDir { path: PathBuf },
    /// File we generated (poster, icon, desktop.ini); undo deletes it.
    WroteFile { path: PathBuf },
    /// Attribute change; undo restores the previous mask.
    SetAttributes { path: PathBuf, previous: u32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub files_moved: usize,
    pub files_copied: usize,
    pub folders_created: usize,
    pub files_written: usize,
    pub failures: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRun {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub source_root: PathBuf,
    pub library_root: PathBuf,
    pub summary: RunSummary,
    pub entries: Vec<UndoOp>,
    /// Set once the run has been rolled back, so the UI can grey it out.
    pub undone_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackReport {
    pub restored: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
}

/// A run that is currently being written to.
pub struct RunJournal {
    run: UndoRun,
    path: PathBuf,
}

impl RunJournal {
    pub fn record(&mut self, op: UndoOp) {
        match &op {
            UndoOp::Moved { .. } => self.run.summary.files_moved += 1,
            UndoOp::Copied { .. } => self.run.summary.files_copied += 1,
            UndoOp::CreatedDir { .. } => self.run.summary.folders_created += 1,
            UndoOp::WroteFile { .. } => self.run.summary.files_written += 1,
            UndoOp::SetAttributes { .. } => {}
        }
        self.run.entries.push(op);
        // Flush immediately: a journal that lags behind reality is worse than
        // no journal at all.
        if let Err(err) = self.flush() {
            eprintln!("could not write the undo journal: {err}");
        }
    }

    pub fn record_failure(&mut self) {
        self.run.summary.failures += 1;
    }

    pub fn finish(mut self) -> Result<UndoRun> {
        self.run.finished_at = Some(Utc::now());
        self.flush()?;
        Ok(self.run)
    }

    fn flush(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.run)?;
        std::fs::write(&self.path, text)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

/// The directory holding one JSON file per run.
pub struct UndoStore {
    dir: PathBuf,
}

impl UndoStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn run_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn begin(&self, source_root: &Path, library_root: &Path) -> RunJournal {
        let id = Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let run = UndoRun {
            id: id.clone(),
            started_at: Utc::now(),
            finished_at: None,
            source_root: source_root.to_path_buf(),
            library_root: library_root.to_path_buf(),
            summary: RunSummary::default(),
            entries: Vec::new(),
            undone_at: None,
        };
        RunJournal {
            path: self.run_path(&id),
            run,
        }
    }

    pub fn load(&self, id: &str) -> Result<UndoRun> {
        let text = std::fs::read_to_string(self.run_path(id))
            .with_context(|| format!("no undo record for run {id}"))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save(&self, run: &UndoRun) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.run_path(&run.id), serde_json::to_string_pretty(run)?)?;
        Ok(())
    }

    /// Newest first.
    pub fn list(&self) -> Vec<UndoRun> {
        let mut runs: Vec<UndoRun> = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|text| serde_json::from_str::<UndoRun>(&text).ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        runs
    }

    /// Delete all but the newest `keep` runs.
    pub fn prune(&self, keep: usize) {
        for run in self.list().into_iter().skip(keep) {
            let _ = std::fs::remove_file(self.run_path(&run.id));
        }
    }

    /// Replay a run backwards.
    ///
    /// Entries are independent: one failure is recorded and the rest still run,
    /// because a partial restore beats stopping halfway.
    pub fn rollback(&self, id: &str) -> Result<RollbackReport> {
        let mut run = self.load(id)?;
        let mut report = RollbackReport {
            restored: 0,
            skipped: 0,
            failures: Vec::new(),
        };

        for op in run.entries.iter().rev() {
            match op {
                UndoOp::Moved { from, to } => {
                    if !to.exists() {
                        report.skipped += 1;
                        continue;
                    }
                    if from.exists() {
                        report.failures.push(format!(
                            "{} already exists again, left {} in place",
                            from.display(),
                            to.display()
                        ));
                        continue;
                    }
                    if let Some(parent) = from.parent() {
                        let _ = fsutil::ensure_dir(parent);
                    }
                    match move_back(to, from) {
                        Ok(()) => report.restored += 1,
                        Err(err) => report
                            .failures
                            .push(format!("{} -> {}: {err}", to.display(), from.display())),
                    }
                }
                UndoOp::Copied { to, .. } | UndoOp::WroteFile { path: to } => {
                    if !to.exists() {
                        report.skipped += 1;
                        continue;
                    }
                    // Generated files may be hidden/system; clear that first.
                    let _ = fsutil::set_attributes(to, 0x80 /* FILE_ATTRIBUTE_NORMAL */);
                    match std::fs::remove_file(fsutil::long_path(to)) {
                        Ok(()) => report.restored += 1,
                        Err(err) => report.failures.push(format!("{}: {err}", to.display())),
                    }
                }
                UndoOp::CreatedDir { path } => {
                    if !path.exists() {
                        report.skipped += 1;
                        continue;
                    }
                    // Only remove what we emptied out; anything left is the
                    // user's and must not be touched.
                    match std::fs::remove_dir(fsutil::long_path(path)) {
                        Ok(()) => report.restored += 1,
                        Err(_) => report.skipped += 1,
                    }
                }
                UndoOp::SetAttributes { path, previous } => {
                    if !path.exists() {
                        report.skipped += 1;
                        continue;
                    }
                    match fsutil::set_attributes(path, *previous) {
                        Ok(()) => report.restored += 1,
                        Err(err) => report.failures.push(format!("{}: {err}", path.display())),
                    }
                }
            }
        }

        run.undone_at = Some(Utc::now());
        self.save(&run)?;
        Ok(report)
    }
}

/// Rename where possible, copy+delete when the move crossed a volume.
fn move_back(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(fsutil::long_path(from), fsutil::long_path(to)) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(fsutil::long_path(from), fsutil::long_path(to))?;
            std::fs::remove_file(fsutil::long_path(from))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cinefold-test-{label}-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rolls_a_move_back() {
        let root = temp_dir("undo-move");
        let source = root.join("source.mkv");
        let dest_dir = root.join("Library/Movie (2020)");
        let dest = dest_dir.join("Movie (2020).mkv");
        std::fs::write(&source, b"data").unwrap();
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::rename(&source, &dest).unwrap();

        let store = UndoStore::new(root.join("undo"));
        let mut journal = store.begin(&root, &root.join("Library"));
        journal.record(UndoOp::CreatedDir {
            path: dest_dir.clone(),
        });
        journal.record(UndoOp::Moved {
            from: source.clone(),
            to: dest.clone(),
        });
        let run = journal.finish().unwrap();

        let report = store.rollback(&run.id).unwrap();
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(source.exists());
        assert!(!dest.exists());
        assert!(!dest_dir.exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn keeps_a_directory_that_still_has_files() {
        let root = temp_dir("undo-keep");
        let dir = root.join("Library/Keep");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("users-own-file.txt"), b"x").unwrap();

        let store = UndoStore::new(root.join("undo"));
        let mut journal = store.begin(&root, &root);
        journal.record(UndoOp::CreatedDir { path: dir.clone() });
        let run = journal.finish().unwrap();

        let report = store.rollback(&run.id).unwrap();
        assert_eq!(report.skipped, 1);
        assert!(dir.exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lists_runs_newest_first() {
        let root = temp_dir("undo-list");
        let store = UndoStore::new(root.join("undo"));

        let mut first = store.begin(&root, &root);
        first.record(UndoOp::WroteFile {
            path: root.join("a"),
        });
        let first = first.finish().unwrap();

        let mut second = store.begin(&root, &root);
        second.record(UndoOp::WroteFile {
            path: root.join("b"),
        });
        let mut second = second.finish().unwrap();
        // Runs are stamped to the second; force an ordering for the test.
        second.started_at = second.started_at + chrono::Duration::seconds(5);
        second.id = format!("{}-b", second.id);
        store.save(&second).unwrap();

        let runs = store.list();
        assert_eq!(runs.first().map(|r| r.id.clone()), Some(second.id));
        assert!(runs.iter().any(|r| r.id == first.id));

        std::fs::remove_dir_all(&root).ok();
    }
}
