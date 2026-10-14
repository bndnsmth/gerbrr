//! One name, one promise, and one properly equipped little gerbil.
use eframe::egui::IconData;
use std::{io::Cursor, sync::OnceLock};

pub const NAME: &str = "Gerbrr";
pub const WORDMARK: &str = "GERBRR";
pub const TAGLINE: &str = "Gerber viewing at ridiculous speed.";
pub const REPOSITORY_URL: &str = "https://github.com/bndnsmth/gerbrr";

pub fn icon() -> IconData {
    static ICON: OnceLock<IconData> = OnceLock::new();
    ICON.get_or_init(|| decode_icon(include_bytes!("../assets/gerbrr.png")))
        .clone()
}

pub fn github_mark() -> IconData {
    static ICON: OnceLock<IconData> = OnceLock::new();
    ICON.get_or_init(|| decode_icon(include_bytes!("../assets/github.png")))
        .clone()
}

fn decode_icon(bytes: &[u8]) -> IconData {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("Bundled icon is a valid PNG");
    let mut data = vec![
        0;
        reader
            .output_buffer_size()
            .expect("Icon has a bounded image size")
    ];
    let info = reader
        .next_frame(&mut data)
        .expect("Bundled icon can be decoded");
    data.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => data,
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => panic!("Icon must use RGB or RGBA colors"),
    };
    IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mascot_is_a_valid_rgba_app_icon() {
        let icon = icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(icon.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        assert_eq!(NAME, "Gerbrr");
        assert_eq!(TAGLINE, "Gerber viewing at ridiculous speed.");
    }

    #[test]
    fn github_mark_is_a_bundled_transparent_icon() {
        let icon = github_mark();
        assert_eq!((icon.width, icon.height), (64, 64));
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        assert!(icon.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
        assert!(icon.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
    }
}
