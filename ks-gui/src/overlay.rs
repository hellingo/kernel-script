use egui::{Context, Id, LayerId, Order};

use crate::lua_runtime::DrawCommand;

/// Paints one frame of `draw.*` commands.
///
/// `origin` is the overlay window's top-left in the same coordinate space as
/// the commands (absolute logical screen units, which is what
/// `memory.get_window_rect` returns), so the painter subtracts it to get
/// window-local points.
///
/// Convert logical units to egui points by undoing only the menu zoom.
/// The renderer still applies native DPI once to each position and size.
pub fn paint_commands(ctx: &Context, commands: &[DrawCommand], origin: (f32, f32)) {
    if commands.is_empty() {
        return;
    }
    let (origin_x, origin_y) = origin;
    let scale = 1.0 / ctx.zoom_factor().max(1e-3);
    // Background layer: strictly below every egui window (settings panels,
    // the control window, the Lua error window), so ESP output never covers
    // the UI that produced it.
    let painter = ctx.layer_painter(LayerId::new(Order::Background, Id::new("draw_overlay")));
    for command in commands {
        match command {
            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                thickness,
            } => {
                painter.line_segment(
                    [
                        egui::pos2((*x1 - origin_x) * scale, (*y1 - origin_y) * scale),
                        egui::pos2((*x2 - origin_x) * scale, (*y2 - origin_y) * scale),
                    ],
                    egui::Stroke::new(*thickness * scale, color32(*color)),
                );
            }
            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                color,
                thickness,
            } => {
                painter.rect_stroke(
                    egui::Rect::from_min_size(
                        egui::pos2((*x - origin_x) * scale, (*y - origin_y) * scale),
                        egui::vec2(*w * scale, *h * scale),
                    ),
                    0.0,
                    egui::Stroke::new(*thickness * scale, color32(*color)),
                    egui::StrokeKind::Middle,
                );
            }
            DrawCommand::FilledRect { x, y, w, h, color } => {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2((*x - origin_x) * scale, (*y - origin_y) * scale),
                        egui::vec2(*w * scale, *h * scale),
                    ),
                    0.0,
                    color32(*color),
                );
            }
            DrawCommand::Circle {
                x,
                y,
                radius,
                color,
                thickness,
            } => {
                painter.circle_stroke(
                    egui::pos2((*x - origin_x) * scale, (*y - origin_y) * scale),
                    *radius * scale,
                    egui::Stroke::new(*thickness * scale, color32(*color)),
                );
            }
            DrawCommand::FilledCircle {
                x,
                y,
                radius,
                color,
            } => {
                painter.circle_filled(
                    egui::pos2((*x - origin_x) * scale, (*y - origin_y) * scale),
                    *radius * scale,
                    color32(*color),
                );
            }
            DrawCommand::Text {
                x,
                y,
                text,
                color,
                size,
            } => {
                let color = color32(*color);
                let galley = ctx.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        text.clone(),
                        egui::FontId::proportional(*size * scale),
                        color,
                    )
                });
                painter.galley(
                    egui::pos2((*x - origin_x) * scale, (*y - origin_y) * scale),
                    galley,
                    egui::Color32::TRANSPARENT,
                );
            }
        }
    }
}

/// Paint the frame-rate readout in a single line at a DPI-adjusted size.
pub fn paint_fps(ctx: &Context, fps: f32) {
    // The readout is part of the HUD. Undo the menu zoom while retaining native DPI.
    let scale = 1.0 / ctx.zoom_factor().max(1e-3);
    let mut font = egui::TextStyle::Monospace.resolve(&ctx.global_style());
    font.size *= scale;
    let color = egui::Color32::from_rgb(130, 255, 170);
    let galley =
        ctx.fonts_mut(|fonts| fonts.layout_no_wrap(format!("{fps:>6.1} FPS"), font, color));
    let padding = egui::vec2(7.0, 3.0) * scale;
    let anchor = ctx.content_rect().right_top() + egui::vec2(-8.0, 8.0) * scale;
    let rect = egui::Align2::RIGHT_TOP.anchor_size(anchor, galley.size() + padding * 2.0);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("fps_meter")));
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((4.0 * scale).round() as u8),
        egui::Color32::from_black_alpha(160),
    );
    painter.galley(rect.min + padding, galley, color);
}

fn color32(color: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_stays_on_one_line_at_a_fixed_size_when_menu_zoom_changes() {
        let ctx = Context::default();
        let native_dpi = 1.25;
        let mut original_size = egui::Vec2::ZERO;

        for zoom in [1.0, 1.25, 0.75, 1.0] {
            let scale = native_dpi * zoom;
            let screen_size = egui::vec2(2560.0, 1430.0) / scale;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_size)),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(native_dpi);
            ctx.options_mut(|options| options.zoom_factor = zoom);
            let mut output = ctx.run_ui(input, |ui| paint_fps(ui.ctx(), 99.9));
            output.textures_delta.clear();
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .expect("FPS text was drawn");
            assert_eq!(text.galley.rows.len(), 1);
            let size = text.galley.size() * output.pixels_per_point;
            assert!(size.x > 0.0 && size.y > 0.0);
            if original_size == egui::Vec2::ZERO {
                original_size = size;
            }
            assert!((size - original_size).length() <= 1.0);
        }
    }
}
