//! Lệnh: Danh sách lệnh đã gom từ deal MT5, lọc và mở nhật ký từng lệnh.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Bộ lọc: khoảng ngày, symbol, Buy/Sell, thắng/thua, setup, tag",
    "Bảng lệnh ảo hoá (data_table): giờ vào/ra, symbol, chiều, lot, giá, SL/TP, Net, R",
    "Trang chi tiết lệnh: setup, tag, lỗi, cảm xúc, sao 1–5, checklist, ghi chú, ảnh chart",
    "Thêm lệnh tay; xuất CSV",
];

pub(crate) fn trades() -> impl IntoElement {
    page(
        "Lệnh",
        "Danh sách lệnh đã gom từ deal MT5, lọc và mở nhật ký từng lệnh.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
