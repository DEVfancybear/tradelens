//! The running total of R over time (`nsh_base::plot`), the overview's R tab.

use nsh_base::{
    plot::{AppearKey, AreaY0, AxisPlacement, Curve, Scale, TickFormat, area, grid, line, plot, plot_axis},
    theme,
};
use nsh_core::{
    element::{RowAccessor, Rows},
    prelude::*,
};

use super::{
    chart::{Baseline, nice_domain},
    motion, theme as tokens,
};
use crate::format;

/// Milliseconds in a minute: the x axis counts minutes, which stay small enough for `f32`.
const MINUTE_MS: i64 = 60_000;
const LABEL_GUTTER: f32 = 56.0;
const HEIGHT: f32 = 300.0;

/// One closed trade's place on the curve: when it closed and the running total of R after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RPoint {
    pub(crate) time_ms: i64,
    pub(crate) total_r: f64,
}

fn minutes(points: &[RPoint]) -> Vec<f64> {
    let first = points.first().map_or(0, |p| p.time_ms);
    points.iter().map(|p| f64::from(i32::try_from((p.time_ms - first) / MINUTE_MS).unwrap_or(i32::MAX))).collect()
}

/// The running total of R as an area and a line. `points` is read again when it changes;
/// `generation` changes with the period, which draws the curve in again.
pub(crate) fn r_curve_chart(
    points: impl Fn() -> Vec<RPoint> + Copy + 'static,
    generation: impl Fn() -> u32 + Copy + 'static,
) -> impl IntoElement {
    let hovered = signal(None::<u32>);
    let rows = move || {
        let points = points();
        let x = minutes(&points);
        Rows::from_columns(
            points.len(),
            [("minute".into(), x), ("r".into(), points.iter().map(|p| p.total_r).collect())],
        )
    };
    let motion = nsh_base::plot::PlotMotion {
        appear: (!reduce_motion())
            .then(|| nsh_core::element::Timing::new(motion::ENTER.as_secs_f32() * 1000.0, Easing::EaseOut)),
        ..nsh_base::plot::PlotMotion::default()
    };
    let readout = move || {
        let at = hovered.get().and_then(|i| usize::try_from(i).ok());
        let all = points();
        let shown = at.and_then(|i| all.get(i)).or(all.last());
        shown.map_or_else(String::new, |p| format!("Tổng R {}", format::r_text(Some(p.total_r))))
    };
    view()
        .gap_2()
        .child(
            text(readout)
                .text_xs()
                .font_family(tokens::mono)
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            plot()
                .data(rows)
                .x(RowAccessor::key("minute"))
                .y(RowAccessor::key("r"))
                .y_scale(move || {
                    let all = points();
                    let low = all.iter().map(|p| p.total_r).fold(f64::INFINITY, f64::min);
                    let high = all.iter().map(|p| p.total_r).fold(f64::NEG_INFINITY, f64::max);
                    Scale::Linear { domain: Some(nice_domain(low, high, Baseline::Zero)), reverse: false }
                })
                .x_scale(Scale::Linear { domain: None, reverse: false })
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .accessibility_label("Tổng R tích luỹ theo thời gian")
                .h(px(HEIGHT))
                .ml(px(LABEL_GUTTER))
                .on_hover_change(move |index: &Option<u32>| hovered.set(*index))
                .cross_line_class("w-px h-full bg-border")
                .dot_class("size-2.5 rounded-full border-2 border-background bg-chart-1")
                .child(grid().stroke_color(move || theme().border))
                .child(
                    area()
                        .curve(Curve::Linear)
                        .y0(AreaY0::Value(0.0))
                        .fill_color(move || theme().chart_1.opacity(0.14)),
                )
                .child(line().curve(Curve::Linear).stroke_color(move || theme().chart_1).stroke_width(px(2.0)))
                .child(
                    plot_axis()
                        .placement(AxisPlacement::Left)
                        .format(TickFormat::from_fn(move |value, _| {
                            format::r_text(Some(value)).replace(".0R", "R").into()
                        }))
                        .label_class(super::chart::AXIS_LABEL_CLASS),
                ),
        )
}
