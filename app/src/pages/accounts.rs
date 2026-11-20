//! Tài khoản & kỷ luật: Các tài khoản MT5 và quy tắc của từng tài khoản.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Danh sách tài khoản: tên, broker, server, tiền tệ, balance, lần sync cuối",
    "Quy tắc: lỗ tối đa/ngày, số lệnh/ngày, risk %/lệnh, thua liên tiếp, mục tiêu ngày",
    "Checklist trước lệnh; danh sách vi phạm",
];

pub(crate) fn accounts() -> impl IntoElement {
    page(
        "Tài khoản & kỷ luật",
        "Các tài khoản MT5 và quy tắc của từng tài khoản.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
