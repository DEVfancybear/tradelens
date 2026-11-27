//! The running total of R trade by trade (`nsh_base::plot`), the overview's R tab.

use nsh_base::{
    plot::{AppearKey, AreaY0, AxisPlacement, Curve, Scale, TickFormat, area, grid, line, plot, plot_axis},
    theme,
};
use nsh_core::{
    element::{RowAccessor, Rows},
    prelude::*,
};

use super::{
    chart::{Baseline, TRADE_SPACING, value_axis},
    motion, theme as tokens,
};
use crate::format;

const LABEL_GUTTER: f32 = 56.0;
const HEIGHT: f32 = 300.0;

/// The running total of R as an area and a line, one trade apart as the balance curve is.
/// `points` holds the total after each closed trade, oldest first, and is read again when it
/// changes; `generation` changes with the period, which draws the curve in again.
pub(crate) fn r_curve_chart(
    points: impl Fn() -> Vec<f64> + Copy + 'static,
    generation: impl Fn() -> u32 + Copy + 'static,
) -> impl IntoElement {
    let hovered = signal(None::<u32>);
    let rows = move || {
        let points = points();
        Rows::from_columns(points.len(), [("r".into(), points)])
    };
    let axis = computed(move || {
        let all = points();
        let low = all.iter().copied().fold(f64::INFINITY, f64::min);
        let high = all.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        value_axis(low, high, Baseline::Zero)
    });
    let motion = nsh_base::plot::PlotMotion {
        appear: (!reduce_motion())
            .then(|| nsh_core::element::Timing::new(motion::ENTER.as_secs_f32() * 1000.0, Easing::EaseOut)),
        ..nsh_base::plot::PlotMotion::default()
    };
    let readout = move || {
        let at = hovered.get().and_then(|i| usize::try_from(i).ok());
        let all = points();
        let shown = at.and_then(|i| all.get(i)).or(all.last());
        shown.map_or_else(String::new, |&total| t!("ui.r_chart.total", r = format::r_text(Some(total))))
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
                .y(RowAccessor::key("r"))
                .y_scale(move || Scale::Linear { domain: Some(axis.with(|a| a.domain)), reverse: false })
                .x_scale(TRADE_SPACING)
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .accessibility_label(t!("ui.r_chart.label"))
                .h(px(HEIGHT))
                .ml(px(LABEL_GUTTER))
                .on_hover_change(move |index: &Option<u32>| hovered.set(*index))
                .cross_line_class("w-px h-full bg-border")
                .dot_class("size-2.5 rounded-full border-2 border-background bg-chart-1")
                .child(grid().y(move || axis.with(|a| a.ticks.clone())).stroke_color(move || theme().border))
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
                        .tick_values(move || axis.with(|a| a.ticks.clone()))
                        .format(TickFormat::from_fn(move |value, _| {
                            format::r_text(Some(value)).replace(".0R", "R").into()
                        }))
                        .label_class(super::chart::AXIS_LABEL_CLASS),
                ),
        )
}
