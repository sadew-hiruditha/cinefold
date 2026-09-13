//! User settings, persisted as JSON next to the undo log.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How the poster is framed inside the generated folder icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IconStyle {
    /// Poster on a tinted rounded square.
    #[default]
    Card,
    /// A folder silhouette in the tint colour with the poster set into it.
    Folder,
    /// The poster alone, with an optional tinted border.
    Poster,
}

/// Serialised in camelCase because these go straight to the React layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Root of the organised library. Movies and TV Shows are created inside.
    pub library_root: String,
    pub movie_template: String,
    pub tv_template: String,
    /// TMDB v3 API key. The user supplies their own - we never ship one.
    pub tmdb_api_key: String,
    /// SubDL API key, for fetching missing subtitles. Optional.
    pub subdl_api_key: String,
    /// After a run, fetch subtitles for titles that arrived without one.
    pub fetch_subtitles: bool,
    pub min_file_size_mb: u64,
    pub video_extensions: Vec<String>,
    pub subtitle_extensions: Vec<String>,
    /// Below this title-match confidence an item is sent to review instead of
    /// being accepted automatically.
    pub match_threshold: f64,
    /// Language codes to keep, best first. Empty means keep everything.
    pub preferred_languages: Vec<String>,
    pub set_folder_icons: bool,
    /// Accent used for film folder icons, as #rrggbb.
    pub folder_tint: String,
    /// Accent for series folders when `separate_tv_tint` is on.
    pub tv_tint: String,
    pub separate_tv_tint: bool,
    pub icon_style: IconStyle,
    /// Border around the poster as a percentage of the icon size (0-20).
    pub icon_border: u8,
    /// Corner rounding as a percentage of the icon size (0-30).
    pub icon_corner: u8,
    /// Crop the poster to fill the whole shape instead of fitting it 2:3
    /// with gaps at the sides.
    pub icon_fill: bool,
    /// Copy rather than move, leaving the source untouched.
    pub copy_instead_of_move: bool,
    /// Hash-verify cross-volume copies before deleting the source.
    pub deep_verify: bool,
    pub keep_undo_runs: usize,
    pub skip_samples: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library_root: String::new(),
            movie_template: "Movies/{title} ({year})/{title} ({year})".to_string(),
            tv_template:
                "TV Shows/{show}/Season {season:02}/{show} - S{season:02}E{episode:02} - {episode_title}"
                    .to_string(),
            tmdb_api_key: String::new(),
            subdl_api_key: String::new(),
            fetch_subtitles: true,
            min_file_size_mb: 50,
            video_extensions: ["mkv", "mp4", "avi", "m4v", "mov", "wmv", "mpg", "mpeg", "ts", "webm"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            subtitle_extensions: ["srt", "ass", "ssa", "sub", "idx", "vtt"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            match_threshold: 0.72,
            // Empty keeps every subtitle; dropping languages is opt-in.
            preferred_languages: Vec::new(),
            set_folder_icons: true,
            folder_tint: "#6366f1".to_string(),
            tv_tint: "#10b981".to_string(),
            separate_tv_tint: false,
            icon_style: IconStyle::Card,
            icon_border: 8,
            icon_corner: 12,
            icon_fill: false,
            copy_instead_of_move: false,
            deep_verify: true,
            keep_undo_runs: 20,
            skip_samples: true,
        }
    }
}

impl Settings {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.json")
    }

    /// Load settings, falling back to defaults if the file is missing or
    /// unreadable. A corrupt file should not stop the app from starting.
    pub fn load(config_dir: &Path) -> Self {
        let path = Self::path(config_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                eprintln!("settings.json could not be parsed ({err}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, config_dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::path(config_dir), text)?;
        Ok(())
    }

    pub fn has_api_key(&self) -> bool {
        !self.tmdb_api_key.trim().is_empty()
    }

    pub fn has_subdl_key(&self) -> bool {
        !self.subdl_api_key.trim().is_empty()
    }

    pub fn min_bytes(&self) -> u64 {
        self.min_file_size_mb.saturating_mul(1024 * 1024)
    }

    pub fn is_video_ext(&self, ext: &str) -> bool {
        let ext = ext.to_lowercase();
        self.video_extensions.iter().any(|e| e.to_lowercase() == ext)
    }

    pub fn is_subtitle_ext(&self, ext: &str) -> bool {
        let ext = ext.to_lowercase();
        self.subtitle_extensions
            .iter()
            .any(|e| e.to_lowercase() == ext)
    }
}
