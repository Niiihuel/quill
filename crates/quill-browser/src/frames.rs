//! The frames a page draws, decoded off the UI thread and left for the host to present: only
//! the newest frame of each surface is kept, so a slow window skips frames instead of
//! queueing them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// A page's pixels, RGB rows top to bottom, rendered for `scale` device pixels per CSS pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub scale: f32,
    pub rgb: Vec<u8>,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Shared between the browser, which publishes, and the host, which takes.
#[derive(Clone, Default)]
pub struct Frames {
    fresh: Arc<Mutex<HashMap<String, Frame>>>,
    wake: Arc<Mutex<Option<Wake>>>,
    /// The window's scale as f32 bits; Chromium is launched at it.
    scale: Arc<AtomicU32>,
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frames")
            .field("scale", &self.scale())
            .finish_non_exhaustive()
    }
}

impl Frames {
    /// Call `wake` (the window's repaint) whenever a frame is published.
    pub fn on_frame(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.lock().unwrap() = Some(Arc::new(wake));
    }

    /// The frames published since the last call, newest of each surface.
    pub fn take(&self) -> Vec<(String, Frame)> {
        self.fresh.lock().unwrap().drain().collect()
    }

    /// The host tells the window's device pixels per logical pixel, every frame.
    pub fn set_scale(&self, scale: f32) {
        self.scale.store(scale.to_bits(), Ordering::Relaxed);
    }

    pub fn scale(&self) -> f32 {
        match f32::from_bits(self.scale.load(Ordering::Relaxed)) {
            scale if scale.is_finite() && scale > 0.0 => scale,
            _ => 1.0,
        }
    }

    /// Wake the window without a frame: something else of the browser's is waiting for it.
    pub(crate) fn wake(&self) {
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    pub(crate) fn publish(&self, surface: String, frame: Frame) {
        self.fresh.lock().unwrap().insert(surface, frame);
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

/// A screencast frame's JPEG as RGB.
pub fn decode(jpeg: &[u8], scale: f32) -> Option<Frame> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let rgb = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (info.width as usize, info.height as usize);
    (rgb.len() == width * height * 3).then_some(Frame {
        width,
        height,
        scale,
        rgb,
    })
}
