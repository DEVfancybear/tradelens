//! Nhật ký ngày: Kế hoạch trước phiên và tổng kết cuối ngày.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &["Kế hoạch, tổng kết, tâm trạng theo ngày", "Lệnh của ngày bên cạnh, kèm vi phạm quy tắc"];

pub(crate) fn journal() -> impl IntoElement {
    page("Nhật ký ngày", "Kế hoạch trước phiên và tổng kết cuối ngày.", view().gap_6().child(planned(BLOCKS)))
}
