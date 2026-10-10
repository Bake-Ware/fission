//! Small vector icons drawn with the egui painter, so there are no image assets.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Stroke, Vec2};

use super::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sketch,
    Line,
    Rectangle,
    Circle,
    Arc,
    Dimension,
    FinishSketch,
    ConHV,
    ConCoincident,
    ConTangent,
    ConEqual,
    ConParallel,
    ConPerpendicular,
    ConFix,
    ConMidpoint,
    ConConcentric,
    Shell,
    Combine,
    Move,
    Mirror,
    PatternRect,
    PatternCirc,
    ExtrudeBox,
    ExtrudeCylinder,
    Revolve,
    Fillet,
    Chamfer,
    ExportStl,
    ExportStep,
    Eye,
    EyeOff,
    Body,
    OffsetPlane,
    PlaneAtAngle,
    Scale,
    Split,
    OffsetFace,
    Point,
    Sphere,
    Thread,
    Coil,
    DeleteFace,
    Unsupported,
    Torus,
    Draft,
    Axis,
    Midplane,
    Measure,
    Parameters,
    Sweep,
    Loft,
    Section,
    Appearance,
    Polygon,
    Slot,
}

/// Map a unit-square point (0..1, y down) into `rect`.
fn at(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(rect.min.x + x * rect.width(), rect.min.y + y * rect.height())
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new((rect.width() / 18.0).max(1.2), color);
    let fill = color.linear_multiply(0.12);
    let p = |x, y| at(rect, x, y);
    match icon {
        Icon::Sketch => {
            // A dashed rectangle with a pencil across it.
            let r = [p(0.12, 0.30), p(0.70, 0.30), p(0.70, 0.88), p(0.12, 0.88), p(0.12, 0.30)];
            painter.add(egui::Shape::dashed_line(&r, Stroke::new(s.width * 0.8, color), 3.0, 2.0));
            painter.line_segment([p(0.40, 0.62), p(0.88, 0.14)], Stroke::new(s.width * 2.2, theme::accent()));
            painter.line_segment([p(0.33, 0.70), p(0.40, 0.62)], s);
        }
        Icon::Line => {
            painter.line_segment([p(0.15, 0.85), p(0.85, 0.15)], s);
            for q in [p(0.15, 0.85), p(0.85, 0.15)] {
                painter.rect_filled(Rect::from_center_size(q, Vec2::splat(rect.width() * 0.14)), 0.0, color);
            }
        }
        Icon::Rectangle => {
            painter.rect_stroke(Rect::from_min_max(p(0.15, 0.25), p(0.85, 0.80)), 0.0, s, egui::StrokeKind::Middle);
            for q in [p(0.15, 0.25), p(0.85, 0.80)] {
                painter.rect_filled(Rect::from_center_size(q, Vec2::splat(rect.width() * 0.14)), 0.0, color);
            }
        }
        Icon::Circle => {
            painter.circle_stroke(p(0.5, 0.5), rect.width() * 0.36, s);
            painter.circle_filled(p(0.5, 0.5), rect.width() * 0.06, color);
        }
        Icon::Arc => {
            let c = p(0.5, 0.75);
            let r = rect.width() * 0.4;
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 20.0;
                    Pos2::new(c.x + r * a.cos(), c.y - r * a.sin())
                })
                .collect();
            painter.line(pts, s);
            painter.circle_filled(c, rect.width() * 0.06, color);
        }
        Icon::Dimension => {
            painter.line_segment([p(0.12, 0.70), p(0.88, 0.70)], s);
            painter.line_segment([p(0.12, 0.55), p(0.12, 0.85)], s);
            painter.line_segment([p(0.88, 0.55), p(0.88, 0.85)], s);
            painter.text(p(0.5, 0.40), egui::Align2::CENTER_CENTER, "10", egui::FontId::proportional(rect.height() * 0.36), theme::accent());
        }
        Icon::FinishSketch => {
            painter.line(vec![p(0.15, 0.55), p(0.42, 0.80), p(0.88, 0.22)], Stroke::new(s.width * 2.0, Color32::from_rgb(40, 160, 70)));
        }
        Icon::ConHV => {
            painter.line_segment([p(0.12, 0.72), p(0.62, 0.72)], s);
            painter.line_segment([p(0.80, 0.15), p(0.80, 0.85)], s);
        }
        Icon::ConCoincident => {
            painter.line_segment([p(0.12, 0.80), p(0.50, 0.45)], s);
            painter.line_segment([p(0.50, 0.45), p(0.88, 0.80)], s);
            painter.circle_filled(p(0.50, 0.45), rect.width() * 0.10, theme::accent());
        }
        Icon::ConTangent => {
            painter.circle_stroke(p(0.50, 0.58), rect.width() * 0.28, s);
            painter.line_segment([p(0.08, 0.30), p(0.92, 0.30)], Stroke::new(s.width, theme::accent()));
        }
        Icon::ConEqual => {
            painter.line_segment([p(0.18, 0.40), p(0.82, 0.40)], s);
            painter.line_segment([p(0.18, 0.62), p(0.82, 0.62)], s);
        }
        Icon::ConParallel => {
            painter.line_segment([p(0.20, 0.85), p(0.50, 0.15)], s);
            painter.line_segment([p(0.50, 0.85), p(0.80, 0.15)], s);
        }
        Icon::ConPerpendicular => {
            painter.line_segment([p(0.15, 0.82), p(0.85, 0.82)], s);
            painter.line_segment([p(0.50, 0.82), p(0.50, 0.15)], s);
        }
        Icon::ConFix => {
            // Padlock.
            painter.rect_filled(Rect::from_min_max(p(0.22, 0.45), p(0.78, 0.88)), 2.0, color);
            let shackle: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 12.0;
                    let c = p(0.5, 0.45);
                    Pos2::new(c.x + rect.width() * 0.18 * a.cos(), c.y - rect.width() * 0.22 * a.sin())
                })
                .collect();
            painter.line(shackle, s);
        }
        Icon::ConMidpoint => {
            painter.line_segment([p(0.10, 0.75), p(0.90, 0.75)], s);
            painter.add(egui::Shape::convex_polygon(vec![p(0.50, 0.38), p(0.66, 0.62), p(0.34, 0.62)], theme::accent(), Stroke::NONE));
        }
        Icon::ConConcentric => {
            painter.circle_stroke(p(0.5, 0.5), rect.width() * 0.38, s);
            painter.circle_stroke(p(0.5, 0.5), rect.width() * 0.18, Stroke::new(s.width, theme::accent()));
        }
        Icon::Shell => {
            painter.rect_stroke(Rect::from_min_max(p(0.12, 0.25), p(0.88, 0.85)), 2.0, s, egui::StrokeKind::Middle);
            painter.rect_stroke(Rect::from_min_max(p(0.28, 0.25), p(0.72, 0.70)), 1.0, Stroke::new(s.width, theme::accent()), egui::StrokeKind::Middle);
        }
        Icon::Combine => {
            painter.rect_filled(Rect::from_min_max(p(0.12, 0.35), p(0.62, 0.85)), 2.0, fill);
            painter.rect_stroke(Rect::from_min_max(p(0.12, 0.35), p(0.62, 0.85)), 2.0, s, egui::StrokeKind::Middle);
            painter.rect_stroke(Rect::from_min_max(p(0.38, 0.15), p(0.88, 0.65)), 2.0, Stroke::new(s.width, theme::accent()), egui::StrokeKind::Middle);
        }
        Icon::Move => {
            let c = p(0.5, 0.5);
            for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                painter.arrow(c, d * rect.width() * 0.38, s);
            }
        }
        Icon::OffsetPlane => {
            // Two stacked planes with an arrow between them.
            let low = vec![p(0.08, 0.78), p(0.62, 0.78), p(0.92, 0.58), p(0.38, 0.58)];
            let high = vec![p(0.08, 0.40), p(0.62, 0.40), p(0.92, 0.20), p(0.38, 0.20)];
            painter.add(egui::Shape::convex_polygon(low, fill, s));
            painter.add(egui::Shape::convex_polygon(high, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
            painter.arrow(p(0.50, 0.66), Vec2::new(0.0, -0.30 * rect.height()), s);
        }
        Icon::PlaneAtAngle => {
            // A flat plane with a second one tipped up from its front edge.
            let flat = vec![p(0.08, 0.82), p(0.62, 0.82), p(0.92, 0.62), p(0.38, 0.62)];
            let tipped = vec![p(0.08, 0.82), p(0.62, 0.82), p(0.80, 0.22), p(0.26, 0.22)];
            painter.add(egui::Shape::convex_polygon(flat, fill, s));
            painter.add(egui::Shape::convex_polygon(tipped, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
        }
        Icon::Scale => {
            // A small square growing into a big one along the diagonal.
            painter.rect_stroke(Rect::from_min_max(p(0.12, 0.52), p(0.48, 0.88)), 1.0, s, egui::StrokeKind::Middle);
            painter.rect_stroke(Rect::from_min_max(p(0.12, 0.12), p(0.88, 0.88)), 1.0, Stroke::new(s.width, theme::accent()), egui::StrokeKind::Middle);
            painter.arrow(p(0.42, 0.58), Vec2::new(0.30, -0.30) * rect.width(), s);
        }
        Icon::Unsupported => {
            // A dashed box with a question mark: a feature kept as a marker.
            let r = Rect::from_min_max(p(0.15, 0.15), p(0.85, 0.85));
            let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.add(egui::Shape::dashed_line(&corners, s, rect.width() * 0.1, rect.width() * 0.06));
            painter.text(r.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(rect.height() * 0.55), theme::error());
        }
        Icon::DeleteFace => {
            // A block with its face struck through.
            let body = vec![p(0.15, 0.85), p(0.75, 0.85), p(0.75, 0.30), p(0.15, 0.30)];
            painter.add(egui::Shape::closed_line(body, s));
            let x = Stroke::new(s.width * 1.3, theme::error());
            painter.line_segment([p(0.55, 0.12), p(0.92, 0.50)], x);
            painter.line_segment([p(0.92, 0.12), p(0.55, 0.50)], x);
        }
        Icon::Thread => {
            // A bolt shank with slanted thread crests.
            painter.rect_stroke(Rect::from_min_max(p(0.30, 0.10), p(0.70, 0.90)), 1.0, s, egui::StrokeKind::Middle);
            let crest = Stroke::new(s.width, theme::accent());
            for i in 0..5 {
                let y = 0.22 + 0.15 * i as f32;
                painter.line_segment([p(0.22, y + 0.06), p(0.78, y - 0.06)], crest);
            }
        }
        Icon::Coil => {
            // A spring seen from the side.
            let pts: Vec<Pos2> = (0..=96)
                .map(|i| {
                    let t = i as f32 / 96.0;
                    let a = t * 4.0 * std::f32::consts::TAU;
                    p(0.5 + 0.30 * a.cos(), 0.88 - 0.76 * t + 0.07 * a.sin())
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(s.width, theme::accent())));
        }
        Icon::Sphere => {
            // A ball: an outline, its equator, and a highlight.
            painter.circle(p(0.5, 0.5), rect.width() * 0.36, theme::accent().linear_multiply(0.2), s);
            let eq: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = std::f32::consts::PI * i as f32 / 24.0;
                    p(0.5 - 0.36 * t.cos(), 0.5 + 0.10 * t.sin())
                })
                .collect();
            painter.add(egui::Shape::line(eq, Stroke::new(s.width, theme::accent())));
        }
        Icon::Torus => {
            // A ring seen at an angle: two ellipses.
            for (rx, ry, stroke) in [(0.40, 0.24, s), (0.16, 0.08, s)] {
                let pts: Vec<Pos2> = (0..=32)
                    .map(|i| {
                        let t = std::f32::consts::TAU * i as f32 / 32.0;
                        p(0.5 + rx * t.cos(), 0.5 + ry * t.sin())
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
            painter.add(egui::Shape::line_segment([p(0.10, 0.5), p(0.34, 0.5)], Stroke::new(s.width, theme::accent())));
        }
        Icon::Point => {
            // An accent dot where two faint lines cross.
            let faint = Stroke::new(s.width * 0.7, color.linear_multiply(0.5));
            painter.line_segment([p(0.12, 0.72), p(0.88, 0.40)], faint);
            painter.line_segment([p(0.30, 0.15), p(0.62, 0.90)], faint);
            painter.circle(p(0.50, 0.57), rect.width() * 0.11, theme::accent(), Stroke::new(s.width * 0.8, color));
        }
        Icon::OffsetFace => {
            // A block whose top is lifted, with an arrow pulling it.
            let body = vec![p(0.15, 0.88), p(0.70, 0.88), p(0.70, 0.50), p(0.15, 0.50)];
            painter.add(egui::Shape::closed_line(body, s));
            let lifted = vec![p(0.15, 0.50), p(0.70, 0.50), p(0.70, 0.30), p(0.15, 0.30)];
            painter.add(egui::Shape::convex_polygon(lifted, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
            painter.arrow(p(0.84, 0.62), Vec2::new(0.0, -0.48) * rect.width(), Stroke::new(s.width, theme::accent()));
        }
        Icon::Split => {
            // A block in two halves pulled apart along an accent cut.
            let left = vec![p(0.10, 0.82), p(0.44, 0.82), p(0.38, 0.22), p(0.10, 0.22)];
            let right = vec![p(0.56, 0.82), p(0.90, 0.82), p(0.90, 0.22), p(0.62, 0.22)];
            painter.add(egui::Shape::closed_line(left, s));
            painter.add(egui::Shape::convex_polygon(right, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
            let dash = Stroke::new(s.width, theme::accent());
            painter.add(egui::Shape::dashed_line(&[p(0.53, 0.95), p(0.47, 0.08)], dash, rect.width() * 0.08, rect.width() * 0.06));
        }
        Icon::Draft => {
            // A block whose sides lean in, over its upright outline.
            let upright = vec![p(0.18, 0.85), p(0.82, 0.85), p(0.82, 0.18), p(0.18, 0.18)];
            painter.add(egui::Shape::closed_line(upright, Stroke::new(s.width * 0.6, color.linear_multiply(0.5))));
            let leaning = vec![p(0.18, 0.85), p(0.82, 0.85), p(0.68, 0.18), p(0.32, 0.18)];
            painter.add(egui::Shape::convex_polygon(leaning, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
        }
        Icon::Axis => {
            // A dashed accent line across a cylinder outline.
            painter.circle_stroke(p(0.5, 0.5), rect.width() * 0.26, s);
            let dash = Stroke::new(s.width * 1.3, theme::accent());
            painter.add(egui::Shape::dashed_line(&[p(0.12, 0.88), p(0.88, 0.12)], dash, rect.width() * 0.12, rect.width() * 0.07));
        }
        Icon::Midplane => {
            // Two planes with the accent one halfway between.
            for (y, accent) in [(0.82, false), (0.52, true), (0.22, false)] {
                let quad = vec![p(0.08, y), p(0.62, y), p(0.92, y - 0.16), p(0.38, y - 0.16)];
                if accent {
                    painter.add(egui::Shape::convex_polygon(quad, theme::accent().linear_multiply(0.25), Stroke::new(s.width, theme::accent())));
                } else {
                    painter.add(egui::Shape::convex_polygon(quad, fill, s));
                }
            }
        }
        Icon::Slot => {
            let r = rect.height() * 0.2;
            let (a, b) = (p(0.28, 0.5), p(0.72, 0.5));
            painter.line_segment([a + Vec2::new(0.0, -r), b + Vec2::new(0.0, -r)], s);
            painter.line_segment([a + Vec2::new(0.0, r), b + Vec2::new(0.0, r)], s);
            for (c, sign) in [(a, -1.0f32), (b, 1.0)] {
                let pts: Vec<Pos2> = (0..=12)
                    .map(|i| {
                        let t = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                        c + Vec2::new(sign * t.cos(), t.sin()) * r
                    })
                    .collect();
                painter.line(pts, s);
            }
        }
        Icon::Polygon => {
            let c = p(0.5, 0.52);
            let r = rect.width() * 0.38;
            let pts: Vec<Pos2> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 6.0;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(egui::Shape::closed_line(pts, s));
        }
        Icon::Appearance => {
            // Three paint dots.
            for (x, y, c) in [(0.32, 0.36, Color32::from_rgb(204, 46, 41)), (0.66, 0.36, Color32::from_rgb(51, 107, 204)), (0.49, 0.66, Color32::from_rgb(238, 205, 51))] {
                painter.circle_filled(p(x, y), rect.width() * 0.17, c);
                painter.circle_stroke(p(x, y), rect.width() * 0.17, Stroke::new(s.width * 0.6, color));
            }
        }
        Icon::Section => {
            // A box with its front half cut away along a plane.
            let back = vec![p(0.40, 0.15), p(0.90, 0.15), p(0.90, 0.65), p(0.40, 0.65)];
            painter.add(egui::Shape::convex_polygon(back, fill, s));
            let cut = vec![p(0.10, 0.35), p(0.60, 0.35), p(0.60, 0.85), p(0.10, 0.85)];
            painter.add(egui::Shape::convex_polygon(cut, theme::accent().linear_multiply(0.30), Stroke::new(s.width, theme::accent())));
        }
        Icon::Sweep => {
            // A circle swept along a bent path.
            let path = [p(0.20, 0.85), p(0.20, 0.45), p(0.30, 0.25), p(0.50, 0.18), p(0.85, 0.18)];
            painter.line(path.to_vec(), Stroke::new(s.width, theme::accent()));
            painter.circle_stroke(p(0.20, 0.82), rect.width() * 0.13, s);
            painter.circle_stroke(p(0.82, 0.18), rect.width() * 0.13, s);
        }
        Icon::Loft => {
            // A big section blending into a small one.
            let low = vec![p(0.08, 0.80), p(0.58, 0.80), p(0.92, 0.62), p(0.42, 0.62)];
            let high = vec![p(0.34, 0.30), p(0.58, 0.30), p(0.72, 0.20), p(0.48, 0.20)];
            painter.add(egui::Shape::convex_polygon(low, fill, s));
            painter.add(egui::Shape::convex_polygon(high, fill, s));
            for (a, b) in [(p(0.08, 0.80), p(0.34, 0.30)), (p(0.58, 0.80), p(0.58, 0.30)), (p(0.92, 0.62), p(0.72, 0.20))] {
                painter.line_segment([a, b], Stroke::new(s.width * 0.7, color));
            }
        }
        Icon::Parameters => {
            // "fx"
            painter.text(p(0.5, 0.52), egui::Align2::CENTER_CENTER, "fx", egui::FontId::proportional(rect.height() * 0.62), color);
        }
        Icon::Measure => {
            // A ruler with ticks.
            let r = vec![p(0.06, 0.62), p(0.62, 0.06), p(0.94, 0.38), p(0.38, 0.94)];
            painter.add(egui::Shape::convex_polygon(r, fill, s));
            for i in 1..6 {
                let t = i as f32 / 6.0;
                let a = p(0.06 + 0.56 * t, 0.62 - 0.56 * t);
                let len = if i % 2 == 0 { 0.16 } else { 0.10 };
                painter.line_segment([a, a + Vec2::new(len, len) * rect.width()], s);
            }
        }
        Icon::Mirror => {
            painter.line_segment([p(0.5, 0.08), p(0.5, 0.92)], Stroke::new(s.width, theme::accent()));
            painter.add(egui::Shape::convex_polygon(vec![p(0.40, 0.25), p(0.40, 0.80), p(0.10, 0.80)], fill, s));
            painter.add(egui::Shape::convex_polygon(vec![p(0.60, 0.25), p(0.90, 0.80), p(0.60, 0.80)], fill, s));
        }
        Icon::PatternRect => {
            for (x, y) in [(0.15, 0.15), (0.55, 0.15), (0.15, 0.55), (0.55, 0.55)] {
                painter.rect_stroke(Rect::from_min_size(p(x, y), Vec2::splat(rect.width() * 0.30)), 1.0, s, egui::StrokeKind::Middle);
            }
        }
        Icon::PatternCirc => {
            let c = p(0.5, 0.5);
            for i in 0..6 {
                let a = std::f32::consts::TAU * i as f32 / 6.0;
                painter.circle_stroke(c + Vec2::new(a.cos(), a.sin()) * rect.width() * 0.32, rect.width() * 0.09, s);
            }
        }
        Icon::ExtrudeBox => {
            // Isometric box with an up arrow.
            let front = vec![p(0.15, 0.50), p(0.55, 0.62), p(0.55, 0.92), p(0.15, 0.80)];
            let side = vec![p(0.55, 0.62), p(0.85, 0.50), p(0.85, 0.80), p(0.55, 0.92)];
            let top = vec![p(0.15, 0.50), p(0.45, 0.38), p(0.85, 0.50), p(0.55, 0.62)];
            for face in [front, side, top] {
                painter.add(egui::Shape::convex_polygon(face, fill, s));
            }
            painter.arrow(p(0.50, 0.42), Vec2::new(0.0, -0.36 * rect.height()), s);
        }
        Icon::ExtrudeCylinder => {
            let w = rect.width() * 0.32;
            let h = rect.height() * 0.10;
            let top = p(0.50, 0.40);
            let bottom = p(0.50, 0.82);
            painter.add(egui::Shape::ellipse_filled(bottom, Vec2::new(w, h), fill));
            painter.add(egui::Shape::ellipse_stroke(bottom, Vec2::new(w, h), s));
            painter.line_segment([top - Vec2::X * w, bottom - Vec2::X * w], s);
            painter.line_segment([top + Vec2::X * w, bottom + Vec2::X * w], s);
            painter.add(egui::Shape::ellipse_filled(top, Vec2::new(w, h), Color32::WHITE));
            painter.add(egui::Shape::ellipse_stroke(top, Vec2::new(w, h), s));
            painter.arrow(p(0.50, 0.36), Vec2::new(0.0, -0.28 * rect.height()), s);
        }
        Icon::Revolve => {
            let c = p(0.5, 0.55);
            let r = rect.width() * 0.30;
            let points: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.15 + 1.55 * i as f32 / 24.0);
                    Pos2::new(c.x + r * a.cos(), c.y - r * a.sin())
                })
                .collect();
            let end = *points.last().unwrap();
            painter.line(points, s);
            painter.arrow(end, Vec2::new(r * 0.45, -r * 0.1), s);
            painter.line_segment([p(0.5, 0.12), p(0.5, 0.95)], Stroke::new(s.width * 0.8, theme::muted()));
        }
        Icon::Fillet => {
            // A block whose top-right corner is rounded; the round is highlighted.
            let r = 0.35;
            let (cx, cy) = (0.85 - r, 0.15 + r);
            let arc: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_2 * (1.0 - i as f32 / 12.0);
                    p(cx + r * a.cos(), cy - r * a.sin())
                })
                .collect();
            painter.line(vec![p(cx, 0.15), p(0.15, 0.15), p(0.15, 0.85), p(0.85, 0.85), p(0.85, cy)], s);
            painter.line(arc, Stroke::new(s.width * 1.4, theme::accent()));
        }
        Icon::Chamfer => {
            painter.line(
                vec![p(0.15, 0.15), p(0.15, 0.85), p(0.85, 0.85), p(0.85, 0.45)],
                s,
            );
            painter.line_segment([p(0.85, 0.45), p(0.55, 0.15)], Stroke::new(s.width * 1.4, theme::accent()));
            painter.line_segment([p(0.55, 0.15), p(0.15, 0.15)], s);
        }
        Icon::ExportStl | Icon::ExportStep => {
            let page = vec![p(0.22, 0.10), p(0.62, 0.10), p(0.80, 0.28), p(0.80, 0.90), p(0.22, 0.90)];
            painter.add(egui::Shape::convex_polygon(page, Color32::WHITE, s));
            painter.line(vec![p(0.62, 0.10), p(0.62, 0.28), p(0.80, 0.28)], s);
            let label = if icon == Icon::ExportStl { "STL" } else { "STP" };
            painter.text(
                p(0.51, 0.64),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(rect.height() * 0.26),
                theme::accent(),
            );
        }
        Icon::Eye | Icon::EyeOff => {
            let c = rect.center();
            let w = rect.width() * 0.42;
            let h = rect.height() * 0.26;
            let upper: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0 * std::f32::consts::PI;
                    Pos2::new(c.x - w * t.cos(), c.y - h * t.sin())
                })
                .collect();
            let lower: Vec<Pos2> = upper.iter().map(|q| Pos2::new(q.x, 2.0 * c.y - q.y)).collect();
            painter.line(upper, s);
            painter.line(lower, s);
            painter.circle_filled(c, h * 0.6, color);
            if icon == Icon::EyeOff {
                painter.line_segment([p(0.15, 0.85), p(0.85, 0.15)], Stroke::new(s.width, color));
            }
        }
        Icon::Body => {
            let face = vec![p(0.2, 0.35), p(0.6, 0.35), p(0.6, 0.85), p(0.2, 0.85)];
            painter.add(egui::Shape::convex_polygon(face, fill, s));
            painter.line(vec![p(0.2, 0.35), p(0.4, 0.15), p(0.8, 0.15), p(0.6, 0.35)], s);
            painter.line(vec![p(0.8, 0.15), p(0.8, 0.65), p(0.6, 0.85)], s);
        }
    }
}

/// An icon-only button.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::palette().hover);
    }
    paint(ui.painter(), rect.shrink(size * 0.12), icon, theme::icon());
    response.on_hover_text(tooltip)
}
