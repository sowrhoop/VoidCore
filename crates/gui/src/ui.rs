//! Reusable VoidCore UI widgets.

use crate::theme::{self, Palette};
use eframe::egui;

pub fn header_bar(ui: &mut egui::Ui, p: &Palette, on_update: &mut bool) {
    egui::Frame::none()
        .fill(p.bg_panel)
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .inner_margin(egui::Margin::symmetric(20.0, 14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.horizontal(|ui| {
                    let logo_rect = ui
                        .allocate_response(egui::vec2(36.0, 36.0), egui::Sense::hover())
                        .rect;
                    if ui.is_rect_visible(logo_rect) {
                        ui.painter().rect_filled(
                            logo_rect,
                            egui::Rounding::same(8.0),
                            p.accent_soft,
                        );
                        ui.painter().text(
                            logo_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "VC",
                            egui::FontId::proportional(13.0),
                            p.text_primary,
                        );
                    }

                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("VoidCore")
                                .strong()
                                .size(20.0)
                                .color(p.text_primary),
                        );
                        ui.label(
                            egui::RichText::new("Command Center")
                                .size(12.0)
                                .color(p.text_muted),
                        );
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, p, "Check for Updates", false).clicked() {
                        *on_update = true;
                    }
                });
            });
        });
}

pub fn nav_item(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    icon: &str,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        p.accent_glow
    } else {
        egui::Color32::TRANSPARENT
    };
    let stroke = if selected {
        egui::Stroke::new(1.0, p.accent)
    } else {
        egui::Stroke::NONE
    };
    let text_color = if selected {
        p.text_primary
    } else {
        p.text_secondary
    };

    let desired = egui::vec2(ui.available_width(), 40.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, egui::Rounding::same(8.0), fill);
        if selected {
            ui.painter().rect_stroke(rect, egui::Rounding::same(8.0), stroke);
            let accent_bar = egui::Rect::from_min_size(
                rect.left_top(),
                egui::vec2(3.0, rect.height()),
            );
            ui.painter().rect_filled(accent_bar, egui::Rounding::same(2.0), p.accent);
        }
        let text = format!("{icon}  {label}");
        ui.painter().text(
            rect.left_center() + egui::vec2(14.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(14.0),
            text_color,
        );
    }

    response
}

pub fn primary_button(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    large: bool,
) -> egui::Response {
    let padding = if large {
        egui::vec2(24.0, 12.0)
    } else {
        egui::vec2(16.0, 8.0)
    };
    let font_size = if large { 15.0 } else { 13.0 };

    let text = egui::RichText::new(label)
        .strong()
        .size(font_size)
        .color(p.text_primary);

    ui.add(
        egui::Button::new(text)
            .fill(p.accent_soft)
            .stroke(egui::Stroke::new(1.0, p.accent))
            .rounding(egui::Rounding::same(8.0))
            .min_size(padding),
    )
}

pub fn secondary_button(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(13.0)
                .color(p.text_secondary),
        )
        .fill(p.bg_card)
        .stroke(egui::Stroke::new(1.0, p.border))
        .rounding(egui::Rounding::same(6.0)),
    )
}

pub fn status_pill(ui: &mut egui::Ui, p: &Palette, label: &str, running: bool) {
    let (dot_color, bg_color, text_color) = if running {
        (p.success, p.success_dim.gamma_multiply(0.35), p.success)
    } else {
        (p.danger, p.danger_dim.gamma_multiply(0.35), p.danger)
    };

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 28.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, egui::Rounding::same(14.0), bg_color);
            let dot_center = rect.left_center() + egui::vec2(14.0, 0.0);
            ui.painter()
                .circle_filled(dot_center, 5.0, dot_color);
            ui.painter().text(
                rect.left_center() + egui::vec2(28.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(13.0),
                text_color,
            );
        }
    });
}

pub fn stat_row(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str, value_color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(14.0)
                .color(p.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .strong()
                    .size(14.0)
                    .color(value_color),
            );
        });
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);
}

pub fn policy_bullet(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("✓")
                .strong()
                .color(p.success)
                .size(13.0),
        );
        ui.label(theme::body_text(text));
    });
    ui.add_space(4.0);
}

pub fn empty_state(ui: &mut egui::Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(
            egui::RichText::new(title)
                .strong()
                .size(16.0)
                .color(p.text_secondary),
        );
        ui.add_space(8.0);
        ui.label(theme::muted_text(subtitle));
    });
}

pub fn drop_hint_frame(ui: &mut egui::Ui, p: &Palette, has_path: bool) {
    let stroke_color = if has_path { p.accent } else { p.border };
    let fill = if has_path {
        p.accent_glow
    } else {
        p.bg_card
    };

    egui::Frame::none()
        .fill(fill)
        .stroke(egui::Stroke::new(
            1.5,
            stroke_color,
        ))
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(if has_path { "📎" } else { "⬇" })
                        .size(18.0),
                );
                ui.label(if has_path {
                    theme::body_text("Path ready — launch when verified")
                } else {
                    theme::muted_text("Drag an .exe onto this window to fill the path")
                });
            });
        });
}

pub fn toast_banner(ui: &mut egui::Ui, p: &Palette, message: &str, success: bool) {
    let (fill, stroke, icon) = if success {
        (
            p.success_dim.gamma_multiply(0.4),
            p.success,
            "✓",
        )
    } else {
        (
            p.danger_dim.gamma_multiply(0.4),
            p.danger,
            "!",
        )
    };

    egui::Frame::none()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(icon)
                        .strong()
                        .color(stroke),
                );
                ui.label(
                    egui::RichText::new(message)
                        .size(13.0)
                        .color(p.text_primary),
                );
            });
        });
}
