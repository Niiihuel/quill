//! Pasted images and the thumbnails of attached ones. Clipboard, disk and decoding run on
//! threads; the window only takes their results on a later frame.

use eframe::egui;
use limn::{RasterImage, RasterImageFormat};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where pasted images are saved: `attachments/` beside quill's database.
fn attachments_dir() -> PathBuf {
    quill_engine::Config::default_database()
        .database
        .and_then(|db| db.parent().map(|dir| dir.join("attachments")))
        .unwrap_or_else(|| std::env::temp_dir().join("quill-attachments"))
}

/// Reads an image from the clipboard, saves it as PNG and gives its path to `pasted`. Nothing
/// happens when the clipboard holds no image (text pastes go on as they do).
pub fn paste(pasted: Arc<Mutex<Vec<PathBuf>>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let Some(clip) = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_image())
            .ok()
        else {
            return;
        };
        let (Ok(width), Ok(height)) = (u32::try_from(clip.width), u32::try_from(clip.height))
        else {
            return;
        };
        let Some(rgba) = image::RgbaImage::from_raw(width, height, clip.bytes.into_owned()) else {
            return;
        };
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let dir = attachments_dir();
        let path = dir.join(format!("pasted-{millis}.png"));
        let saved = std::fs::create_dir_all(&dir).and_then(|()| {
            rgba.save_with_format(&path, image::ImageFormat::Png)
                .map_err(std::io::Error::other)
        });
        if let Err(error) = saved {
            eprintln!("quill: the pasted image was not saved: {error}");
            return;
        }
        pasted.lock().unwrap().push(path);
        ctx.request_repaint();
    });
}

/// The thumbnail of an image as limn takes it: 256 px at most, PNG. `None` when the file does
/// not decode, and the tile keeps its file icon.
pub fn thumbnail(path: &Path) -> Option<RasterImage> {
    use base64::Engine;
    use sha2::Digest;
    let decoded = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let mut png = Vec::new();
    decoded
        .thumbnail(256, 256)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    let digest: String = sha2::Sha256::digest(&png)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let data = base64::engine::general_purpose::STANDARD.encode(&png);
    let alt = path.file_name()?.to_string_lossy().into_owned();
    RasterImage::new(
        digest,
        RasterImageFormat::Png,
        u32::try_from(png.len()).ok()?,
        data.into(),
        alt,
    )
}
