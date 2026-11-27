//! Charts of the analytics and the overview: upright bars with a signed colour (`bar_chart`,
//! `histogram`), a hour-by-weekday heatmap, bars that grow either way from a centre line
//! (`diverging_bars`) and the cost of one mistake (`mistake_cost_bar`).
//!
//! The upright bars are an `nsh_base::plot`; the heatmap and the diverging bars are views,
//! because each of their cells and rows carries text the trader reads and tests find.

use nsh_base::{
    button,
    plot::{BandScale, BarCornerRadii, Scale, bar, grid, plot},
    progress, theme,
};
use nsh_core::{
    element::{RowAccessor, Rows},
    prelude::*,
};

use super::{
    chips::Tone,
    parts::empty_state,
    theme::{self as tokens, loss, profit},
};

/// One bar of a [`bar_chart`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BarItem {
    /// Under the bar: its name (`T2`, `Phiên Á`).
    pub(crate) label: String,
    /// Under the name, smaller: a second line (`64% thắng`); may be empty.
    pub(crate) caption: String,
    /// Above the bar: the value as the trader reads it (`+$312.40`, `23`).
    pub(crate) shown: String,
    pub(crate) value: f64,
    /// The bar's colour.
    pub(crate) tone: Tone,
}

impl BarItem {
    /// A bar coloured by the sign of its value: green up, red down.
    pub(crate) fn signed(
        label: impl Into<String>,
        caption: impl Into<String>,
        shown: impl Into<String>,
        value: f64,
    ) -> Self {
        let tone = if value > 0.0 {
            Tone::Profit
        } else if value < 0.0 {
            Tone::Loss
        } else {
            Tone::Neutral
        };
        Self { label: label.into(), caption: caption.into(), shown: shown.into(), value, tone }
    }
}

fn tone_ink(tone: Tone) -> AdaptiveColor {
    match tone {
        Tone::Profit => profit(),
        Tone::Loss => loss(),
        Tone::Neutral | Tone::Primary => theme().muted_foreground,
    }
}

/// The value scale's two ends: zero and the extremes, never a flat range.
fn domain(items: &[BarItem]) -> [f64; 2] {
    let low = items.iter().map(|i| i.value).fold(0.0, f64::min);
    let high = items.iter().map(|i| i.value).fold(0.0, f64::max);
    if high - low < 1e-9 { [0.0, 1.0] } else { [low, high] }
}

/// The values of the bars of one tone, zero for the others: stacked, each bar shows once.
fn column(items: &[BarItem], tone: Tone) -> Vec<f64> {
    items
        .iter()
        .map(|item| {
            let own = match item.tone {
                Tone::Primary => Tone::Neutral,
                other => other,
            };
            if own == tone { item.value } else { 0.0 }
        })
        .collect()
}

fn bars_plot(items: &[BarItem], height: f32, label: &'static str) -> impl IntoElement + use<> {
    let rows = Rows::from_columns(
        items.len(),
        [
            ("profit".into(), column(items, Tone::Profit)),
            ("loss".into(), column(items, Tone::Loss)),
            ("flat".into(), column(items, Tone::Neutral)),
        ],
    );
    let mut band = BandScale::default();
    band.padding_inner = Percentage(0.4);
    band.padding_outer = Percentage(0.2);
    band.max_band_width = Some(px(48.0));
    let tip = BarCornerRadii::all(px(3.0));
    plot()
        .data(rows)
        .x_scale(Scale::Band(band))
        .y_scale(Scale::Linear { domain: Some(domain(items)), reverse: false })
        .interactive(false)
        .accessibility_label(label)
        .h(px(height))
        .child(grid().y(vec![0.0]).stroke_color(move || theme().border))
        .child(bar().value(RowAccessor::key("profit")).stack("net").corner_radii(tip).fill_color(profit))
        .child(bar().value(RowAccessor::key("loss")).stack("net").corner_radii(tip).fill_color(loss))
        .child(
            bar()
                .value(RowAccessor::key("flat"))
                .stack("net")
                .corner_radii(tip)
                .fill_color(move || theme().muted_foreground),
        )
}

/// Upright bars side by side, one per item: the value above, the bars between, then the name
/// and its caption. `items` is read again when it changes; `label` is what screen readers say
/// of the chart and `height` the bars' area.
pub(crate) fn bar_chart(
    items: impl Fn() -> Vec<BarItem> + 'static,
    label: &'static str,
    height: f32,
) -> impl IntoElement {
    dynamic(move || {
        let items = items();
        untracked(|| {
            if items.is_empty() {
                return empty_state("Chưa có dữ liệu", "Chọn khoảng thời gian dài hơn.").into_element();
            }
            let values = view().flex_row().children(items.iter().map(|item| {
                let tone = item.tone;
                text(item.shown.clone())
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_center()
                    .font_family(tokens::mono)
                    .tabular_nums()
                    .number_of_lines(1)
                    .text_color(move || tone_ink(tone))
            }));
            let names = view().flex_row().children(items.iter().map(|item| {
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .items_center()
                    .gap_0()
                    .child(text(item.label.clone()).text_xs().font_weight(FontWeight::MEDIUM).number_of_lines(1))
                    .child(
                        text(item.caption.clone())
                            .text_xs()
                            .number_of_lines(1)
                            .text_color(move || theme().muted_foreground),
                    )
            }));
            view().gap_2().child(values).child(bars_plot(&items, height, label)).child(names).into_element()
        })
    })
}

/// A histogram: bars of counts per bucket, the buckets below zero red and the others green.
/// `buckets` is `(name, count, below_zero)` in order.
pub(crate) fn histogram(
    buckets: impl Fn() -> Vec<(String, usize, bool)> + 'static,
    label: &'static str,
    height: f32,
) -> impl IntoElement {
    bar_chart(
        move || {
            buckets()
                .into_iter()
                .map(|(name, count, below_zero)| {
                    let value = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
                    BarItem {
                        label: name,
                        caption: String::new(),
                        shown: count.to_string(),
                        value,
                        tone: if below_zero { Tone::Loss } else { Tone::Profit },
                    }
                })
                .collect()
        },
        label,
        height,
    )
}

/// One cell of a [`heatmap`]: its value and what a screen reader says of it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeatValue {
    pub(crate) value: f64,
    pub(crate) description: String,
}

/// The rows and columns of a [`heatmap`]; `cells[row][column]` is `None` where nothing happened.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct HeatGrid {
    pub(crate) row_labels: Vec<String>,
    pub(crate) column_labels: Vec<String>,
    pub(crate) cells: Vec<Vec<Option<HeatValue>>>,
}

const HEAT_CELL_HEIGHT: f32 = 24.0;
const HEAT_LABEL_WIDTH: f32 = 28.0;
/// The weakest and the strongest opacity of a heat cell, so a small value still shows.
const HEAT_FLOOR: f32 = 0.18;
const HEAT_CEILING: f32 = 0.85;

/// How strong a cell of `value` is drawn when the strongest cell has `peak` (0 to 1 scaled
/// into the floor and the ceiling).
fn heat_strength(value: f64, peak: f64) -> f32 {
    if peak < 1e-9 {
        return HEAT_FLOOR;
    }
    let share = (value.abs() / peak).clamp(0.0, 1.0);
    // Held to 0..=1 above, so the narrowing loses nothing the eye can see.
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "held to 0..=1 before the cast")]
    let share = share as f32;
    HEAT_FLOOR + (HEAT_CEILING - HEAT_FLOOR) * share
}

/// A grid of cells coloured by their value: green for gains, red for losses, darker the
/// larger the result against the grid's strongest cell.
pub(crate) fn heatmap(grid: impl Fn() -> HeatGrid + 'static) -> impl IntoElement {
    dynamic(move || {
        let grid = grid();
        untracked(|| {
            if grid.cells.iter().flatten().all(Option::is_none) {
                return empty_state("Chưa có dữ liệu", "Chọn khoảng thời gian dài hơn.").into_element();
            }
            let peak = grid.cells.iter().flatten().flatten().map(|c| c.value.abs()).fold(0.0, f64::max);
            let header = view().flex_row().gap_1().child(view().w(px(HEAT_LABEL_WIDTH))).children(
                grid.column_labels.iter().map(|name| {
                    text(name.clone())
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_center()
                        .number_of_lines(1)
                        .text_color(move || theme().muted_foreground)
                }),
            );
            let body = grid.row_labels.iter().zip(&grid.cells).map(|(name, row)| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        text(name.clone())
                            .w(px(HEAT_LABEL_WIDTH))
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                    .children(row.iter().map(|cell| {
                        let slot = view().flex_1().min_w(px(0.0)).h(px(HEAT_CELL_HEIGHT)).rounded(px(4.0));
                        match cell {
                            Some(c) => {
                                let strength = heat_strength(c.value, peak);
                                let gain = c.value >= 0.0;
                                slot.accessible(true).label(c.description.clone()).bg(move || {
                                    if gain { profit().opacity(strength) } else { loss().opacity(strength) }
                                })
                            }
                            None => slot.bg(move || theme().secondary.opacity(0.5)),
                        }
                    }))
            });
            view().gap_1().child(header).children(body).into_element()
        })
    })
}

/// One row of [`diverging_bars`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DivergingItem {
    pub(crate) label: String,
    /// The value as the trader reads it, signed.
    pub(crate) shown: String,
    pub(crate) value: f64,
}

const DIVERGING_LABEL_WIDTH: f32 = 76.0;
const DIVERGING_VALUE_WIDTH: f32 = 96.0;
const DIVERGING_BAR_HEIGHT: f32 = 8.0;
/// The shortest a bar of a non-zero value is drawn, as a share of its half, so it shows.
const DIVERGING_MIN_SHARE: f64 = 0.04;

/// A half's two parts: the bar and the empty rest, as flex shares of 0 to 1.
fn share_of(value: f64, peak: f64) -> (f32, f32) {
    let share =
        if peak < 1e-9 || value.abs() < 1e-9 { 0.0 } else { (value.abs() / peak).clamp(DIVERGING_MIN_SHARE, 1.0) };
    // Held to 0..=1 above.
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "held to 0..=1 before the cast")]
    let bar = share as f32;
    (bar, 1.0 - bar)
}

/// Rows of a name, a bar that grows left from a centre line for a loss and right for a gain,
/// and the value; the longest bar fills its half. Pressing a row tells `on_pick` its name.
pub(crate) fn diverging_bars(
    items: impl Fn() -> Vec<DivergingItem> + 'static,
    on_pick: impl Fn(&str) + Copy + 'static,
) -> impl IntoElement {
    dynamic(move || {
        let items = items();
        untracked(|| {
            if items.is_empty() {
                return empty_state("Chưa có lệnh nào", "Kết quả theo symbol sẽ hiện ở đây.").into_element();
            }
            let peak = items.iter().map(|i| i.value.abs()).fold(0.0, f64::max);
            view()
                .gap_2()
                .children(items.iter().map(|item| {
                    let (bar_share, rest) = share_of(item.value, peak);
                    let gain = item.value >= 0.0;
                    let tone = if item.value > 0.0 {
                        Tone::Profit
                    } else if item.value < 0.0 {
                        Tone::Loss
                    } else {
                        Tone::Neutral
                    };
                    let fill = move || tone_ink(tone);
                    let negative_half =
                        view().flex_1().flex_row().child(view().flex_grow(if gain { 1.0 } else { rest })).child(
                            view()
                                .flex_grow(if gain { 0.0 } else { bar_share })
                                .h(px(DIVERGING_BAR_HEIGHT))
                                .rounded_l(px(3.0))
                                .bg(fill),
                        );
                    let positive_half = view()
                        .flex_1()
                        .flex_row()
                        .child(
                            view()
                                .flex_grow(if gain { bar_share } else { 0.0 })
                                .h(px(DIVERGING_BAR_HEIGHT))
                                .rounded_r(px(3.0))
                                .bg(fill),
                        )
                        .child(view().flex_grow(if gain { rest } else { 1.0 }));
                    let picked = item.label.clone();
                    button()
                        .cursor_pointer()
                        .on_press(move || on_pick(&picked))
                        .accessibility_label(format!("Xem lệnh {} {}", item.label, item.shown))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .p_1()
                        .rounded(move || theme().radius)
                        .hover(|s| s.bg(move || theme().accent))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(
                            text(item.label.clone())
                                .w(px(DIVERGING_LABEL_WIDTH))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(1),
                        )
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex_row()
                                .items_center()
                                .child(negative_half)
                                .child(view().w(px(1.0)).h(px(14.0)).bg(move || theme().border))
                                .child(positive_half),
                        )
                        .child(
                            text(item.shown.clone())
                                .w(px(DIVERGING_VALUE_WIDTH))
                                .text_sm()
                                .text_right()
                                .font_family(tokens::mono)
                                .tabular_nums()
                                .number_of_lines(1)
                                .text_color(fill),
                        )
                }))
                .into_element()
        })
    })
}

const COST_COUNT_WIDTH: f32 = 64.0;
const COST_VALUE_WIDTH: f32 = 96.0;

/// One mistake's cost: its name over a bar filled by its share of the money lost (0 to 1),
/// how many trades carried it, and what they lost in money and in R.
/// A [`mistake_cost_bar`] that opens the trades with the mistake when pressed.
pub(crate) fn mistake_cost_bar_pickable(
    name: &str,
    trades: usize,
    net: String,
    r: String,
    share: f64,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    mistake_cost_bar(name, trades, net, r, share).cursor_pointer().on_press(on_press)
}

pub(crate) fn mistake_cost_bar(name: &str, trades: usize, net: String, r: String, share: f64) -> View {
    // The share is held to 0..=100 before the narrowing.
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "held to 0..=100 before the cast")]
    let percent = (share.clamp(0.0, 1.0) * 100.0) as f32;
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .py_2()
        .child(
            view().flex_1().min_w(px(0.0)).gap_2().child(text(name.to_owned()).text_sm().number_of_lines(1)).child(
                progress()
                    .value(move || percent)
                    .accessibility_label(format!("Chi phí lỗi {name}"))
                    .track_style(|s| s.h(px(6.0)).rounded_full().overflow_hidden().bg(move || theme().secondary))
                    .indicator_style(|s| s.h_full().rounded_full().bg(loss)),
            ),
        )
        .child(
            text(format!("{trades} lệnh"))
                .w(px(COST_COUNT_WIDTH))
                .text_xs()
                .text_right()
                .font_family(tokens::mono)
                .tabular_nums()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .w(px(COST_VALUE_WIDTH))
                .items_end()
                .child(text(net).text_sm().font_family(tokens::mono).tabular_nums().text_color(loss))
                .child(
                    text(r)
                        .text_xs()
                        .font_family(tokens::mono)
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(value: f64) -> BarItem {
        BarItem::signed("x", "", "", value)
    }

    #[test]
    fn a_bar_is_coloured_by_the_sign_of_its_value() {
        assert_eq!(item(3.0).tone, Tone::Profit);
        assert_eq!(item(-3.0).tone, Tone::Loss);
        assert_eq!(item(0.0).tone, Tone::Neutral);
    }

    #[test]
    fn the_scale_always_holds_zero_and_never_collapses() {
        assert_eq!(domain(&[item(5.0), item(9.0)]), [0.0, 9.0]);
        assert_eq!(domain(&[item(-4.0), item(2.0)]), [-4.0, 2.0]);
        assert_eq!(domain(&[item(0.0)]), [0.0, 1.0]);
        assert_eq!(domain(&[]), [0.0, 1.0]);
    }

    #[test]
    fn stacked_columns_carry_each_bar_once() {
        let items = [item(5.0), item(-2.0), item(0.0)];
        assert_eq!(column(&items, Tone::Profit), [5.0, 0.0, 0.0]);
        assert_eq!(column(&items, Tone::Loss), [0.0, -2.0, 0.0]);
        assert_eq!(column(&items, Tone::Neutral), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn heat_strength_runs_from_the_floor_to_the_ceiling() {
        assert!((heat_strength(0.0, 100.0) - HEAT_FLOOR).abs() < f32::EPSILON);
        assert!((heat_strength(-100.0, 100.0) - HEAT_CEILING).abs() < f32::EPSILON);
        assert!((heat_strength(5.0, 0.0) - HEAT_FLOOR).abs() < f32::EPSILON, "no peak, no contrast");
    }

    #[test]
    fn a_small_value_still_gets_a_visible_bar() {
        let (bar, rest) = share_of(1.0, 1000.0);
        assert!((f64::from(bar) - DIVERGING_MIN_SHARE).abs() < 1e-6);
        assert!((bar + rest - 1.0).abs() < 1e-6);
        assert_eq!(share_of(0.0, 1000.0), (0.0, 1.0));
        assert_eq!(share_of(-1000.0, 1000.0), (1.0, 0.0));
    }
}
