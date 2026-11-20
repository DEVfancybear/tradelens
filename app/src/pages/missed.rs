//! Lệnh bỏ lỡ: setup đúng nhưng không vào, và R đã bỏ lỡ.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] =
    &["Danh sách và form ghi lệnh bỏ lỡ (`core::missed`)", "Tổng hợp theo lý do, tách lệnh bị quy tắc chặn"];

pub(crate) fn missed() -> impl IntoElement {
    page(
        "Lệnh bỏ lỡ",
        "Setup đúng nhưng không vào — ghi lại để biết do dự tốn bao nhiêu.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
