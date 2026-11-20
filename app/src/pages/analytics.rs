//! Phân tích: Tìm lợi thế: kết quả theo symbol, thời gian, setup và lỗi.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Theo symbol, thứ trong tuần, giờ vào lệnh, phiên Á/Âu/Mỹ",
    "Theo setup, tag, lỗi, cảm xúc; Buy so với Sell",
    "Phân phối R; heatmap giờ × thứ (v0.2)",
];

pub(crate) fn analytics() -> impl IntoElement {
    page(
        "Phân tích",
        "Tìm lợi thế: kết quả theo symbol, thời gian, setup và lỗi.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
