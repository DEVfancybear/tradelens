//! Cài đặt: Giao diện, múi giờ và dữ liệu.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Sáng / tối / theo hệ thống",
    "Múi giờ hiển thị (mặc định UTC+7), độ lệch giờ server theo tài khoản",
    "Danh mục setup, tag, lỗi, cảm xúc",
    "Vị trí file dữ liệu, sao lưu",
];

pub(crate) fn settings() -> impl IntoElement {
    page("Cài đặt", "Giao diện, múi giờ và dữ liệu.", view().gap_6().child(planned(BLOCKS)))
}
