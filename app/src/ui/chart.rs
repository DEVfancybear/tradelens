//! The balance curve with its drawdown underneath (`nsh_base::plot`).

use nsh_base::{
    plot::{AppearKey, AreaY0, AxisPlacement, Curve, Scale, TickFormat, area, grid, line, plot, plot_axis},
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

/// The look of an axis's labels: the figures of the design's rule 2, in the monospaced family
/// (`font-mono` of `theme.json`) with tabular digits.
pub(crate) const AXIS_LABEL_CLASS: &str = "pr-3 text-xs font-mono tabular-nums text-muted-foreground";

/// Room the balance's labels take at the left of both plots, so their edges line up.
const LABEL_GUTTER: f32 = 72.0;

const BALANCE_HEIGHT: f32 = 220.0;
const DRAWDOWN_HEIGHT: f32 = 84.0;

/// Whether a value axis must reach zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Baseline {
    /// Zero is on the axis (a running total of R, a drawdown): areas hang from it.
    Zero,
    /// Only the values are (a balance far from zero).
    Values,
}

/// The step of 1, 2 or 5 times a power of ten nearest above `raw`.
fn nice_step(raw: f64) -> f64 {
    let base = 10.0_f64.powf(raw.log10().floor());
    let mantissa = raw / base;
    let nice = if mantissa <= 1.0 {
        1.0
    } else if mantissa <= 2.0 {
        2.0
    } else if mantissa <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

/// The domain of a value axis for values from `low` to `high`: it holds them (and zero with
/// [`Baseline::Zero`]), is never flat, and its ends fall on round steps so that the axis has at
/// least three round labels. A line of one value (a balance that never moved, results of zero) gets
/// room above and below instead of a single label on a squashed axis.
pub(crate) fn nice_domain(low: f64, high: f64, baseline: Baseline) -> [f64; 2] {
    let (mut low, mut high) = (low.min(high), low.max(high));
    if baseline == Baseline::Zero {
        low = low.min(0.0);
        high = high.max(0.0);
    }
    if high - low < 1e-9 {
        let room = (low.abs() * 0.05).max(1.0);
        low -= room;
        high += room;
    }
    let step = nice_step((high - low) / 4.0);
    [(low / step).floor() * step, (high / step).ceil() * step]
}

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
            t!("ui.chart.balance", amount = format::money(p.balance, Sign::OnlyNegative, &currency()))
        })
    };
    let balance_domain = move || {
        let all = points();
        let low = all.iter().map(|p| p.balance).fold(f64::INFINITY, f64::min);
        let high = all.iter().map(|p| p.balance).fold(f64::NEG_INFINITY, f64::max);
        nice_domain(low, high, Baseline::Values)
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
                .data(balance_rows)
                .x(RowAccessor::key("minute"))
                .y(RowAccessor::key("balance"))
                .y_scale(move || Scale::Linear { domain: Some(balance_domain()), reverse: false })
                .x_scale(Scale::Linear { domain: None, reverse: false })
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .accessibility_label(t!("ui.chart.balance_label"))
                .h(px(BALANCE_HEIGHT))
                .ml(px(LABEL_GUTTER))
                .on_hover_change(move |index: &Option<u32>| hovered.set(*index))
                .cross_line_class("w-px h-full bg-border")
                .dot_class("size-2.5 rounded-full border-2 border-background bg-chart-1")
                .child(grid().stroke_color(move || theme().border))
                .child(area().curve(Curve::Linear).fill_color(move || theme().chart_1.opacity(0.14)))
                .child(line().curve(Curve::Linear).stroke_color(move || theme().chart_1).stroke_width(px(2.0)))
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
                        .label_class(AXIS_LABEL_CLASS),
                ),
        )
        .child(drawdown_panel(points, currency, generation, motion))
}

/// The drawdown from the peak under the balance: its title and deepest fall, and a plot that hangs from
/// zero (an area from the line up to 0, never down to the bottom of the plot).
fn drawdown_panel(
    points: impl Fn() -> Vec<EquityPoint> + Copy + 'static,
    currency: impl Fn() -> String + Copy + 'static,
    generation: impl Fn() -> u32 + Copy + 'static,
    motion: nsh_base::plot::PlotMotion,
) -> impl IntoElement {
    let max_drawdown = move || points().iter().map(|p| p.drawdown).fold(0.0_f64, f64::max);
    let rows = move || {
        let points = points();
        let x = minutes(&points);
        Rows::from_columns(
            points.len(),
            [("minute".into(), x), ("drawdown".into(), points.iter().map(|p| -p.drawdown).collect())],
        )
    };
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .justify_between()
                .ml(px(LABEL_GUTTER))
                .child(text("Drawdown").text_xs().text_color(move || theme().muted_foreground))
                .child(
                    text(move || {
                        t!(
                            "ui.chart.max_drawdown",
                            amount = format::money(-max_drawdown(), Sign::OnlyNegative, &currency())
                        )
                    })
                    .text_xs()
                    .font_family(tokens::mono)
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            plot()
                .data(rows)
                .x(RowAccessor::key("minute"))
                .y(RowAccessor::key("drawdown"))
                .y_scale(move || Scale::Linear {
                    domain: Some(nice_domain(-max_drawdown(), 0.0, Baseline::Zero)),
                    reverse: false,
                })
                .x_scale(Scale::Linear { domain: None, reverse: false })
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .interactive(false)
                .accessibility_label(t!("ui.chart.drawdown_label"))
                .h(px(DRAWDOWN_HEIGHT))
                .ml(px(LABEL_GUTTER))
                .child(
                    area().curve(Curve::Linear).y0(AreaY0::Value(0.0)).fill_color(move || tokens::loss().opacity(0.25)),
                )
                .child(line().curve(Curve::Linear).stroke_color(tokens::loss).stroke_width(px(1.5))),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_line_gets_room_and_round_ends() {
        let [low, high] = nice_domain(0.0, 0.0, Baseline::Zero);
        assert!(low < 0.0 && high > 0.0, "{low}..{high}");
        assert_eq!([low, high], [-1.0, 1.0]);
        let [low, high] = nice_domain(10_000.0, 10_000.0, Baseline::Values);
        assert!(low < 10_000.0 && high > 10_000.0, "a balance that never moved: {low}..{high}");
    }

    #[test]
    fn a_domain_holds_the_values_and_zero_and_ends_on_round_steps() {
        let [low, high] = nice_domain(0.4, 3.7, Baseline::Zero);
        assert_eq!([low, high], [0.0, 4.0], "zero is on the axis, the top rounds up to a step");
        let [low, high] = nice_domain(-1.3, 2.2, Baseline::Zero);
        assert_eq!([low, high], [-2.0, 3.0], "whole steps of one: six round labels");
        let [low, high] = nice_domain(9_870.0, 10_410.0, Baseline::Values);
        assert!(low > 0.0 && low <= 9_870.0 && high >= 10_410.0, "a balance does not stretch to zero: {low}..{high}");
    }

    #[test]
    fn a_drawdown_domain_hangs_from_zero() {
        let [low, high] = nice_domain(-340.0, 0.0, Baseline::Zero);
        assert!(low <= -340.0 && high.abs() < f64::EPSILON, "{low}..{high}");
        let [low, high] = nice_domain(0.0, 0.0, Baseline::Zero);
        assert!(low < high, "no drawdown still has an axis: {low}..{high}");
    }
}
