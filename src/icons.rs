use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
#[derive(Clone, Copy)]
pub enum Icon {
    Refresh,
    Home,
    Root,
    Up,
    Search,
    Trash,
    Copy,
    Cancel,
    Folder,
    Video,
}
pub fn button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tip: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (r, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
        let style = ui.style().interact(&response);
        ui.painter().rect(
            r,
            4.0,
            style.weak_bg_fill,
            style.bg_stroke,
            egui::StrokeKind::Inside,
        );
        paint(
            ui,
            r.shrink(5.0),
            icon,
            if matches!(icon, Icon::Cancel) {
                Color32::from_rgb(235, 70, 70)
            } else {
                style.fg_stroke.color
            },
        );
        response.on_hover_text(tip)
    })
    .inner
}
pub fn file(ui: &mut egui::Ui, dir: bool) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    paint(
        ui,
        r.shrink(1.0),
        if dir { Icon::Folder } else { Icon::Video },
        if dir {
            Color32::from_rgb(240, 190, 65)
        } else {
            Color32::from_rgb(80, 170, 235)
        },
    );
}
fn paint(ui: &egui::Ui, rect: Rect, icon: Icon, color: Color32) {
    let p = ui.painter();
    let s = Stroke::new(1.7_f32, color);
    let point = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([point(a.0, a.1), point(b.0, b.1)], s);
    };
    let box_at = |a: (f32, f32), b: (f32, f32)| {
        p.rect_stroke(
            Rect::from_two_pos(point(a.0, a.1), point(b.0, b.1)),
            1.0,
            s,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Cancel => {
            line((0.15, 0.15), (0.85, 0.85));
            line((0.85, 0.15), (0.15, 0.85));
        }
        Icon::Home => {
            line((0.05, 0.45), (0.5, 0.05));
            line((0.5, 0.05), (0.95, 0.45));
            box_at((0.2, 0.4), (0.8, 0.95));
            box_at((0.43, 0.62), (0.6, 0.95));
        }
        Icon::Folder | Icon::Root => {
            p.rect_filled(
                Rect::from_two_pos(point(0.03, 0.3), point(0.97, 0.93)),
                2.0,
                color,
            );
            p.rect_filled(
                Rect::from_two_pos(point(0.03, 0.12), point(0.46, 0.4)),
                2.0,
                color,
            );
            if matches!(icon, Icon::Root) {
                p.text(
                    point(0.5, 0.6),
                    egui::Align2::CENTER_CENTER,
                    "/…",
                    egui::FontId::proportional(11.0),
                    ui.visuals().panel_fill,
                );
            }
        }
        Icon::Video => {
            box_at((0.05, 0.12), (0.95, 0.88));
            for y in [0.22, 0.5, 0.78] {
                line((0.1, y), (0.24, y));
                line((0.76, y), (0.9, y));
            }
            p.add(egui::Shape::convex_polygon(
                vec![point(0.4, 0.32), point(0.4, 0.68), point(0.68, 0.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Search => {
            p.circle_stroke(point(0.38, 0.38), rect.width() * 0.3, s);
            line((0.62, 0.62), (0.95, 0.95));
        }
        Icon::Trash => {
            box_at((0.23, 0.28), (0.78, 0.94));
            line((0.1, 0.23), (0.9, 0.23));
            box_at((0.36, 0.04), (0.64, 0.22));
            line((0.4, 0.4), (0.4, 0.8));
            line((0.6, 0.4), (0.6, 0.8));
        }
        Icon::Copy => {
            box_at((0.1, 0.03), (0.7, 0.76));
            box_at((0.32, 0.25), (0.94, 0.98));
        }
        Icon::Up => {
            line((0.83, 0.8), (0.38, 0.8));
            line((0.38, 0.8), (0.38, 0.15));
            line((0.1, 0.42), (0.38, 0.15));
            line((0.38, 0.15), (0.66, 0.42));
        }
        Icon::Refresh => {
            let center = point(0.5, 0.5);
            let radius = rect.width() * 0.36;
            for (start, end) in [(0.3_f32, 2.8_f32), (3.45, 5.95)] {
                let pts = (0..18)
                    .map(|i| {
                        let a = start + (end - start) * i as f32 / 17.0;
                        center + Vec2::new(a.cos(), a.sin()) * radius
                    })
                    .collect();
                p.add(egui::Shape::line(pts, s));
            }
            line((0.05, 0.5), (0.15, 0.67));
            line((0.15, 0.67), (0.31, 0.56));
            line((0.95, 0.5), (0.85, 0.33));
            line((0.85, 0.33), (0.69, 0.44));
        }
    }
}
