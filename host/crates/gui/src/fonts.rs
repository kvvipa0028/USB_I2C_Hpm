//! Built-in Chinese UI font.
//!
//! `assets/fonts/NotoSansSC-UI.otf` is Noto Sans CJK SC Regular (SIL OFL 1.1,
//! `assets/fonts/OFL.txt`) cut down to the characters in `src/` by
//! `assets/fonts/subset.sh`, so the window's Chinese looks the same on every
//! machine without shipping the full 16 MB font. It comes after egui's own
//! fonts, so Latin text and digits keep egui's look. Characters outside the
//! subset (an OS error message in Chinese, pasted text) still go to eframe's
//! system font fallback, which stays on.
//!
//! After adding Chinese text to the UI, rerun `assets/fonts/subset.sh`;
//! the test below fails until the subset covers it.

use eframe::egui;
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

const NAME: &str = "noto-sans-sc-ui";
static FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-UI.otf");

fn insert() -> FontInsert {
    let families = [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
        .into_iter()
        .map(|family| InsertFontFamily {
            family,
            priority: FontPriority::Lowest,
        })
        .collect();
    FontInsert::new(NAME, egui::FontData::from_static(FONT), families)
}

pub fn install(ctx: &egui::Context) {
    ctx.add_font(insert());
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use egui::epaint::text::{Fonts, TextOptions};

    use super::*;

    fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path.display().to_string(), text));
            }
        }
    }

    #[test]
    fn subset_covers_ui_text() {
        let ins = insert();
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert(ins.name.clone(), Arc::new(ins.data));
        for f in ins.families {
            defs.families
                .entry(f.family)
                .or_default()
                .push(ins.name.clone());
        }
        let mut fonts = Fonts::new(TextOptions::default(), defs);
        let font = egui::FontId::proportional(13.0);
        // the check below means something only if a character can be missing
        assert!(fonts.has_glyph(&font, '连'));
        // a rare Han character, written as a code point so the scan below skips it
        assert!(!fonts.has_glyph(&font, '\u{9f98}'));

        let mut files = Vec::new();
        sources(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut missing = String::new();
        for (file, text) in &files {
            for c in text.chars().filter(|c| !c.is_ascii()) {
                if !fonts.has_glyph(&font, c) && !missing.contains(c) {
                    missing.push_str(&format!("{c} (U+{:04X}, {file})\n", c as u32));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "not in the font subset; rerun assets/fonts/subset.sh:\n{missing}"
        );
    }
}
