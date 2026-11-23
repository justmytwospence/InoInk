//! Text presets: named sets of the Aa panel's text choices (font, size, line spacing and
//! alignment; margins stay as they are). The idea comes from Kobalt's text presets
//! (https://github.com/nicoverbruggen/kobalt, MIT).

use serde::{Deserialize, Serialize};
use crate::metadata::TextAlign;
use super::{DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE, DEFAULT_LINE_HEIGHT, DEFAULT_TEXT_ALIGN};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TextPreset {
    pub name: String,
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub text_align: TextAlign,
}

impl TextPreset {
    pub fn new(name: &str, font_family: &str, font_size: f32, line_height: f32, text_align: TextAlign) -> TextPreset {
        TextPreset {
            name: name.to_string(),
            font_family: font_family.to_string(),
            font_size,
            line_height,
            text_align,
        }
    }

    /// The same text, whatever the names (sizes compare to the panel's steps).
    pub fn same_text(&self, other: &TextPreset) -> bool {
        self.font_family == other.font_family &&
        (self.font_size - other.font_size).abs() < 0.05 &&
        (self.line_height - other.line_height).abs() < 0.05 &&
        self.text_align == other.text_align
    }
}

/// The presets InoInk ships, built from the default family and the bundled reading fonts
/// (`fonts/reading`). Default is the out-of-the-box text.
pub fn bundled_text_presets() -> Vec<TextPreset> {
    vec![
        TextPreset::new("Default", DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE, DEFAULT_LINE_HEIGHT, DEFAULT_TEXT_ALIGN),
        TextPreset::new("Bookish", "NV Literata", 10.5, 1.4, TextAlign::Justify),
        TextPreset::new("Modern", "NV Jost", 10.5, 1.4, TextAlign::Left),
        TextPreset::new("Sturdy", "NV Bitter", 10.0, 1.5, TextAlign::Left),
    ]
}

/// The preset in force and whether the text has changed since it was applied: the chosen one
/// (by name) if it still exists, else the first that matches the text.
pub fn preset_in_force(presets: &[TextPreset], chosen: Option<&str>, current: &TextPreset) -> Option<(usize, bool)> {
    if let Some(index) = chosen.and_then(|name| find_preset(presets, name)) {
        return Some((index, !presets[index].same_text(current)));
    }
    presets.iter().position(|p| p.same_text(current)).map(|index| (index, false))
}

/// The index of the preset of that name (names are unique without regard to case).
pub fn find_preset(presets: &[TextPreset], name: &str) -> Option<usize> {
    let name = name.trim().to_lowercase();
    presets.iter().position(|p| p.name.to_lowercase() == name)
}

/// A free name for a new preset of this text, like "NV Literata 10.5".
pub fn suggest_preset_name(presets: &[TextPreset], current: &TextPreset) -> String {
    let size = if current.font_size.fract().abs() < 0.05 {
        format!("{:.0}", current.font_size)
    } else {
        format!("{:.1}", current.font_size)
    };
    let base = format!("{} {}", current.font_family, size);
    if find_preset(presets, &base).is_none() {
        return base;
    }
    (2..).map(|n| format!("{base} ({n})"))
         .find(|name| find_preset(presets, name).is_none())
         .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(family: &str, size: f32, line_height: f32, align: TextAlign) -> TextPreset {
        TextPreset::new("", family, size, line_height, align)
    }

    #[test]
    fn the_default_preset_is_the_default_text() {
        let presets = bundled_text_presets();
        let default = text(DEFAULT_FONT_FAMILY, DEFAULT_FONT_SIZE, DEFAULT_LINE_HEIGHT, DEFAULT_TEXT_ALIGN);
        assert_eq!(preset_in_force(&presets, None, &default), Some((0, false)));
    }

    #[test]
    fn bundled_presets_use_bundled_fonts() {
        for p in bundled_text_presets() {
            assert!(p.font_family == DEFAULT_FONT_FAMILY || crate::font::BUNDLED_FONT_FAMILIES.contains(&p.font_family.as_str()), "{}", p.name);
        }
    }

    #[test]
    fn a_change_marks_the_chosen_preset_modified() {
        let presets = bundled_text_presets();
        let mut t = presets[1].clone();
        assert_eq!(preset_in_force(&presets, Some("bookish"), &t), Some((1, false)));
        t.font_size += 0.5;
        assert_eq!(preset_in_force(&presets, Some("Bookish"), &t), Some((1, true)));
        // Without a choice, a text that matches no preset has none in force.
        assert_eq!(preset_in_force(&presets, None, &t), None);
        // A deleted choice falls back to a matching preset.
        assert_eq!(preset_in_force(&presets, Some("Gone"), &presets[2]), Some((2, false)));
    }

    #[test]
    fn suggested_names_are_free() {
        let mut presets = bundled_text_presets();
        let t = text("NV Jost", 11.0, 1.2, TextAlign::Left);
        assert_eq!(suggest_preset_name(&presets, &t), "NV Jost 11");
        presets.push(TextPreset { name: "nv jost 11".into(), ..t.clone() });
        assert_eq!(suggest_preset_name(&presets, &t), "NV Jost 11 (2)");
        let half = text("Libron", 11.5, 1.2, TextAlign::Left);
        assert_eq!(suggest_preset_name(&presets, &half), "Libron 11.5");
    }

    #[test]
    fn presets_round_trip_through_settings() {
        let mut settings = crate::settings::Settings::default();
        settings.reader.text_preset = Some("Modern".into());
        let s = toml::to_string(&settings).unwrap();
        let back: crate::settings::Settings = toml::from_str(&s).unwrap();
        assert_eq!(back.reader.text_presets, bundled_text_presets());
        assert_eq!(back.reader.text_preset.as_deref(), Some("Modern"));
        // Settings saved before presets existed get the bundled ones.
        let old: crate::settings::Settings = toml::from_str("[reader]\nfont-size = 12.0\n").unwrap();
        assert_eq!(old.reader.text_presets, bundled_text_presets());
        assert_eq!(old.reader.text_preset, None);
    }
}
