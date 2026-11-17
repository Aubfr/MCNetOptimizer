//! Graphique de latence dessine directement avec le peintre egui (aucune dependance en plus).

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Shape, Stroke};

use crate::theme;

pub struct ChartSeries<'a> {
    pub name: &'static str,
    pub color: Color32,
    pub points: &'a [(u64, Option<f32>)],
}

fn nice_ceiling(v: f32) -> f32 {
    let steps = [20.0, 30.0, 50.0, 80.0, 100.0, 150.0, 200.0, 300.0, 500.0, 1000.0, 2000.0];
    for s in steps {
        if v <= s {
            return s;
        }
    }
    v
}

pub fn latency_chart(
    ui: &mut egui::Ui,
    height: f32,
    window_s: f64,
    now_ms: u64,
    series: &[ChartSeries],
    markers: &[u64],
) {
    let w = ui.available_width();
    let (resp, painter) = ui.allocate_painter(vec2(w, height), egui::Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, 6.0, theme::CHART_BG);
    let plot = Rect::from_min_max(rect.min + vec2(48.0, 26.0), rect.max - vec2(10.0, 20.0));
    if plot.width() < 20.0 || plot.height() < 20.0 {
        return;
    }

    let mut vmax = 20.0f32;
    for s in series {
        for (_, r) in s.points.iter() {
            if let Some(v) = r {
                if *v > vmax {
                    vmax = *v;
                }
            }
        }
    }
    let vmax = nice_ceiling(vmax * 1.05);
    let win_ms = (window_s * 1000.0) as f32;

    let x_of = |t: u64| -> f32 {
        let age = now_ms.saturating_sub(t) as f32;
        (plot.right() - (age / win_ms) * plot.width()).max(plot.left())
    };
    let y_of = |v: f32| -> f32 { plot.bottom() - (v / vmax).min(1.0) * plot.height() };

    // Grille
    for i in 0..=4 {
        let val = vmax * i as f32 / 4.0;
        let y = y_of(val);
        painter.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, theme::GRID));
        painter.text(
            pos2(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{:.0}", val),
            FontId::proportional(11.0),
            theme::MUTED,
        );
    }
    painter.text(
        pos2(plot.left(), plot.bottom() + 4.0),
        Align2::LEFT_TOP,
        format!("-{:.0} s", window_s),
        FontId::proportional(11.0),
        theme::MUTED,
    );
    painter.text(
        pos2(plot.right(), plot.bottom() + 4.0),
        Align2::RIGHT_TOP,
        "maintenant (ms)",
        FontId::proportional(11.0),
        theme::MUTED,
    );

    // Marqueurs
    for t in markers {
        if now_ms.saturating_sub(*t) as f32 <= win_ms {
            let x = x_of(*t);
            painter.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.5, theme::WARN));
        }
    }

    // Series
    for s in series {
        let stroke = Stroke::new(1.8, s.color);
        let mut seg: Vec<egui::Pos2> = Vec::new();
        let flush = |seg: &mut Vec<egui::Pos2>| {
            if seg.len() >= 2 {
                painter.add(Shape::line(seg.clone(), stroke));
            } else if seg.len() == 1 {
                painter.circle_filled(seg[0], 2.0, s.color);
            }
            seg.clear();
        };
        for (t, r) in s.points.iter() {
            match r {
                Some(v) => seg.push(pos2(x_of(*t), y_of(*v))),
                None => {
                    flush(&mut seg);
                    let x = x_of(*t);
                    painter.line_segment(
                        [pos2(x, plot.bottom()), pos2(x, plot.bottom() - 14.0)],
                        Stroke::new(2.0, theme::BAD),
                    );
                }
            }
        }
        flush(&mut seg);
    }

    // Legende
    let mut x = plot.left() + 4.0;
    for s in series {
        painter.rect_filled(Rect::from_min_size(pos2(x, rect.top() + 9.0), vec2(10.0, 10.0)), 2.0, s.color);
        let r = painter.text(
            pos2(x + 14.0, rect.top() + 7.0),
            Align2::LEFT_TOP,
            s.name,
            FontId::proportional(12.0),
            theme::TEXT,
        );
        x = r.right() + 18.0;
    }
    painter.text(
        pos2(plot.right(), rect.top() + 7.0),
        Align2::RIGHT_TOP,
        "trait rouge = perte ; trait jaune = marqueur",
        FontId::proportional(11.0),
        theme::MUTED,
    );
}
