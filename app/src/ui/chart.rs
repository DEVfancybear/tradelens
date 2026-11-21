//! The balance curve with its drawdown underneath (`nsh_base::plot`).

use nsh_base::{
    plot::{AppearKey, AxisPlacement, Scale, TickFormat, area, grid, line, plot, plot_axis},
    theme,
};
use nsh_core::{
    element::{RowAccessor, Rows},
    prelude::*,
};
use tradelens_core::metrics::EquityPoint;

use super::{motion, theme as tokens};
use crate::format::{self, Sign};

/// Milliseconds in a minute: the curve's x axis counts minutes, which stay small enough
/// to keep every digit in an `f32` chart.
const MINUTE_MS: i64 = 60_000;

/// Room the balance's labels take at the left of both plots, so their edges line up.
const LABEL_GUTTER: f32 = 72.0;

const BALANCE_HEIGHT: f32 = 220.0;
const DRAWDOWN_HEIGHT: f32 = 84.0;

/// Minutes since the first point, as the plots' x values.
fn minutes(points: &[EquityPoint]) -> Vec<f64> {
    let first = points.first().map_or(0, |p| p.time_ms);
    points.iter().map(|p| f64::from(i32::try_from((p.time_ms - first) / MINUTE_MS).unwrap_or(i32::MAX))).collect()
}

/// The balance curve as an area and a line, and the drawdown from its peak below it.
/// `points` is read again when it changes; `currency` labels the axis. `generation` changes
/// when the period does, which draws the curve in again.
pub(crate) fn equity_chart(
    points: impl Fn() -> Vec<EquityPoint> + Copy + 'static,
    currency: impl Fn() -> String + Copy + 'static,
    generation: impl Fn() -> u32 + Copy + 'static,
) -> impl IntoElement {
    let hovered = signal(None::<u32>);
    let balance_rows = move || {
        let points = points();
        let x = minutes(&points);
        Rows::from_columns(
            points.len(),
            [("minute".into(), x), ("balance".into(), points.iter().map(|p| p.balance).collect())],
        )
    };
    let drawdown_rows = move || {
        let points = points();
        let x = minutes(&points);
        Rows::from_columns(
            points.len(),
            [("minute".into(), x), ("drawdown".into(), points.iter().map(|p| -p.drawdown).collect())],
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
        shown.map_or_else(String::new, |p| {
            format!("Số dư {}", format::money(p.balance, Sign::OnlyNegative, &currency()))
        })
    };
    let max_drawdown = move || points().iter().map(|p| p.drawdown).fold(0.0_f64, f64::max);

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
                .data(balance_rows)
                .x(RowAccessor::key("minute"))
                .y(RowAccessor::key("balance"))
                .x_scale(Scale::Linear { domain: None, reverse: false })
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .accessibility_label("Đường số dư theo thời gian")
                .h(px(BALANCE_HEIGHT))
                .ml(px(LABEL_GUTTER))
                .on_hover_change(move |index: &Option<u32>| hovered.set(*index))
                .cross_line_class("w-px h-full bg-border")
                .dot_class("size-2.5 rounded-full border-2 border-background bg-chart-1")
                .child(grid().stroke_color(move || theme().border))
                .child(area().fill_color(move || theme().chart_1.opacity(0.14)))
                .child(line().stroke_color(move || theme().chart_1).stroke_width(px(2.0)))
                .child(
                    plot_axis()
                        .placement(AxisPlacement::Left)
                        .format(TickFormat::from_fn(move |value, _| {
                            format::money(value, Sign::OnlyNegative, &currency())
                                .split('.')
                                .next()
                                .unwrap_or_default()
                                .to_owned()
                                .into()
                        }))
                        .label_class("pr-3 text-xs text-muted-foreground"),
                ),
        )
        .child(
            view()
                .flex_row()
                .justify_between()
                .ml(px(LABEL_GUTTER))
                .child(text("Drawdown").text_xs().text_color(move || theme().muted_foreground))
                .child(
                    text(move || format!("tối đa {}", format::money(-max_drawdown(), Sign::OnlyNegative, &currency())))
                        .text_xs()
                        .font_family(tokens::mono)
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            plot()
                .data(drawdown_rows)
                .x(RowAccessor::key("minute"))
                .y(RowAccessor::key("drawdown"))
                .x_scale(Scale::Linear { domain: None, reverse: false })
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .interactive(false)
                .accessibility_label("Drawdown từ đỉnh số dư")
                .h(px(DRAWDOWN_HEIGHT))
                .ml(px(LABEL_GUTTER))
                .child(area().fill_color(move || tokens::loss().opacity(0.25)))
                .child(line().stroke_color(tokens::loss).stroke_width(px(1.5))),
        )
}
