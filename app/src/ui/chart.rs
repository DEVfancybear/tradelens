//! The balance curve with its drawdown underneath (`nsh_base::plot`).
//!
//! Both plots place their points one trade apart (the design's "after 60 trades"), not by
//! time: trades bunched into the last days of a period would otherwise pile up at its right
//! edge.

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

/// The x scale of the curves: one place per point, evenly spaced, the first and last at the edges.
pub(crate) const TRADE_SPACING: Scale = Scale::Point { reverse: false };

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

/// The round steps an axis takes, as multiples of a power of ten.
const MANTISSAS: [f64; 4] = [1.0, 2.0, 2.5, 5.0];

/// [`MANTISSAS`] without 2.5, for steps below one: 0.25 would need a second decimal that an R label
/// (`+0.3R`) does not show.
const FRACTION_MANTISSAS: [f64; 3] = [1.0, 2.0, 5.0];

/// The most steps between an axis's ends: few enough that the labels of the 220 pt balance plot
/// stay well apart, enough that the values fill most of its height.
const MAX_STEPS: i32 = 6;

/// Room left beyond the values at each end, as a share of their spread, so that the line never
/// runs along the plot's edge.
const PADDING: f64 = 0.05;

/// A value axis: the domain of its scale and the round values its labels sit on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValueAxis {
    /// The values at the bottom and top of the plot.
    pub(crate) domain: [f64; 2],
    /// The labelled values, from the bottom up, both ends included.
    pub(crate) ticks: Vec<f64>,
}

/// The axis for values from `low` to `high`. It fits the values, not zero: a balance near a million
/// that moves by a few thousand fills the plot. It holds them with a little room at each end (an end
/// at zero stays there), reaches zero with [`Baseline::Zero`], is never flat, and its ends fall on
/// round steps (1, 2, 2.5 or 5 times a power of ten), at most [`MAX_STEPS`] of them. A line of one
/// value (a balance that never moved, results of zero) gets room above and below instead of a single
/// label on a squashed axis. Values that are not finite (no points) read as zero.
pub(crate) fn value_axis(low: f64, high: f64, baseline: Baseline) -> ValueAxis {
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let (low, high) = (finite(low), finite(high));
    let (low, high) = (low.min(high), low.max(high));
    let room = (high - low) * PADDING;
    let away = |value: f64, by: f64| if value.abs() < 1e-9 { value } else { value + by };
    let (mut low, mut high) = (away(low, -room), away(high, room));
    if baseline == Baseline::Zero {
        low = low.min(0.0);
        high = high.max(0.0);
    }
    if high - low < 1e-9 {
        let room = (low.abs() * 0.05).max(1.0);
        low -= room;
        high += room;
    }
    let (step, domain) = round_ends(low, high);
    let ticks = (0..=MAX_STEPS)
        .map(|i| domain[0] + f64::from(i) * step)
        .take_while(|v| *v <= domain[1] + step * 1e-6)
        .collect();
    ValueAxis { domain, ticks }
}

/// The smallest round step that lays `low..high` (`low < high`) out in at most [`MAX_STEPS`]
/// whole steps, and the ends it rounds them out to.
fn round_ends(low: f64, high: f64) -> (f64, [f64; 2]) {
    let most = f64::from(MAX_STEPS);
    let mut power = 10.0_f64.powf(((high - low) / most).log10().floor());
    loop {
        let mantissas: &[f64] = if power >= 1.0 { &MANTISSAS } else { &FRACTION_MANTISSAS };
        for &mantissa in mantissas {
            let step = mantissa * power;
            let ends = [(low / step).floor() * step, (high / step).ceil() * step];
            if (ends[1] - ends[0]) / step <= most + 1e-9 {
                return (step, ends);
            }
        }
        power *= 10.0;
    }
}

/// An axis label of money: whole units on whole values, cents on the steps of a curve that
/// moved by less than a unit.
fn money_label(value: f64, currency: &str) -> String {
    if value.fract().abs() < 1e-9 {
        format::money_whole(value, Sign::OnlyNegative, currency)
    } else {
        format::money(value, Sign::OnlyNegative, currency)
    }
}

/// The axis of the balance line: it fits the balances shown.
fn balance_axis(points: &[EquityPoint]) -> ValueAxis {
    let low = points.iter().map(|p| p.balance).fold(f64::INFINITY, f64::min);
    let high = points.iter().map(|p| p.balance).fold(f64::NEG_INFINITY, f64::max);
    value_axis(low, high, Baseline::Values)
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
        Rows::from_columns(points.len(), [("balance".into(), points.iter().map(|p| p.balance).collect())])
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
    let axis = computed(move || balance_axis(&points()));

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
                .y(RowAccessor::key("balance"))
                .y_scale(move || Scale::Linear { domain: Some(axis.with(|a| a.domain)), reverse: false })
                .x_scale(TRADE_SPACING)
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .accessibility_label("Đường số dư theo từng lệnh")
                .h(px(BALANCE_HEIGHT))
                .ml(px(LABEL_GUTTER))
                .on_hover_change(move |index: &Option<u32>| hovered.set(*index))
                .cross_line_class("w-px h-full bg-border")
                .dot_class("size-2.5 rounded-full border-2 border-background bg-chart-1")
                .child(grid().y(move || axis.with(|a| a.ticks.clone())).stroke_color(move || theme().border))
                // Without a `y0` the area closes down to the bottom of the plot, the axis's lowest
                // value, never to a zero far below it.
                .child(area().curve(Curve::Linear).fill_color(move || theme().chart_1.opacity(0.14)))
                .child(line().curve(Curve::Linear).stroke_color(move || theme().chart_1).stroke_width(px(2.0)))
                .child(
                    plot_axis()
                        .placement(AxisPlacement::Left)
                        .tick_values(move || axis.with(|a| a.ticks.clone()))
                        .format(TickFormat::from_fn(move |value, _| money_label(value, &currency()).into()))
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
        Rows::from_columns(points.len(), [("drawdown".into(), points.iter().map(|p| -p.drawdown).collect())])
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
                    text(move || format!("tối đa {}", format::money(-max_drawdown(), Sign::OnlyNegative, &currency())))
                        .text_xs()
                        .font_family(tokens::mono)
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            plot()
                .data(rows)
                .y(RowAccessor::key("drawdown"))
                .y_scale(move || Scale::Linear {
                    domain: Some(value_axis(-max_drawdown(), 0.0, Baseline::Zero).domain),
                    reverse: false,
                })
                .x_scale(TRADE_SPACING)
                .motion(motion)
                .appear_generation(move || AppearKey::from(generation()))
                .interactive(false)
                .accessibility_label("Drawdown từ đỉnh số dư")
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

    fn domain(low: f64, high: f64, baseline: Baseline) -> [f64; 2] {
        value_axis(low, high, baseline).domain
    }

    fn balances(values: &[f64]) -> Vec<EquityPoint> {
        values
            .iter()
            .zip(0_i64..)
            .map(|(&balance, i)| EquityPoint { time_ms: i, balance, peak: balance, drawdown: 0.0 })
            .collect()
    }

    #[test]
    fn a_flat_line_gets_room_and_round_ends() {
        let [low, high] = domain(0.0, 0.0, Baseline::Zero);
        assert!(low < 0.0 && high > 0.0, "{low}..{high}");
        assert_eq!([low, high], [-1.0, 1.0]);
        let [low, high] = domain(10_000.0, 10_000.0, Baseline::Values);
        assert!(low < 10_000.0 && high > 10_000.0, "a balance that never moved: {low}..{high}");
    }

    #[test]
    fn a_domain_holds_the_values_and_zero_and_ends_on_round_steps() {
        let [low, high] = domain(0.4, 3.7, Baseline::Zero);
        assert_eq!([low, high], [0.0, 4.0], "zero is on the axis, the top rounds up to a step");
        let [low, high] = domain(-1.3, 2.2, Baseline::Zero);
        assert_eq!([low, high], [-2.0, 3.0], "whole steps of one: six round labels");
        let [low, high] = domain(9_870.0, 10_410.0, Baseline::Values);
        assert!(low > 0.0 && low <= 9_870.0 && high >= 10_410.0, "a balance does not stretch to zero: {low}..{high}");
    }

    #[test]
    fn a_drawdown_domain_hangs_from_zero() {
        let [low, high] = domain(-340.0, 0.0, Baseline::Zero);
        assert!(low <= -340.0 && high.abs() < f64::EPSILON, "{low}..{high}");
        let [low, high] = domain(-4_224.22, 0.0, Baseline::Zero);
        assert!((-8_000.0..=-4_224.22).contains(&low) && high.abs() < f64::EPSILON, "{low}..{high}");
        let [low, high] = domain(0.0, 0.0, Baseline::Zero);
        assert!(low < high, "no drawdown still has an axis: {low}..{high}");
    }

    #[test]
    fn a_balance_near_a_million_fills_the_plot_instead_of_starting_at_zero() {
        let axis = balance_axis(&balances(&[1_000_000.0, 997_800.0, 1_002_100.0]));
        let [low, high] = axis.domain;
        assert!(low > 900_000.0, "the axis does not reach down to zero: {low}..{high}");
        assert!(low < 997_800.0 && high > 1_002_100.0, "the values have room at both ends: {low}..{high}");
        assert!((1_002_100.0 - 997_800.0) / (high - low) >= 0.5, "the values fill most of the height: {low}..{high}");

        let ticks = &axis.ticks;
        assert!((3..=7).contains(&ticks.len()), "{ticks:?}");
        assert_eq!(ticks.first().copied(), Some(low), "{ticks:?}");
        assert_eq!(ticks.last().copied(), Some(high), "{ticks:?}");
        assert!(ticks.contains(&1_000_000.0), "a round label at the deposit: {ticks:?}");
        let step = ticks[1] - ticks[0];
        assert!(ticks.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-6), "evenly spaced: {ticks:?}");
        assert!(ticks.iter().all(|t| (t / step).fract().abs() < 1e-9), "round values: {ticks:?}");
        let labels: Vec<String> = ticks.iter().map(|&t| money_label(t, "USD")).collect();
        assert!(labels.contains(&"$1,000,000".to_owned()), "{labels:?}");
    }

    #[test]
    fn values_on_round_steps_do_not_touch_the_edges() {
        let [low, high] = domain(1_000.0, 2_000.0, Baseline::Values);
        assert!(low < 1_000.0 && high > 2_000.0, "{low}..{high}");
        assert!(1_000.0 / (high - low) >= 0.5, "{low}..{high}");
    }

    #[test]
    fn small_steps_need_no_more_than_one_decimal() {
        let axis = value_axis(0.0, 1.2, Baseline::Zero);
        assert!(axis.ticks.iter().all(|t| (t * 10.0 - (t * 10.0).round()).abs() < 1e-6), "{axis:?}");
    }

    #[test]
    fn no_points_still_give_a_finite_axis() {
        let axis = balance_axis(&[]);
        assert!(axis.domain.iter().all(|v| v.is_finite()) && axis.domain[0] < axis.domain[1], "{axis:?}");
        assert!(axis.ticks.len() >= 2, "{axis:?}");
    }

    #[test]
    fn money_labels_keep_the_currency_and_show_cents_only_on_fractional_steps() {
        assert_eq!(money_label(996_000.0, "USD"), "$996,000");
        assert_eq!(money_label(996_000.0, "EUR"), "996,000 EUR");
        assert_eq!(money_label(10.5, "USD"), "$10.50");
    }
}
