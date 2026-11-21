//! Nguồn dữ liệu: Kết nối EA, script Python và import báo cáo.
//! Design: `docs/design/screens/data-sources.html` (v0.1: the bridge's state, port, token
//! and log; the install guide and the importers follow).

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{
    store::{BridgePhase, store},
    ui::{
        chips::figure_text,
        controls::{Variant, action},
        parts::{card, page, planned, property_row},
    },
};

const BLOCKS: &[&str] = &[
    "Hướng dẫn cài EA TradeLensSync và script Python",
    "Import báo cáo MT5 (HTML, XLSX), file JSON",
    "Import Notion: file CSV với màn hình ghép cột, hoặc Notion API",
];

pub(crate) fn data_sources() -> impl IntoElement {
    page(
        "Nguồn dữ liệu",
        "Kết nối EA, script Python và import báo cáo.",
        view().gap_6().child(bridge_card()).child(planned(BLOCKS)),
    )
}

/// The bridge the EA and the Python script send deals to: whether it listens, where, with
/// which token, and what it received.
fn bridge_card() -> impl IntoElement {
    let store = store();
    card(
        "Bridge MT5",
        view()
            .gap_3()
            .child(text(move || store.bridge().status_line()).text_sm().font_weight(FontWeight::MEDIUM))
            .child(dynamic(move || match store.bridge().phase {
                BridgePhase::Unavailable { reason } => {
                    Some(text(format!("Lý do: {reason}")).text_sm().text_color(move || theme().muted_foreground))
                }
                BridgePhase::Starting | BridgePhase::Listening { .. } => None,
            }))
            .child(property_row("Cổng", figure_text(move || store.bridge().port.to_string()).text_sm()))
            .child(property_row(
                "Token",
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(figure_text(move || store.bridge().token).text_sm())
                    .child(action("Sao chép", Variant::Secondary, move || set_clipboard(store.bridge().token))),
            ))
            .child(text("Nhật ký nhận dữ liệu").text_xs().text_color(move || theme().muted_foreground))
            .child(dynamic(move || {
                let lines = store.bridge().log;
                let empty = lines.is_empty();
                view()
                    .gap_1()
                    .when(empty, |v| {
                        v.child(
                            text("Chưa nhận dữ liệu nào từ EA hay script.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        )
                    })
                    .children(lines.into_iter().rev().map(|line| figure_text(move || line.clone()).text_xs()))
            })),
    )
}
