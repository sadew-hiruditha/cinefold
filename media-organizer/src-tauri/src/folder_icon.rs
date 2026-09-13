//! Turn a TMDB poster into a Windows folder icon.
//!
//! Windows only reads `desktop.ini` when the folder itself carries the
//! read-only (or system) attribute, and only picks up the icon once the shell
//! is told to refresh - both of which are handled here. Every file written and
//! every attribute changed is journalled so the whole thing can be undone.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};

use crate::fsutil;
use crate::parser::MediaKind;
use crate::settings::{IconStyle, Settings};
use crate::undo::{RunJournal, UndoOp};

/// Sizes Explorer asks for, from the tree view up to extra-large icons.
const ICON_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];

const DESKTOP_INI: &str = "desktop.ini";
const ICON_FILE: &str = "folder.ico";
const POSTER_FILE: &str = "poster.jpg";

fn parse_hex_colour(input: &str) -> [u8; 3] {
    let hex = input.trim().trim_start_matches('#');
    if hex.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        ) {
            return [r, g, b];
        }
    }
    [0x63, 0x66, 0xf1]
}

/// Signed distance to a rounded rectangle; negative inside.
fn rounded_rect_distance(
    px: f32,
    py: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> f32 {
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0;
    let half_w = (right - left) / 2.0;
    let half_h = (bottom - top) / 2.0;

    let qx = (px - cx).abs() - (half_w - radius);
    let qy = (py - cy).abs() - (half_h - radius);
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - radius
}

/// Coverage of a pixel by the shape, antialiased over one pixel.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn blend_pixel(canvas: &mut RgbaImage, x: u32, y: u32, colour: [u8; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let existing = canvas.get_pixel(x, y).0;
    let inv = 1.0 - alpha;
    let mix = |src: u8, dst: u8| -> u8 {
        (src as f32 * alpha + dst as f32 * inv).round().clamp(0.0, 255.0) as u8
    };
    canvas.put_pixel(
        x,
        y,
        Rgba([
            mix(colour[0], existing[0]),
            mix(colour[1], existing[1]),
            mix(colour[2], existing[2]),
            mix(255, existing[3]),
        ]),
    );
}

/// Everything that shapes the generated icon, taken from Settings.
#[derive(Debug, Clone, Copy)]
pub struct IconOptions {
    pub style: IconStyle,
    pub tint: [u8; 3],
    /// Fraction of the icon size (0.0-0.2).
    pub border: f32,
    /// Fraction of the icon size (0.0-0.3).
    pub corner: f32,
    /// Crop to fill rather than fit.
    pub fill: bool,
}

impl IconOptions {
    /// Options for a film folder (the default colour).
    pub fn from_settings(settings: &Settings) -> Self {
        Self::for_kind(settings, MediaKind::Movie)
    }

    /// Options for a folder of the given kind; series can carry their own
    /// colour so the two shelves tell apart at a glance.
    pub fn for_kind(settings: &Settings, kind: MediaKind) -> Self {
        let tint = match kind {
            MediaKind::Tv if settings.separate_tv_tint => &settings.tv_tint,
            _ => &settings.folder_tint,
        };
        Self {
            style: settings.icon_style,
            tint: parse_hex_colour(tint),
            border: (settings.icon_border.min(20) as f32) / 100.0,
            corner: (settings.icon_corner.min(30) as f32) / 100.0,
            fill: settings.icon_fill,
        }
    }
}

fn scale_colour(colour: [u8; 3], factor: f32) -> [u8; 3] {
    let f = |c: u8| ((c as f32 * factor).round().clamp(0.0, 255.0)) as u8;
    [f(colour[0]), f(colour[1]), f(colour[2])]
}

/// Fill a rounded rectangle, optionally lifting the colour towards the top so
/// large flat areas keep a little depth.
fn fill_rounded_rect(
    canvas: &mut RgbaImage,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
    colour: [u8; 3],
    lift: f32,
) {
    let (w, h) = canvas.dimensions();
    let x0 = left.floor().max(0.0) as u32;
    let y0 = top.floor().max(0.0) as u32;
    let x1 = (right.ceil() as u32).min(w);
    let y1 = (bottom.ceil() as u32).min(h);
    let height = (bottom - top).max(1.0);

    for y in y0..y1 {
        let t = 1.0 - (y as f32 - top) / height;
        let shaded = scale_colour(colour, 1.0 + lift * t.clamp(0.0, 1.0));
        for x in x0..x1 {
            let distance = rounded_rect_distance(
                x as f32 + 0.5,
                y as f32 + 0.5,
                left,
                top,
                right,
                bottom,
                radius,
            );
            blend_pixel(canvas, x, y, shaded, coverage(distance));
        }
    }
}

/// Draw the poster into a rectangle with rounded corners.
fn blit_poster(
    canvas: &mut RgbaImage,
    poster: &DynamicImage,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
) {
    let (pw, ph) = (width.round().max(1.0) as u32, height.round().max(1.0) as u32);
    let resized = poster
        .resize_to_fill(pw, ph, FilterType::Lanczos3)
        .to_rgba8();
    let (cw, ch) = canvas.dimensions();
    let (ox, oy) = (left.round() as i64, top.round() as i64);

    for y in 0..ph {
        for x in 0..pw {
            let cx = ox + x as i64;
            let cy = oy + y as i64;
            if cx < 0 || cy < 0 || cx >= cw as i64 || cy >= ch as i64 {
                continue;
            }
            let distance = rounded_rect_distance(
                x as f32 + 0.5,
                y as f32 + 0.5,
                0.0,
                0.0,
                pw as f32,
                ph as f32,
                radius,
            );
            let pixel = resized.get_pixel(x, y).0;
            blend_pixel(
                canvas,
                cx as u32,
                cy as u32,
                [pixel[0], pixel[1], pixel[2]],
                coverage(distance) * (pixel[3] as f32 / 255.0),
            );
        }
    }
}

/// Compose one square icon frame in the chosen style.
///
/// Posters are 2:3, so every style has to decide what fills the rest of the
/// square; the tint doubles as the "folder colour" feature and keeps the
/// small sizes legible.
pub fn compose_icon(poster: Option<&DynamicImage>, size: u32, opts: &IconOptions) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let s = size as f32;
    let border = (s * opts.border).round();
    let corner = (s * opts.corner).max(0.0);

    match opts.style {
        IconStyle::Card => {
            fill_rounded_rect(&mut canvas, 0.0, 0.0, s, s, corner, opts.tint, 0.18);
            if let Some(poster) = poster {
                let h = (s - border * 2.0).max(1.0);
                let w = if opts.fill { h } else { (h * 2.0 / 3.0).min(h) };
                blit_poster(
                    &mut canvas,
                    poster,
                    (s - w) / 2.0,
                    (s - h) / 2.0,
                    w,
                    h,
                    (corner * 0.55).max(0.5),
                );
            }
        }

        IconStyle::Folder => {
            // Tab at the back-left, then the body over it. The body sits a
            // little lower than the tab so the tab reads as a separate flap.
            let tab_radius = (corner * 0.5).max(1.0);
            fill_rounded_rect(
                &mut canvas,
                s * 0.03,
                s * 0.06,
                s * 0.42,
                s * 0.34,
                tab_radius,
                scale_colour(opts.tint, 0.78),
                0.0,
            );
            fill_rounded_rect(
                &mut canvas,
                s * 0.03,
                s * 0.20,
                s * 0.97,
                s * 0.95,
                corner,
                opts.tint,
                0.16,
            );
            if let Some(poster) = poster {
                // Fit within the body, leaving the border all round.
                let body_top = s * 0.20;
                let body_h = s * 0.75;
                let h = (body_h - border * 2.0).max(1.0);
                let avail_w = (s * 0.94 - border * 2.0).max(1.0);
                let w = if opts.fill { avail_w } else { (h * 2.0 / 3.0).min(avail_w) };
                blit_poster(
                    &mut canvas,
                    poster,
                    (s - w) / 2.0,
                    body_top + (body_h - h) / 2.0,
                    w,
                    h,
                    (corner * 0.4).max(0.5),
                );
            }
        }

        IconStyle::Poster => {
            // Artwork only: a portrait rectangle with transparent sides. The
            // border, if any, becomes a tinted frame directly around it.
            let h = (s - border * 2.0).max(1.0);
            let w = if opts.fill { h } else { (h * 2.0 / 3.0).min(h) };
            let left = (s - w) / 2.0;
            let top = (s - h) / 2.0;
            if border >= 1.0 {
                fill_rounded_rect(
                    &mut canvas,
                    left - border,
                    top - border,
                    left + w + border,
                    top + h + border,
                    corner,
                    opts.tint,
                    0.12,
                );
            }
            match poster {
                Some(poster) => blit_poster(
                    &mut canvas,
                    poster,
                    left,
                    top,
                    w,
                    h,
                    (corner * 0.7).max(0.5),
                ),
                None => fill_rounded_rect(
                    &mut canvas,
                    left,
                    top,
                    left + w,
                    top + h,
                    (corner * 0.7).max(0.5),
                    scale_colour(opts.tint, 0.6),
                    0.2,
                ),
            }
        }
    }

    canvas
}

/// A stand-in poster for the settings preview when no real one is available:
/// a gradient with a lighter title band, so the frame styles still read.
pub fn placeholder_poster() -> DynamicImage {
    let (w, h) = (200u32, 300u32);
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f32 / h as f32;
            let u = x as f32 / w as f32;
            let r = (40.0 + 60.0 * t + 20.0 * u) as u8;
            let g = (44.0 + 30.0 * t) as u8;
            let b = (70.0 + 90.0 * (1.0 - t)) as u8;
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    fill_rounded_rect(
        &mut img,
        w as f32 * 0.15,
        h as f32 * 0.72,
        w as f32 * 0.85,
        h as f32 * 0.80,
        4.0,
        [230, 230, 240],
        0.0,
    );
    DynamicImage::ImageRgba8(img)
}

/// Render a single frame as a PNG data URL, for the live preview in Settings.
pub fn preview_png(poster: Option<&DynamicImage>, size: u32, opts: &IconOptions) -> Result<String> {
    use base64::Engine;
    use image::ImageEncoder;

    let frame = compose_icon(poster, size, opts);
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(
        frame.as_raw(),
        size,
        size,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Build a multi-resolution .ico from poster bytes.
pub fn build_ico(poster_bytes: Option<&[u8]>, opts: &IconOptions) -> Result<Vec<u8>> {
    let poster = match poster_bytes {
        Some(bytes) => Some(
            image::load_from_memory(bytes).context("the poster image could not be decoded")?,
        ),
        None => None,
    };

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICON_SIZES {
        let frame = compose_icon(poster.as_ref(), *size, opts);
        let entry = ico::IconImage::from_rgba_data(*size, *size, frame.into_raw());
        dir.add_entry(ico::IconDirEntry::encode(&entry)?);
    }

    let mut buffer = Cursor::new(Vec::new());
    dir.write(&mut buffer)?;
    Ok(buffer.into_inner())
}

fn desktop_ini_contents(title: &str) -> String {
    // CRLF and the trailing blank line are what Explorer expects.
    format!(
        "[.ShellClassInfo]\r\nIconResource={ICON_FILE},0\r\nIconFile={ICON_FILE}\r\nIconIndex=0\r\nInfoTip={title}\r\nConfirmFileOp=0\r\n"
    )
}

fn write_and_journal(path: &Path, bytes: &[u8], journal: &mut RunJournal) -> Result<()> {
    let existed = path.exists();
    fsutil::write_atomic(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    if !existed {
        journal.record(UndoOp::WroteFile {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Write the icon files into a folder and flip the attributes that make
/// Windows honour them. Synchronous: the poster bytes are fetched up front so
/// this can run inside the move loop, *before* files land in the folder -
/// Explorer then never gets a chance to cache the folder as plain.
///
/// Failures here are cosmetic - the library still gets organised - so the
/// caller records them as warnings rather than failing the run.
pub fn decorate(
    folder: &Path,
    title: &str,
    poster_bytes: Option<&[u8]>,
    opts: &IconOptions,
    journal: &mut RunJournal,
) -> Result<()> {
    if !folder.is_dir() {
        anyhow::bail!("{} does not exist", folder.display());
    }

    if let Some(bytes) = poster_bytes {
        write_and_journal(&folder.join(POSTER_FILE), bytes, journal)?;
    }

    let ico_bytes = build_ico(poster_bytes, opts)?;
    let icon_path = folder.join(ICON_FILE);
    write_and_journal(&icon_path, &ico_bytes, journal)?;

    let ini_path = folder.join(DESKTOP_INI);
    write_and_journal(&ini_path, desktop_ini_contents(title).as_bytes(), journal)?;

    // desktop.ini has to be hidden + system or Explorer ignores it.
    if let Ok(previous) = fsutil::modify_attributes(
        &ini_path,
        fsutil::FILE_ATTRIBUTE_HIDDEN | fsutil::FILE_ATTRIBUTE_SYSTEM,
        0,
    ) {
        journal.record(UndoOp::SetAttributes {
            path: ini_path.clone(),
            previous,
        });
    }

    // The icon itself is hidden so it does not clutter the folder.
    let _ = fsutil::modify_attributes(&icon_path, fsutil::FILE_ATTRIBUTE_HIDDEN, 0);

    // And the folder needs read-only set for desktop.ini to be read at all.
    if let Ok(previous) = fsutil::modify_attributes(folder, fsutil::FILE_ATTRIBUTE_READONLY, 0) {
        journal.record(UndoOp::SetAttributes {
            path: folder.to_path_buf(),
            previous,
        });
    }

    fsutil::notify_folder_changed(folder);
    Ok(())
}

/// Film or series? A show folder holds season folders; a film folder holds
/// the video directly. Used when re-applying, where no plan is around.
pub fn guess_kind(folder: &Path) -> MediaKind {
    let has_subdirs = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        })
        .unwrap_or(false);
    if has_subdirs {
        return MediaKind::Tv;
    }
    let parent = folder
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if parent.contains("tv") || parent.contains("series") || parent.contains("show") {
        MediaKind::Tv
    } else {
        MediaKind::Movie
    }
}

/// Whether this folder was decorated by us (has our icon or saved poster).
pub fn is_decorated(folder: &Path) -> bool {
    folder.join(POSTER_FILE).is_file() || folder.join(ICON_FILE).is_file()
}

/// Regenerate `folder.ico` for an already-decorated folder using the current
/// options, reusing the saved `poster.jpg`. No download, no journal: this
/// only rewrites a file we generated in the first place.
pub fn reapply(folder: &Path, opts: &IconOptions) -> Result<()> {
    let poster_path = folder.join(POSTER_FILE);
    let poster_bytes = if poster_path.is_file() {
        Some(std::fs::read(fsutil::long_path(&poster_path))?)
    } else {
        None
    };

    let ico_bytes = build_ico(poster_bytes.as_deref(), opts)?;
    let icon_path = folder.join(ICON_FILE);

    // Atomic swap: Explorer watches this folder and will re-read the icon the
    // moment it changes, so it must never observe a half-written file.
    fsutil::write_atomic(&icon_path, &ico_bytes)
        .with_context(|| format!("writing {}", icon_path.display()))?;
    let _ = fsutil::modify_attributes(&icon_path, fsutil::FILE_ATTRIBUTE_HIDDEN, 0);

    // Make sure the folder still has what Explorer needs, in case the user
    // cleaned it up by hand at some point.
    let ini_path = folder.join(DESKTOP_INI);
    if !ini_path.is_file() {
        let title = folder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        std::fs::write(
            fsutil::long_path(&ini_path),
            desktop_ini_contents(&title).as_bytes(),
        )?;
        let _ = fsutil::modify_attributes(
            &ini_path,
            fsutil::FILE_ATTRIBUTE_HIDDEN | fsutil::FILE_ATTRIBUTE_SYSTEM,
            0,
        );
    }
    let _ = fsutil::modify_attributes(folder, fsutil::FILE_ATTRIBUTE_READONLY, 0);

    fsutil::notify_folder_changed(folder);
    Ok(())
}

/// Every decorated folder under the library, shallow enough to cover
/// `Movies/Title` and `TV Shows/Show` without crawling into episodes.
pub fn decorated_folders(library_root: &Path) -> Vec<std::path::PathBuf> {
    walkdir::WalkDir::new(library_root)
        .min_depth(1)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .filter(|p| is_decorated(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_hex_colour() {
        assert_eq!(parse_hex_colour("#6366f1"), [0x63, 0x66, 0xf1]);
        assert_eq!(parse_hex_colour("ff0000"), [255, 0, 0]);
    }

    #[test]
    fn falls_back_on_a_bad_colour() {
        assert_eq!(parse_hex_colour("not a colour"), [0x63, 0x66, 0xf1]);
    }

    fn opts(style: IconStyle) -> IconOptions {
        IconOptions {
            style,
            tint: [255, 0, 0],
            border: 0.08,
            corner: 0.12,
            fill: false,
        }
    }

    #[test]
    fn fill_mode_covers_the_sides() {
        let poster = placeholder_poster();
        let filled = IconOptions {
            fill: true,
            border: 0.0,
            ..opts(IconStyle::Poster)
        };
        let icon = compose_icon(Some(&poster), 128, &filled);
        assert_eq!(icon.get_pixel(2, 64).0[3], 255);
        assert_eq!(icon.get_pixel(125, 64).0[3], 255);
    }

    #[test]
    fn card_fills_the_centre_and_rounds_the_corners() {
        let icon = compose_icon(None, 64, &opts(IconStyle::Card));
        assert_eq!(icon.get_pixel(32, 32).0[3], 255);
        assert_eq!(icon.get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn folder_style_leaves_the_top_right_transparent() {
        // The tab only spans the left; above the body on the right is empty.
        let icon = compose_icon(None, 128, &opts(IconStyle::Folder));
        assert_eq!(icon.get_pixel(120, 10).0[3], 0);
        assert!(icon.get_pixel(20, 20).0[3] > 0, "tab should be drawn");
        assert_eq!(icon.get_pixel(64, 80).0[3], 255, "body should be solid");
    }

    #[test]
    fn poster_style_keeps_the_sides_transparent() {
        let poster = placeholder_poster();
        let icon = compose_icon(Some(&poster), 128, &opts(IconStyle::Poster));
        // A 2:3 poster in a square leaves the far left and right empty.
        assert_eq!(icon.get_pixel(2, 64).0[3], 0);
        assert_eq!(icon.get_pixel(64, 64).0[3], 255);
    }

    #[test]
    fn builds_an_ico_with_every_size() {
        let bytes = build_ico(None, &opts(IconStyle::Card)).unwrap();
        let dir = ico::IconDir::read(Cursor::new(bytes)).unwrap();
        assert_eq!(dir.entries().len(), ICON_SIZES.len());
    }

    #[test]
    fn preview_is_a_png_data_url() {
        let poster = placeholder_poster();
        let url = preview_png(Some(&poster), 64, &opts(IconStyle::Folder)).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 100);
    }

    #[test]
    fn desktop_ini_points_at_the_icon() {
        let text = desktop_ini_contents("Inception (2010)");
        assert!(text.contains("IconResource=folder.ico,0"));
        assert!(text.contains("InfoTip=Inception (2010)"));
        assert!(text.ends_with("\r\n"));
    }
}
