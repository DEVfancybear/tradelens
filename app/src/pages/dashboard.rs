//! Tổng quan: Kết quả giao dịch và kỷ luật của tài khoản đang chọn.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned, stat_tile};

const BLOCKS: &[&str] = &[
    "Hàng chỉ số: Net P/L, Win rate, Profit Factor, Expectancy, Max DD, R trung bình",
    "Đường balance và drawdown theo thời gian (nsh_base::plot: Area + Line)",
    "Lịch P/L thu nhỏ của tháng này",
    "Kỷ luật hôm nay: lỗ trong ngày, số lệnh, vi phạm",
    "5 lệnh gần nhất, bấm để mở nhật ký",
];

pub(crate) fn dashboard() -> impl IntoElement {
    page(
        "Tổng quan",
        "Kết quả giao dịch và kỷ luật của tài khoản đang chọn.",
        view().gap_6().child(stat_row()).child(planned(BLOCKS)),
    )
}

/// The headline numbers, empty until the store serves statistics.
fn stat_row() -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(stat_tile("Net P/L", "—"))
        .child(stat_tile("Win rate", "—"))
        .child(stat_tile("Profit Factor", "—"))
        .child(stat_tile("Max drawdown", "—"))
}
