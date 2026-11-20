//! Lịch P/L: Lãi lỗ theo từng ngày, tô màu theo kết quả.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Lưới tháng, mỗi ô: Net, số lệnh, win rate; tổng theo tuần ở cột phải",
    "Chuyển tháng, chọn ngày để xem lệnh và nhật ký ngày",
];

pub(crate) fn calendar() -> impl IntoElement {
    page("Lịch P/L", "Lãi lỗ theo từng ngày, tô màu theo kết quả.", view().gap_6().child(planned(BLOCKS)))
}
