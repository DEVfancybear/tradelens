//! The row of a discipline rule: its name and hint, the limit and a switch that turns the
//! rule on and off. Turning a rule off keeps the limit, so turning it back on restores it.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::forms::{NumberRange, number_field, setting_row, switch_control};

/// A rule with a limit. `value` is the limit while the rule is on and `None` while it is off;
/// `default` is the limit a rule that was never on starts with. `on_change` hears the new
/// limit, or `None` when the rule is turned off.
pub fn rule_row(
    label: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    unit: Option<SharedString>,
    range: NumberRange,
    default: f64,
    value: impl Fn() -> Option<f64> + Copy + 'static,
    on_change: impl Fn(Option<f64>) + Copy + 'static,
) -> impl IntoElement {
    let label: SharedString = label.into();
    // The last limit the rule had, so that switching off and on again gives it back.
    let kept = signal(default);
    effect(move || {
        if let Some(limit) = value() {
            kept.set(limit);
        }
    });
    let on = computed(move || value().is_some());
    let control = view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            // An emptied field is a limit being typed, not a rule turned off: only the switch turns it
            // off. Read as off, the field vanished under the trader's keys.
            when(on, {
                let (label, unit) = (label.clone(), unit);
                move || {
                    untracked(|| {
                        number_field(t!("ui.rule.value", label = label), unit.clone(), range, value, move |typed| {
                            if typed.is_some() {
                                on_change(typed);
                            }
                        })
                    })
                }
            })
            .otherwise(|| text(t!("ui.rule.off")).text_sm().text_color(move || theme().muted_foreground)),
        )
        .child(switch_control(
            label.clone(),
            move || on.get(),
            move |next| on_change(next.then(|| kept.get_untracked())),
        ));
    setting_row(label, Some(hint.into()), control)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nsh_core::Size;
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;

    #[component]
    fn Harness() -> impl IntoElement {
        super::super::theme::install();
        crate::i18n::install();
        let limit = signal(Some(300.0_f64));
        view()
            .gap_2()
            .child(rule_row(
                "Lỗ tối đa trong ngày",
                "Chạm ngưỡng thì cảnh báo",
                Some("USD".into()),
                NumberRange::between(0.0, 1_000_000.0).stepping(10.0),
                100.0,
                move || limit.get(),
                move |next| limit.set(next),
            ))
            .child(text(move || format!("limit {:?}", limit.get())))
    }

    fn start() -> Headless {
        let config =
            EngineConfig { viewport: Size::new(px(900.0), px(300.0)), ..EngineConfig::new(Root::of::<Harness>()) };
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("limit Some(300.0)").expect("the harness shows");
        app
    }

    #[test]
    fn turning_a_rule_off_and_on_gives_the_limit_back() {
        let mut app = start();
        app.press("label:Lỗ tối đa trong ngày").expect("the switch");
        app.wait_for_within("limit None", Duration::from_secs(1)).expect("the rule is off");
        assert!(app.screen().shows("Tắt"), "an off rule says so");
        app.press("label:Lỗ tối đa trong ngày").expect("the switch again");
        app.wait_for_within("limit Some(300.0)", Duration::from_secs(1)).expect("the limit came back");
    }
}
