//! Playbook: quy tắc, checklist, ảnh mẫu và số liệu từng setup.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Danh sách playbook kèm kỳ vọng R (`core::playbook`)",
    "Quy tắc vào/thoát/huỷ, checklist, ảnh mẫu, thống kê theo hạng setup",
];

pub(crate) fn playbook() -> impl IntoElement {
    page(
        "Playbook",
        "Mỗi setup một bộ quy tắc, checklist và ảnh mẫu, kèm số liệu thật.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
