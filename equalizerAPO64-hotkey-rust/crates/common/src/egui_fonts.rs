//! CJK font fallback for the egui-based applications.
//!
//! The C++ applications use Qt, which resolves Chinese/Japanese/Korean device
//! names through the system font. egui's built-in font has no CJK glyphs, so
//! instal the Windows UI font as a fallback when available.

pub fn install_cjk_font(ctx: &egui::Context) {
    const CANDIDATES: &[(&str, u32)] = &[
        (r"C:\Windows\Fonts\msyh.ttc", 0),
        (r"C:\Windows\Fonts\msyhbd.ttc", 0),
        (r"C:\Windows\Fonts\simhei.ttf", 0),
        (r"C:\Windows\Fonts\simsun.ttc", 0),
    ];

    for &(path, index) in CANDIDATES {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        let mut font_data = egui::FontData::from_owned(data);
        font_data.index = index;
        fonts
            .font_data
            .insert("eapo_system".to_owned(), std::sync::Arc::new(font_data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("eapo_system".to_owned());
            }
        }
        ctx.set_fonts(fonts);
        return;
    }
}
