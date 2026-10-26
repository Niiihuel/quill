//! What quill's apps share so they look the same: the tokens, the icons and the faces.

pub mod icons;
pub mod look;
pub mod pieces;
pub mod settings;

/// The system's sans and monospace faces, as Desktop's Code surface draws in: it sets
/// `--font-anthropic-sans` to the system fallbacks, which fontconfig resolves (DejaVu on
/// NixOS). `None` keeps limn's bundled faces when fontconfig cannot say.
pub fn system_fonts() -> Option<limn::Fonts> {
    let face = |pattern: &str| -> Option<limn::Font> {
        let out = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", pattern])
            .output()
            .ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        limn::Font::new(std::fs::read(path.trim()).ok()?).ok()
    };
    use limn::FontFamily::*;
    let mut fonts = limn::Fonts::bundled();
    for (family, pattern) in [
        (Sans, "sans-serif"),
        (SansMedium, "sans-serif:bold"),
        (SansBold, "sans-serif:bold"),
        (Monospace, "monospace"),
        (MonospaceBold, "monospace:bold"),
    ] {
        fonts = fonts.face(family, face(pattern)?);
    }
    Some(fonts)
}
