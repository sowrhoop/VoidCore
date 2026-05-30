//! VoidCore design tokens and egui theme setup.

use eframe::egui;

pub struct Palette {
    pub bg_app: egui::Color32,
    pub bg_panel: egui::Color32,
    pub bg_sidebar: egui::Color32,
    pub bg_card: egui::Color32,
    pub bg_card_hover: egui::Color32,
    pub border: egui::Color32,
    pub border_subtle: egui::Color32,
    pub accent: egui::Color32,
    pub accent_soft: egui::Color32,
    pub accent_glow: egui::Color32,
    pub success: egui::Color32,
    pub success_dim: egui::Color32,
    pub danger: egui::Color32,
    pub danger_dim: egui::Color32,
    pub warning: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub text_muted: egui::Color32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg_app: egui::Color32::from_rgb(11, 13, 16),
            bg_panel: egui::Color32::from_rgb(16, 18, 23),
            bg_sidebar: egui::Color32::from_rgb(14, 16, 21),
            bg_card: egui::Color32::from_rgb(22, 25, 32),
            bg_card_hover: egui::Color32::from_rgb(28, 32, 42),
            border: egui::Color32::from_rgb(42, 48, 58),
            border_subtle: egui::Color32::from_rgb(32, 36, 44),
            accent: egui::Color32::from_rgb(0, 180, 255),
            accent_soft: egui::Color32::from_rgb(0, 120, 200),
            accent_glow: egui::Color32::from_rgba_premultiplied(0, 140, 220, 40),
            success: egui::Color32::from_rgb(61, 220, 151),
            success_dim: egui::Color32::from_rgb(30, 100, 72),
            danger: egui::Color32::from_rgb(255, 107, 107),
            danger_dim: egui::Color32::from_rgb(180, 70, 70),
            warning: egui::Color32::from_rgb(255, 184, 77),
            text_primary: egui::Color32::from_rgb(240, 242, 245),
            text_secondary: egui::Color32::from_rgb(180, 186, 198),
            text_muted: egui::Color32::from_rgb(120, 128, 142),
        }
    }
}

pub fn apply_theme(ctx: &egui::Context) {
    let p = Palette::default();

    let mut visuals = egui::Visuals::dark();
    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.menu_rounding = egui::Rounding::same(8.0);
    visuals.panel_fill = p.bg_app;
    visuals.window_fill = p.bg_app;
    visuals.extreme_bg_color = p.bg_sidebar;
    visuals.faint_bg_color = p.bg_card;
    visuals.widgets.noninteractive.bg_fill = p.bg_card;
    visuals.widgets.noninteractive.fg_stroke.color = p.text_muted;
    visuals.widgets.inactive.bg_fill = p.bg_card;
    visuals.widgets.inactive.fg_stroke.color = p.text_secondary;
    visuals.widgets.hovered.bg_fill = p.bg_card_hover;
    visuals.widgets.hovered.fg_stroke.color = p.text_primary;
    visuals.widgets.active.bg_fill = p.accent_soft;
    visuals.widgets.active.fg_stroke.color = p.text_primary;
    visuals.widgets.open.bg_fill = p.accent_soft;
    visuals.selection.bg_fill = p.accent_glow;
    visuals.selection.stroke.color = p.accent;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.danger;
    visuals.override_text_color = Some(p.text_primary);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.indent = 20.0;
    style.visuals = visuals;
    ctx.set_style(style);
}

pub fn card_frame(p: &Palette) -> egui::Frame {
    egui::Frame::none()
        .fill(p.bg_card)
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(20.0, 18.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 4.0),
            blur: 12.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(60),
        })
}

pub fn page_title(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .strong()
        .size(26.0)
        .color(Palette::default().text_primary)
}

pub fn section_title(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .strong()
        .size(15.0)
        .color(Palette::default().accent)
}

pub fn body_text(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(14.0)
        .color(Palette::default().text_secondary)
        .line_height(Some(22.0))
}

pub fn muted_text(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(13.0)
        .color(Palette::default().text_muted)
}
