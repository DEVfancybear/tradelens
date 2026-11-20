//! Đăng nhập & sync: Đăng nhập Google hoặc Microsoft để sync lên Google Drive / OneDrive.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Nút Đăng nhập với Google / Microsoft (mở trình duyệt hệ thống)",
    "Tài khoản đang dùng, lần sync cuối, Sync ngay, tự động mỗi 15 phút",
    "Khôi phục sang máy mới; đăng xuất",
];

pub(crate) fn cloud() -> impl IntoElement {
    page(
        "Đăng nhập & sync",
        "Đăng nhập Google hoặc Microsoft để sync lên Google Drive / OneDrive.",
        view().gap_6().child(planned(BLOCKS)),
    )
}
