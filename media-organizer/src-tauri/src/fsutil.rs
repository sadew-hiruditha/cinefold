//! Filesystem primitives shared by the planner, executor and decorator.
//!
//! Everything that touches Windows-specific behaviour (long paths, file
//! attributes, reserved names) is funnelled through here so the rest of the
//! codebase can stay platform-agnostic.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// Conservative per-component cap. Windows allows 255, but long titles plus a
/// deep library root hit the 260 char total limit first.
pub const MAX_COMPONENT: usize = 90;

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Strip characters Windows forbids in a path component and trim it to length.
///
/// Illegal characters become spaces rather than being dropped, so a title like
/// "Face/Off" reads as "Face Off" instead of "FaceOff".
pub fn sanitize_component(input: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }

    let mut s: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string();

    if s.chars().count() > max_len {
        s = s.chars().take(max_len).collect::<String>();
        // Avoid cutting mid-word where we can: fall back to the last space.
        if let Some(idx) = s.rfind(' ') {
            if idx > max_len / 2 {
                s.truncate(idx);
            }
        }
        s = s.trim_end_matches(['.', ' ']).to_string();
    }

    if s.is_empty() {
        return "Untitled".to_string();
    }

    let stem = s.split('.').next().unwrap_or(&s).to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        s.push('_');
    }
    s
}

/// Prefix a path with the extended-length marker so it escapes the 260
/// character MAX_PATH limit.
///
/// Only applied to long, absolute, drive-letter paths - the extended prefix
/// disables path normalisation, so handing it a relative path would break it.
#[cfg(windows)]
pub fn long_path(path: &Path) -> PathBuf {
    use std::path::Prefix;

    let as_str = path.as_os_str().to_string_lossy();
    if as_str.len() < 240 || as_str.starts_with(r"\\?\") {
        return path.to_path_buf();
    }
    let is_drive_abs = matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_))
    );
    if !is_drive_abs {
        return path.to_path_buf();
    }
    PathBuf::from(format!(r"\\?\{}", as_str))
}

#[cfg(not(windows))]
pub fn long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Returns true when the path still fits in classic MAX_PATH. Paths beyond it
/// work for us but confuse other tools, so the preview flags them.
pub fn within_max_path(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().chars().count() < 260
}

/// The same path with " (n)" appended to the file stem.
pub fn numbered(path: &Path, n: u32) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    match path.extension().map(|s| s.to_string_lossy().to_string()) {
        Some(ext) => parent.join(format!("{stem} ({n}).{ext}")),
        None => parent.join(format!("{stem} ({n})")),
    }
}

/// Pick a destination that does not already exist by appending " (2)", " (3)".
///
/// We never overwrite, so this is the "append" half of the safety rule; the
/// caller decides between this and skipping.
pub fn unique_destination(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    for n in 2..1000 {
        let candidate = numbered(path, n);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// Whether two paths sit on the same volume, i.e. whether a rename can be a
/// metadata-only operation rather than a full copy.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    fn prefix(p: &Path) -> Option<String> {
        match p.components().next() {
            Some(Component::Prefix(pre)) => Some(pre.as_os_str().to_string_lossy().to_lowercase()),
            _ => None,
        }
    }
    match (prefix(a), prefix(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Case-insensitive, separator-agnostic path key for comparisons on Windows.
fn path_key(path: &Path) -> String {
    path.as_os_str()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Whether two paths name the same location, ignoring case and separators.
pub fn paths_equal(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// Whether `child` is `ancestor` itself or somewhere beneath it.
pub fn is_within(child: &Path, ancestor: &Path) -> bool {
    let child = path_key(child);
    let ancestor = path_key(ancestor);
    child == ancestor || child.starts_with(&format!("{ancestor}\\"))
}

pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(long_path(path))
}

pub fn file_size(path: &Path) -> std::io::Result<u64> {
    Ok(fs::metadata(long_path(path))?.len())
}

/// SHA-256 of a file, streamed in 1 MiB chunks so a 60 GB remux does not land
/// in memory.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(long_path(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Windows file attributes
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub fn get_attributes(path: &Path) -> std::io::Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetFileAttributesW;

    let wide: Vec<u16> = long_path(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let current = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if current == u32::MAX {
        return Err(std::io::Error::last_os_error());
    }
    Ok(current)
}

#[cfg(windows)]
pub fn set_attributes(path: &Path, attributes: u32) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::SetFileAttributesW;

    let wide: Vec<u16> = long_path(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe { SetFileAttributesW(wide.as_ptr(), attributes) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Add and/or clear attribute bits, returning the previous mask so an undo
/// entry can restore it exactly.
#[cfg(windows)]
pub fn modify_attributes(path: &Path, add: u32, remove: u32) -> std::io::Result<u32> {
    let previous = get_attributes(path)?;
    let next = (previous | add) & !remove;
    if next != previous {
        set_attributes(path, next)?;
    }
    Ok(previous)
}

#[cfg(not(windows))]
pub fn get_attributes(_path: &Path) -> std::io::Result<u32> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn set_attributes(_path: &Path, _attributes: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn modify_attributes(_path: &Path, _add: u32, _remove: u32) -> std::io::Result<u32> {
    Ok(0)
}

/// Ask the shell to re-read icons. Explorer caches aggressively, so a freshly
/// written desktop.ini often shows the stock folder until this fires.
#[cfg(windows)]
pub fn refresh_icon_cache() {
    use windows_sys::Win32::UI::Shell::SHChangeNotify;

    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    const SHCNF_IDLIST: u32 = 0x0000;

    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
    // Belt and braces: nudges the shell to rebuild its icon database too.
    let _ = std::process::Command::new("ie4uinit.exe")
        .arg("-show")
        .status();
}

#[cfg(not(windows))]
pub fn refresh_icon_cache() {}

/// Tell the shell that one specific folder's appearance changed.
///
/// The global refresh above is not enough on its own: Explorer caches each
/// folder's icon and only re-reads it when told that *that item* changed.
/// Without this, a folder Explorer happened to examine mid-write keeps showing
/// the fallback it computed at that instant.
#[cfg(windows)]
pub fn notify_folder_changed(folder: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::SHChangeNotify;

    const SHCNE_ATTRIBUTES: i32 = 0x0000_0800;
    const SHCNE_UPDATEITEM: i32 = 0x0000_2000;
    const SHCNE_UPDATEDIR: i32 = 0x0000_1000;
    const SHCNF_PATHW: u32 = 0x0005;
    const SHCNF_FLUSH: u32 = 0x1000;

    let wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };

    let item = wide(folder);
    // ATTRIBUTES is the event Explorer keys "re-read desktop.ini" on, since
    // the read-only bit is what turns customisation on; UPDATEITEM refreshes
    // the cached icon itself.
    for event in [SHCNE_ATTRIBUTES, SHCNE_UPDATEITEM] {
        unsafe {
            SHChangeNotify(
                event,
                SHCNF_PATHW | SHCNF_FLUSH,
                item.as_ptr() as *const _,
                std::ptr::null(),
            );
        }
    }
    if let Some(parent) = folder.parent() {
        let dir = wide(parent);
        unsafe {
            SHChangeNotify(
                SHCNE_UPDATEDIR,
                SHCNF_PATHW | SHCNF_FLUSH,
                dir.as_ptr() as *const _,
                std::ptr::null(),
            );
        }
    }
}

#[cfg(not(windows))]
pub fn notify_folder_changed(_folder: &Path) {}

/// Write a file by way of a temporary sibling and an atomic rename, so no
/// reader ever sees it half-written. The temp file is hidden while it exists,
/// but the finished file is always plain and visible - a same-volume rename
/// on Windows is metadata-only, so the temp file's Hidden bit would otherwise
/// ride along onto the destination. Callers that actually want a hidden
/// result (desktop.ini, folder.ico) set that explicitly afterward.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(long_path(&temp), bytes)?;
    let _ = modify_attributes(&temp, FILE_ATTRIBUTE_HIDDEN, 0);
    // Rename fails onto a hidden/system target on some Windows versions, so
    // strip the destination's attributes first if it exists.
    if path.exists() {
        let _ = set_attributes(path, 0x80);
    }
    fs::rename(long_path(&temp), long_path(path))?;
    let _ = set_attributes(path, 0x80);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitises_illegal_characters() {
        assert_eq!(sanitize_component("Face/Off", MAX_COMPONENT), "Face Off");
        assert_eq!(
            sanitize_component("  trailing.  ", MAX_COMPONENT),
            "trailing"
        );
    }

    #[test]
    fn avoids_reserved_device_names() {
        assert_eq!(sanitize_component("CON", MAX_COMPONENT), "CON_");
        assert_eq!(sanitize_component("Contact", MAX_COMPONENT), "Contact");
    }

    #[test]
    fn truncates_on_a_word_boundary() {
        let long = "The Assassination of Jesse James by the Coward Robert Ford";
        let cut = sanitize_component(long, 30);
        assert!(cut.chars().count() <= 30);
        assert!(!cut.ends_with(' '));
    }

    #[test]
    fn empty_input_gets_a_placeholder() {
        assert_eq!(sanitize_component("...", MAX_COMPONENT), "Untitled");
    }

    #[test]
    fn path_comparison_ignores_case_and_trailing_separators() {
        assert!(paths_equal(Path::new(r"G:\Media\Library"), Path::new(r"g:\media\library\")));
        assert!(!paths_equal(Path::new(r"G:\Media\Library"), Path::new(r"G:\Media\Library2")));
    }

    #[test]
    fn is_within_matches_self_and_descendants_only() {
        let root = Path::new(r"G:\Media");
        assert!(is_within(Path::new(r"G:\Media"), root));
        assert!(is_within(Path::new(r"g:\media\Movies\X"), root));
        assert!(!is_within(Path::new(r"G:\MediaOther"), root));
        assert!(!is_within(Path::new(r"G:\"), root));
    }

    #[test]
    fn atomic_write_replaces_a_hidden_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!(
            "cinefold-atomic-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("folder.ico");

        write_atomic(&target, b"first").unwrap();
        let _ = modify_attributes(&target, FILE_ATTRIBUTE_HIDDEN, 0);
        write_atomic(&target, b"second").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"second");
        assert!(!dir.join("folder.tmp").exists());
        // The temp file used to hide the destination while writing; the
        // finished file must not inherit that on a same-volume rename.
        assert_eq!(get_attributes(&target).unwrap() & FILE_ATTRIBUTE_HIDDEN, 0);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomic_never_leaves_a_hidden_result() {
        // Regression test: a rename on the same volume carries file
        // attributes across, so hiding the temp file used to leave every
        // freshly written file (fetched subtitles, posters) invisible in
        // Explorer unless "show hidden items" was on.
        let dir = std::env::temp_dir().join(format!(
            "cinefold-atomic-visible-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Movie (2020).en.srt");

        write_atomic(&target, b"1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();

        assert_eq!(get_attributes(&target).unwrap() & FILE_ATTRIBUTE_HIDDEN, 0);

        fs::remove_dir_all(&dir).ok();
    }
}
