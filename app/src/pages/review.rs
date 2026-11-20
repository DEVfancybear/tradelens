//! Tổng kết & xem lại: câu hỏi tổng kết tuần/tháng, hàng chờ xem lại, tóm tắt AI.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Số liệu kỳ và câu hỏi theo mẫu (`core::review`)",
    "Hàng chờ: lệnh chưa ghi nhật ký, đánh dấu xem lại, lỗ quá 1R chưa gắn lỗi",
    "Tóm tắt AI (tắt mặc định) và thẻ chia sẻ",
];

pub(crate) fn review() -> impl IntoElement {
    page(
        "Tổng kết & xem lại",
        "Nhìn lại tuần/tháng, trả lời câu hỏi tổng kết và dọn hàng chờ xem lại.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
