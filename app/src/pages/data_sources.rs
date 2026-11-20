//! Nguồn dữ liệu: Kết nối EA, script Python và import báo cáo.

use nsh_core::prelude::*;

use crate::ui::parts::{page, planned};

const BLOCKS: &[&str] = &[
    "Bridge: cổng, token (copy), trạng thái, nhật ký nhận dữ liệu",
    "Hướng dẫn cài EA TradeLensSync và script Python",
    "Import báo cáo MT5 (HTML, XLSX), file JSON",
    "Import Notion: file CSV với màn hình ghép cột, hoặc Notion API",
];

pub(crate) fn data_sources() -> impl IntoElement {
    page("Nguồn dữ liệu", "Kết nối EA, script Python và import báo cáo.", view().gap_6().child(planned(BLOCKS)))
}
