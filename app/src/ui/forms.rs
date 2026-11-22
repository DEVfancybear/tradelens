//! Form parts: number field, switch, setting row, checklist and the bar of a discipline rule.
//! Built on `nsh_base`'s `number_input`, `switch`, `checkbox` and `progress`, which draw
//! nothing until styled here.

use nsh_base::{CheckboxElement, checkbox, number_input, progress, switch, theme};
use nsh_core::prelude::*;
use tradelens_core::ChecklistItem;

use super::{
    icons::{Icon, icon},
    theme::{self as tokens, mono},
};

/// How wide a number field is, in points.
const NUMBER_WIDTH: f32 = 148.0;

/// The limits and step of a [`number_field`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// What the − and + buttons and the arrow keys add (default 1).
    pub step: f64,
}

impl Default for NumberRange {
    fn default() -> Self {
        Self::any()
    }
}

impl NumberRange {
    /// No limits, step 1.
    pub const fn any() -> Self {
        Self { min: None, max: None, step: 1.0 }
    }

    /// From `min` to `max`, step 1.
    pub const fn between(min: f64, max: f64) -> Self {
        Self { min: Some(min), max: Some(max), step: 1.0 }
    }

    /// The same range stepping by `step`.
    #[must_use]
    pub const fn stepping(self, step: f64) -> Self {
        Self { step, ..self }
    }
}

/// A number typed in the field or stepped with − and +; `unit` (a currency, `%`, `lot`) sits
/// after the digits. `value` is `None` while the field is empty; `on_change` hears `None` the
/// same way. Typing is kept as the trader wrote it until the field is left, then held to `range`.
pub fn number_field(
    label: impl Into<SharedString>,
    unit: Option<&'static str>,
    range: NumberRange,
    value: impl Fn() -> Option<f64> + 'static,
    on_change: impl Fn(Option<f64>) + 'static,
) -> impl IntoElement {
    let step_button = |sign: &'static str| text(sign).text_sm().text_color(move || theme().muted_foreground);
    let step_style =
        |s: nsh_core::element::StyleRefinement| s.size(px(30.0)).items_center().justify_center().rounded(px(6.0));
    number_input()
        .value(value)
        .min(range.min)
        .max(range.max)
        .step(range.step)
        .accessibility_label(label.into())
        .on_change(move |next: &Option<f64>| on_change(*next))
        .decrement_button(step_button("\u{2212}"))
        .increment_button(step_button("+"))
        .decrement_button_style(step_style)
        .increment_button_style(step_style)
        .input_style(|s| {
            s.flex_1()
                .min_w(px(0.0))
                .h(px(30.0))
                .px_1()
                .text_sm()
                .font_family(mono)
                .tabular_nums()
                .text_color(move || theme().foreground)
        })
        .when_some(unit, |field, unit| {
            field.suffix(text(unit).pr_2().text_xs().text_color(move || theme().muted_foreground))
        })
        .w(px(NUMBER_WIDTH))
        .flex_row()
        .items_center()
        .px_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().card)
        .focus_within(|s| s.border_color(move || theme().ring))
}

/// A row of a settings list: the name and an optional hint at the left, the control at the right.
pub fn setting_row(label: &'static str, hint: Option<&'static str>, control: impl IntoElement) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .py_3()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(text(label).text_sm().font_weight(FontWeight::MEDIUM))
                .when_some(hint, |v, hint| v.child(text(hint).text_xs().text_color(move || theme().muted_foreground))),
        )
        .child(control)
}

/// A setting that is on or off, taking effect at once.
pub fn switch_row(
    label: &'static str,
    hint: Option<&'static str>,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    setting_row(label, hint, switch_control(label, checked, on_change))
}

/// The switch of a [`switch_row`]: a track and a thumb that slides along it.
fn switch_control(
    label: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .checked(checked)
        .on_change(move |next: &bool| on_change(*next))
        .accessibility_label(label)
        .hit_slop(EdgeInsets::all(px(6.0)))
        .rounded_full()
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .track_style(|s| {
            s.w(px(36.0))
                .h(px(20.0))
                .p(px(2.0))
                .rounded_full()
                .bg(move || theme().input)
                .checked(|s| s.bg(move || theme().primary))
        })
        .thumb_style(|s| {
            s.size(px(16.0))
                .rounded_full()
                .bg(move || theme().background)
                .checked(|s| s.translate_x(px(16.0)).bg(move || theme().primary_foreground))
        })
}

/// A list of things to confirm one by one (the checklist before an entry). `items` are the
/// trader's items, `checked` the ids of those confirmed, and `on_toggle` hears `(id, on)`.
/// Rows are made again only when the items change, never when the ticks do.
pub fn checklist(
    label: &'static str,
    items: impl Fn() -> Vec<ChecklistItem> + 'static,
    checked: impl Fn() -> Vec<i64> + Copy + 'static,
    on_toggle: impl Fn(i64, bool) + Copy + 'static,
) -> impl IntoElement {
    view().gap_2().child(dynamic(move || {
        let items = items();
        if items.is_empty() {
            return text("Chưa có mục nào trong checklist.")
                .text_sm()
                .text_color(move || theme().muted_foreground)
                .into_element();
        }
        view()
            .gap_2()
            .children(items.into_iter().map(|item| checklist_row(label, item, checked, on_toggle)))
            .into_element()
    }))
}

fn checklist_row(
    group: &'static str,
    item: ChecklistItem,
    checked: impl Fn() -> Vec<i64> + Copy + 'static,
    on_toggle: impl Fn(i64, bool) + Copy + 'static,
) -> impl IntoElement {
    let id = item.id;
    check_box(format!("{group}: {}", item.text), move || checked().contains(&id), move |next| on_toggle(id, next))
        .flex_row()
        .items_center()
        .gap_3()
        .child(text(item.text).text_sm().flex_1().min_w(px(0.0)))
}

/// A box that is ticked or not. Returns the checkbox element so that a caption can follow as
/// a child (`.child(text(..))`); `label` is what screen readers say and what tests press.
pub fn check_box(
    label: impl Into<SharedString>,
    checked: impl Fn() -> bool + Copy + 'static,
    on_change: impl Fn(bool) + 'static,
) -> CheckboxElement {
    let label: SharedString = label.into();
    checkbox()
        .checked(checked)
        .on_change(move |next: &bool| on_change(*next))
        .accessibility_label(label)
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .indicator_style(|s| {
            s.size(px(18.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().card)
                .checked(|s| s.bg(move || theme().primary).border_color(move || theme().primary))
        })
        .indicator(
            icon(Icon::Check, 14.0)
                .text_color(move || theme().primary_foreground)
                .opacity(move || if checked() { 1.0_f32 } else { 0.0 }),
        )
}

/// How near a discipline rule's limit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleLevel {
    /// Well within the limit.
    Within,
    /// Close to the limit: a warning.
    Near,
    /// At or past the limit.
    Over,
}

impl RuleLevel {
    /// From the share of the limit used (`used / limit`): a warning from 80 %, over from 100 %.
    pub fn of(ratio: f64) -> Self {
        if ratio >= 1.0 {
            Self::Over
        } else if ratio >= NEAR_LIMIT {
            Self::Near
        } else {
            Self::Within
        }
    }

    fn ink(self) -> AdaptiveColor {
        match self {
            Self::Within => theme().primary,
            Self::Near => tokens::warning(),
            Self::Over => tokens::loss(),
        }
    }
}

/// The share of a rule's limit from which it is flagged as near.
const NEAR_LIMIT: f64 = 0.8;

/// The bar of a rule with a limit (max daily loss, trades per day): its name, what is used
/// against the limit (`summary`, e.g. `−$18 / −$100`) and a thin bar filled by `ratio`
/// (`used / limit`; may pass 1). The bar and the summary turn amber near the limit and red
/// at it, and the state is also in the progress bar's accessible value.
pub fn progress_rule(
    label: &'static str,
    summary: impl Fn() -> String + 'static,
    ratio: impl Fn() -> f64 + Copy + 'static,
) -> impl IntoElement {
    let level = move || RuleLevel::of(ratio());
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(text(label).text_sm().font_weight(FontWeight::MEDIUM))
                .child(text(summary).text_xs().font_family(mono).tabular_nums().number_of_lines(1).text_color(
                    move || match level() {
                        RuleLevel::Within => theme().muted_foreground,
                        other => other.ink(),
                    },
                )),
        )
        .child(
            progress()
                .value(move || bar_percent(ratio()))
                .accessibility_label(label)
                .track_style(|s| s.h(px(6.0)).rounded_full().overflow_hidden().bg(move || theme().secondary))
                .indicator_style(|s| s.h_full().rounded_full().bg(move || level().ink())),
        )
}

/// The bar's fill, 0 to 100, for a ratio that may be below 0 or past 1.
fn bar_percent(ratio: f64) -> f32 {
    // The cast cannot lose what matters: the value is held to 0 – 100 first.
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "held to 0..=100 before the cast")]
    let percent = (ratio.clamp(0.0, 1.0) * 100.0) as f32;
    percent
}

#[cfg(test)]
mod tests {
    use nsh_core::Size;
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;

    #[test]
    fn a_rule_turns_amber_at_eighty_percent_and_red_at_the_limit() {
        assert_eq!(RuleLevel::of(0.0), RuleLevel::Within);
        assert_eq!(RuleLevel::of(0.79), RuleLevel::Within);
        assert_eq!(RuleLevel::of(0.8), RuleLevel::Near);
        assert_eq!(RuleLevel::of(0.99), RuleLevel::Near);
        assert_eq!(RuleLevel::of(1.0), RuleLevel::Over);
        assert_eq!(RuleLevel::of(2.5), RuleLevel::Over);
    }

    #[test]
    fn the_bar_is_held_between_empty_and_full() {
        assert!((bar_percent(-1.0)).abs() < f32::EPSILON);
        assert!((bar_percent(0.5) - 50.0).abs() < f32::EPSILON);
        assert!((bar_percent(3.0) - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn number_ranges_read_as_built() {
        assert_eq!(NumberRange::default(), NumberRange::any());
        let range = NumberRange::between(0.0, 100.0).stepping(0.5);
        assert_eq!((range.min, range.max, range.step), (Some(0.0), Some(100.0), 0.5));
    }

    #[component]
    fn Harness() -> impl IntoElement {
        super::super::theme::install();
        let ticked = signal(vec![2_i64]);
        let amount = signal(Some(1.5_f64));
        let on = signal(false);
        view()
            .gap_2()
            .child(checklist(
                "Checklist",
                || {
                    vec![
                        ChecklistItem { id: 1, text: "Cùng chiều trend H4".into() },
                        ChecklistItem { id: 2, text: "Có vùng cản rõ".into() },
                    ]
                },
                move || ticked.get(),
                move |id, now| {
                    ticked.update(|all| {
                        all.retain(|x| *x != id);
                        if now {
                            all.push(id);
                        }
                    });
                },
            ))
            .child(text(move || format!("ticked {:?}", ticked.get())))
            .child(number_field(
                "Risk",
                Some("%"),
                NumberRange::between(0.0, 10.0).stepping(0.5),
                move || amount.get(),
                move |v| {
                    amount.set(v);
                },
            ))
            .child(text(move || format!("risk {:?}", amount.get())))
            .child(switch_row("Nhắc ghi kế hoạch", Some("Khi EA báo lệnh mới"), move || on.get(), move |v| on.set(v)))
            .child(text(move || format!("switch {}", on.get())))
            .child(progress_rule("Lỗ tối đa trong ngày", || "−$80 / −$100".to_owned(), || 0.8))
    }

    fn start() -> Headless {
        let config =
            EngineConfig { viewport: Size::new(px(900.0), px(700.0)), ..EngineConfig::new(Root::of::<Harness>()) };
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("ticked [2]").expect("the harness shows");
        app
    }

    #[test]
    fn a_checklist_item_toggles_on_and_off() {
        let mut app = start();
        app.press("label:Checklist: Cùng chiều trend H4").expect("the item is a checkbox");
        app.wait_for("ticked [2, 1]").expect("ticking adds the id");
        app.press("label:Checklist: Có vùng cản rõ").expect("the second item");
        app.wait_for("ticked [1]").expect("pressing a ticked item clears it");
    }

    #[test]
    fn a_switch_flips_and_a_number_steps() {
        let mut app = start();
        app.press("label:Nhắc ghi kế hoạch").expect("the switch");
        app.wait_for("switch true").expect("the switch is on");
        app.type_text("label:Risk", "3").expect("typing in the number field");
        app.wait_for_within("text:risk Some(1.53)", std::time::Duration::from_secs(1))
            .expect("typed digits are reported");
    }
}
