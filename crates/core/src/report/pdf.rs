//! The monthly report as an A4 PDF. The report is first laid out as plain drawing operations,
//! page by page (so the layout is testable without a PDF reader), then each page is drawn with
//! `krilla`, which embeds subsets of the bundled fonts and keeps the text selectable.
//!
//! The page is white and printable: it uses the light tokens of the design, not the dark theme.

use std::{collections::HashMap, path::Path};

use chrono::{Datelike, NaiveDate};
use krilla::{
    Data, Document,
    color::rgb,
    geom::{PathBuilder, Point, Rect},
    metadata::Metadata,
    num::NormalizedF32,
    page::PageSettings,
    paint::{Fill, FillRule, Stroke},
    surface::Surface,
    text::{Font, TextDirection},
};

use super::{
    MonthlyReport, Outcome, Privacy, ReportError,
    fonts::{FontFace, Metrics},
    money, money_whole, percent, r_text,
};
use crate::{metrics::CalendarWeek, review::Period};

const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 40.0;
/// Where the flow starts on a continuation page, below the running header.
const TOP: f32 = 64.0;
/// Where the flow must stop: above the footer.
const BOTTOM: f32 = PAGE_HEIGHT - 52.0;
const CONTENT_WIDTH: f32 = PAGE_WIDTH - 2.0 * MARGIN;

// The light tokens of the design (`docs/design/README.md`): a printed page is always light.
const INK: Rgb = Rgb(0x0f, 0x17, 0x2a);
const MUTED: Rgb = Rgb(0x5b, 0x64, 0x75);
const RULE: Rgb = Rgb(0xe3, 0xe7, 0xee);
const PANEL: Rgb = Rgb(0xf6, 0xf7, 0xf9);
const PRIMARY: Rgb = Rgb(0x0d, 0x94, 0x88);
const PRIMARY_FOREGROUND: Rgb = Rgb(0xff, 0xff, 0xff);
const PROFIT: Rgb = Rgb(0x05, 0x96, 0x69);
const LOSS: Rgb = Rgb(0xdc, 0x26, 0x26);
const PROFIT_SOFT: Rgb = Rgb(0xd1, 0xfa, 0xe5);
const LOSS_SOFT: Rgb = Rgb(0xfe, 0xe2, 0xe2);

const BODY: f32 = 9.5;
const LEADING: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

/// One thing to draw. Text is left-aligned at its baseline; the layout works out alignment.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Text {
        x: f32,
        y: f32,
        size: f32,
        face: FontFace,
        color: Rgb,
        text: String,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: Option<Rgb>,
        stroke: Option<(Rgb, f32)>,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        color: Rgb,
        width: f32,
    },
    Poly {
        points: Vec<(f32, f32)>,
        color: Rgb,
        width: f32,
    },
    /// The region between a polyline and the line `y = base`.
    Area {
        points: Vec<(f32, f32)>,
        base: f32,
        color: Rgb,
        opacity: f32,
    },
}

/// A count as a float for layout arithmetic; the counts here are tiny.
fn count(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

fn tone(value: f64) -> Rgb {
    match Outcome::of(value) {
        Outcome::Win => PROFIT,
        Outcome::Loss => LOSS,
        Outcome::Flat => INK,
    }
}

/// A column of a table: its title, its width and which side its text hugs.
struct Column {
    title: &'static str,
    width: f32,
    right: bool,
}

/// A cell: its text and the colour it is set in.
type Cell = (String, Rgb);

fn plain(text: impl Into<String>) -> Cell {
    (text.into(), INK)
}

/// The pages being laid out, and where the next line goes.
struct Layout<'m> {
    metrics: &'m Metrics,
    privacy: Privacy,
    currency: String,
    pages: Vec<Vec<Op>>,
    y: f32,
}

impl<'m> Layout<'m> {
    fn new(metrics: &'m Metrics, privacy: Privacy, currency: &str) -> Self {
        Self { metrics, privacy, currency: currency.to_owned(), pages: vec![Vec::new()], y: MARGIN }
    }

    fn ops(&mut self) -> &mut Vec<Op> {
        self.pages.last_mut().expect("a layout always has a page")
    }

    fn push(&mut self, op: Op) {
        self.ops().push(op);
    }

    /// Starts a continuation page when `height` more points do not fit on this one.
    fn ensure(&mut self, height: f32) {
        if self.y + height > BOTTOM {
            self.pages.push(Vec::new());
            self.y = TOP;
        }
    }

    fn text(&mut self, x: f32, y: f32, size: f32, face: FontFace, color: Rgb, text: &str) {
        self.push(Op::Text { x, y, size, face, color, text: text.to_owned() });
    }

    fn width(&self, face: FontFace, size: f32, text: &str) -> f32 {
        self.metrics.width(face, size, text)
    }

    /// Text whose right edge is at `right`.
    fn text_right(&mut self, right: f32, y: f32, size: f32, face: FontFace, color: Rgb, text: &str) {
        let x = right - self.width(face, size, text);
        self.text(x, y, size, face, color, text);
    }

    /// Text centred on `middle`.
    fn text_centre(&mut self, middle: f32, y: f32, size: f32, face: FontFace, color: Rgb, text: &str) {
        let x = middle - self.width(face, size, text) / 2.0;
        self.text(x, y, size, face, color, text);
    }

    /// `text` cut with an ellipsis so that it is at most `max` wide.
    fn clipped(&self, face: FontFace, size: f32, max: f32, text: &str) -> String {
        if self.width(face, size, text) <= max {
            return text.to_owned();
        }
        let mut kept = String::new();
        for c in text.chars() {
            let candidate = format!("{kept}{c}…");
            if self.width(face, size, &candidate) > max {
                break;
            }
            kept.push(c);
        }
        format!("{}…", kept.trim_end())
    }

    /// `text` broken into lines of at most `width`, at spaces where it can be.
    fn wrap(&self, face: FontFace, size: f32, width: f32, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.lines() {
            let mut line = String::new();
            for word in paragraph.split_whitespace() {
                let joined = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
                if self.width(face, size, &joined) <= width {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                // A word wider than the line is broken where it overflows.
                let mut rest = word;
                while self.width(face, size, rest) > width {
                    let mut cut = 0;
                    for (i, c) in rest.char_indices() {
                        if self.width(face, size, &rest[..i + c.len_utf8()]) > width {
                            break;
                        }
                        cut = i + c.len_utf8();
                    }
                    let cut = if cut == 0 { rest.chars().next().map_or(rest.len(), char::len_utf8) } else { cut };
                    lines.push(rest[..cut].to_owned());
                    rest = &rest[cut..];
                }
                rest.clone_into(&mut line);
            }
            lines.push(line);
        }
        lines
    }

    /// A paragraph, line by line, flowing onto the next page when it must.
    fn paragraph(&mut self, face: FontFace, size: f32, color: Rgb, text: &str) {
        for line in self.wrap(face, size, CONTENT_WIDTH, text) {
            self.ensure(LEADING);
            let y = self.y + size;
            self.text(MARGIN, y, size, face, color, &line);
            self.y += LEADING;
        }
    }

    fn note(&mut self, text: &str) {
        self.paragraph(FontFace::Regular, BODY, MUTED, text);
        self.y += 6.0;
    }

    /// A section title with a rule under it; kept with the first lines that follow.
    fn heading(&mut self, title: &str) {
        self.ensure(70.0);
        self.y += 10.0;
        let baseline = self.y + 12.0;
        self.text(MARGIN, baseline, 12.0, FontFace::SemiBold, INK, title);
        self.push(Op::Line {
            from: (MARGIN, baseline + 6.0),
            to: (MARGIN + CONTENT_WIDTH, baseline + 6.0),
            color: RULE,
            width: 0.75,
        });
        self.y = baseline + 14.0;
    }

    /// A table: the header, then the rows (a row never splits across pages).
    fn table(&mut self, columns: &[Column], rows: &[Vec<Cell>]) {
        const ROW: f32 = 20.0;
        const SIZE: f32 = 9.0;
        let left_of = |index: usize| MARGIN + columns[..index].iter().map(|c| c.width).sum::<f32>();
        let draw = |layout: &mut Self, cells: &[(String, Rgb)], face: FontFace, y: f32| {
            for (i, (column, (text, color))) in columns.iter().zip(cells).enumerate() {
                let left = left_of(i);
                let shown = layout.clipped(face, SIZE, column.width - 8.0, text);
                if column.right {
                    layout.text_right(left + column.width - 4.0, y, SIZE, face, *color, &shown);
                } else {
                    layout.text(left + 4.0, y, SIZE, face, *color, &shown);
                }
            }
        };
        let head: Vec<Cell> = columns.iter().map(|c| (c.title.to_owned(), MUTED)).collect();
        self.ensure(ROW * 2.0);
        let baseline = self.y + 13.0;
        draw(self, &head, FontFace::SemiBold, baseline);
        self.y += ROW;
        for row in rows {
            self.ensure(ROW);
            let top = self.y;
            self.push(Op::Line { from: (MARGIN, top), to: (MARGIN + CONTENT_WIDTH, top), color: RULE, width: 0.5 });
            draw(self, row, FontFace::Regular, top + 13.0);
            self.y += ROW;
        }
        self.y += 6.0;
    }

    fn rect(&mut self, (x, y, w, h): (f32, f32, f32, f32), radius: f32, fill: Option<Rgb>, stroke: Option<Rgb>) {
        self.push(Op::Rect { x, y, w, h, radius, fill, stroke: stroke.map(|c| (c, 0.75)) });
    }

    /// The top of the first page: the brand, the title and the headline figures.
    fn cover(&mut self, report: &MonthlyReport) {
        self.rect((MARGIN, MARGIN, 20.0, 20.0), 5.0, Some(PRIMARY), None);
        self.text(MARGIN + 6.0, MARGIN + 14.5, 12.0, FontFace::Bold, PRIMARY_FOREGROUND, "T");
        self.text(MARGIN + 28.0, MARGIN + 15.0, 12.0, FontFace::SemiBold, INK, "TradeLens");
        let title = format!("Báo cáo {}", Period::Month.title(report.month).to_lowercase());
        self.text(MARGIN, MARGIN + 62.0, 26.0, FontFace::Bold, INK, &title);
        let who = if report.account.is_empty() { "Tất cả tài khoản" } else { report.account.as_str() };
        let currency = if report.currency.is_empty() { String::new() } else { format!(" · {}", report.currency) };
        self.text(MARGIN, MARGIN + 82.0, 11.0, FontFace::Regular, MUTED, &format!("{who}{currency}"));
        self.y = MARGIN + 100.0;
        self.kpis(report);
    }

    fn kpis(&mut self, report: &MonthlyReport) {
        const TILE_HEIGHT: f32 = 62.0;
        const GAP: f32 = 12.0;
        let s = &report.stats;
        let currency = self.currency.clone();
        let result = match self.privacy {
            Privacy::ShowMoney => (
                money(s.net, &currency),
                tone(s.net),
                (s.r_trades > 0).then(|| format!("{} trên {} lệnh có R", r_text(s.total_r, 1), s.r_trades)),
            ),
            Privacy::OnlyR => (
                if s.r_trades > 0 { r_text(s.total_r, 1) } else { "—".to_owned() },
                tone(s.total_r),
                s.avg_r.map(|r| format!("kỳ vọng {}/lệnh", r_text(r, 2))),
            ),
            Privacy::NoFigures => {
                (Outcome::of(s.net).label().to_owned(), tone(s.net), Some("đã ẩn số liệu".to_owned()))
            }
        };
        let drawdown = match (self.privacy, s.max_drawdown_pct) {
            (_, None) => ("—".to_owned(), None),
            (Privacy::ShowMoney, Some(pct)) => {
                (format!("{:.1}%", pct * 100.0), Some(money(-s.max_drawdown, &currency)))
            }
            (_, Some(pct)) => (format!("{:.1}%", pct * 100.0), None),
        };
        let tiles: [(&str, String, Rgb, Option<String>); 6] = [
            ("Kết quả", result.0, result.1, result.2),
            (
                "Tỷ lệ thắng",
                format!("{:.1}%", s.win_rate * 100.0),
                INK,
                Some(format!("{} thắng · {} thua · {} hòa", s.wins, s.losses, s.breakeven)),
            ),
            (
                "Profit factor",
                s.profit_factor.map_or_else(|| "—".to_owned(), |p| format!("{p:.2}")),
                INK,
                s.payoff.map(|p| format!("thắng/thua trung bình {p:.2}")),
            ),
            ("Số lệnh", s.trades.to_string(), INK, Some(format!("{:.2} lot", s.volume))),
            (
                "Drawdown tối đa",
                drawdown.0,
                if drawdown.1.is_some() || s.max_drawdown_pct.is_some() { LOSS } else { INK },
                drawdown.1,
            ),
            (
                "Ngày xanh",
                format!("{} / {}", s.green_days, s.trading_days),
                INK,
                Some(format!("chuỗi thắng {} · chuỗi thua {}", s.max_win_streak, s.max_loss_streak)),
            ),
        ];
        let tile_width = (CONTENT_WIDTH - 2.0 * GAP) / 3.0;
        for (i, (label, value, color, sub)) in tiles.into_iter().enumerate() {
            let (col, row) = (count(i % 3), count(i / 3));
            let (x, y) = (MARGIN + col * (tile_width + GAP), self.y + row * (TILE_HEIGHT + GAP));
            self.rect((x, y, tile_width, TILE_HEIGHT), 6.0, Some(PANEL), Some(RULE));
            self.text(x + 12.0, y + 18.0, 8.5, FontFace::Regular, MUTED, label);
            let value = self.clipped(FontFace::MonoSemiBold, 17.0, tile_width - 24.0, &value);
            self.text(x + 12.0, y + 40.0, 17.0, FontFace::MonoSemiBold, color, &value);
            if let Some(sub) = sub {
                let sub = self.clipped(FontFace::Regular, 7.5, tile_width - 24.0, &sub);
                self.text(x + 12.0, y + 53.0, 7.5, FontFace::Regular, MUTED, &sub);
            }
        }
        self.y += 2.0 * TILE_HEIGHT + GAP + 8.0;
    }

    /// The balance curve with its range, the cumulative R, or only the shape of the curve.
    fn curve(&mut self, report: &MonthlyReport) {
        const HEIGHT: f32 = 140.0;
        let (title, values): (&str, Vec<f64>) = if self.privacy.shows_money() {
            ("Đường balance", report.curve.iter().map(|p| p.balance).collect())
        } else {
            ("Đường R tích luỹ", report.r_curve.clone())
        };
        self.heading(title);
        if values.len() < 2 {
            self.note("Chưa đủ lệnh đóng trong tháng để vẽ đường.");
            return;
        }
        self.ensure(HEIGHT + 10.0);
        let (x0, y0) = (MARGIN, self.y);
        self.rect((x0, y0, CONTENT_WIDTH, HEIGHT), 6.0, None, Some(RULE));
        let (left, right, top, bottom) = (x0 + 56.0, x0 + CONTENT_WIDTH - 14.0, y0 + 14.0, y0 + HEIGHT - 22.0);
        let (low, high) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let flat = (high - low).abs() < f64::EPSILON;
        let last = count(values.len() - 1);
        let points: Vec<(f32, f32)> = values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let fraction = if flat { 0.5 } else { narrow((v - low) / (high - low)) };
                (left + (right - left) * count(i) / last, bottom - fraction * (bottom - top))
            })
            .collect();
        for y in [top, bottom] {
            self.push(Op::Line { from: (left, y), to: (right, y), color: RULE, width: 0.5 });
        }
        let up = values.last().copied().unwrap_or(0.0) >= values.first().copied().unwrap_or(0.0);
        let color = if up { PROFIT } else { LOSS };
        self.push(Op::Area { points: points.clone(), base: bottom, color, opacity: 0.12 });
        self.push(Op::Poly { points, color, width: 1.6 });
        match self.privacy {
            Privacy::NoFigures => {
                self.text(x0 + 10.0, y0 + 20.0, 7.5, FontFace::Regular, MUTED, "chỉ hình dạng, đã ẩn số liệu");
            }
            privacy => {
                let label = |v: f64, me: &Self| match privacy {
                    Privacy::ShowMoney => money(v, &me.currency).trim_start_matches('+').to_owned(),
                    _ => r_text(v, 1),
                };
                let (top_label, bottom_label) = (label(high, self), label(low, self));
                self.text_right(left - 6.0, top + 3.0, 7.5, FontFace::Mono, MUTED, &top_label);
                self.text_right(left - 6.0, bottom + 3.0, 7.5, FontFace::Mono, MUTED, &bottom_label);
            }
        }
        let (first_day, last_day) = month_bounds(report.month);
        self.text(left, y0 + HEIGHT - 8.0, 7.5, FontFace::Regular, MUTED, &first_day.format("%d/%m").to_string());
        self.text_right(right, y0 + HEIGHT - 8.0, 7.5, FontFace::Regular, MUTED, &last_day.format("%d/%m").to_string());
        self.y = y0 + HEIGHT + 10.0;
    }

    /// The month as a grid of days, tinted by the day's result, with money, R or nothing in it.
    fn calendar(&mut self, weeks: &[CalendarWeek]) {
        const CELL_HEIGHT: f32 = 36.0;
        self.heading("Lịch tháng");
        if weeks.is_empty() {
            self.note("Chưa có dữ liệu ngày.");
            return;
        }
        self.ensure(16.0 + count(weeks.len()) * CELL_HEIGHT);
        let cell = CONTENT_WIDTH / 7.0;
        for (i, name) in ["T2", "T3", "T4", "T5", "T6", "T7", "CN"].into_iter().enumerate() {
            let middle = MARGIN + (count(i) + 0.5) * cell;
            self.text_centre(middle, self.y + 9.0, 8.0, FontFace::SemiBold, MUTED, name);
        }
        self.y += 14.0;
        let currency = self.currency.clone();
        for week in weeks {
            for (i, day) in week.days.iter().enumerate().filter(|(_, d)| d.in_month) {
                let (x, y) = (MARGIN + count(i) * cell, self.y);
                let net = day.stat.map(|s| s.net);
                let fill = match net.map(Outcome::of) {
                    Some(Outcome::Win) => PROFIT_SOFT,
                    Some(Outcome::Loss) => LOSS_SOFT,
                    _ => Rgb(0xff, 0xff, 0xff),
                };
                self.rect((x + 1.0, y + 1.0, cell - 2.0, CELL_HEIGHT - 2.0), 4.0, Some(fill), Some(RULE));
                self.text(x + 5.0, y + 11.0, 7.5, FontFace::Regular, MUTED, &day.date.day().to_string());
                let Some(stat) = day.stat else { continue };
                let (text, color) = match self.privacy {
                    Privacy::ShowMoney => (money_whole(stat.net, &currency), tone(stat.net)),
                    Privacy::OnlyR if stat.r_trades > 0 => (r_text(stat.r, 1), tone(stat.r)),
                    Privacy::OnlyR => (format!("{} lệnh", stat.trades), MUTED),
                    Privacy::NoFigures => continue,
                };
                self.text_centre(x + cell / 2.0, y + 27.0, 8.0, FontFace::MonoSemiBold, color, &text);
            }
            self.y += CELL_HEIGHT;
        }
        self.y += 6.0;
    }

    fn playbooks(&mut self, report: &MonthlyReport) {
        self.heading("Theo playbook");
        if report.by_playbook.is_empty() {
            self.note("Chưa có lệnh nào trong tháng.");
            return;
        }
        let privacy = self.privacy;
        let currency = self.currency.clone();
        let mut columns = vec![
            Column { title: "Setup", width: 200.0, right: false },
            Column { title: "Lệnh", width: 55.0, right: true },
            Column { title: "Thắng", width: 60.0, right: true },
            Column { title: "PF", width: 55.0, right: true },
        ];
        if privacy.shows_r() {
            columns.push(Column { title: "Tổng R", width: 70.0, right: true });
        }
        if privacy.shows_money() {
            columns.push(Column { title: "Net", width: 75.0, right: true });
        }
        let used: f32 = columns.iter().map(|c| c.width).sum();
        columns[0].width += CONTENT_WIDTH - used;
        let rows: Vec<Vec<Cell>> = report
            .by_playbook
            .iter()
            .map(|g| {
                let mut row = vec![
                    plain(g.label.clone()),
                    plain(g.trades.to_string()),
                    plain(percent(g.win_rate())),
                    plain(g.profit_factor().map_or_else(|| "—".to_owned(), |p| format!("{p:.2}"))),
                ];
                if privacy.shows_r() {
                    row.push(if g.r_trades > 0 { (r_text(g.total_r, 1), tone(g.total_r)) } else { plain("—") });
                }
                if privacy.shows_money() {
                    row.push((money(g.net, &currency), tone(g.net)));
                }
                row
            })
            .collect();
        self.table(&columns, &rows);
    }

    fn mistakes(&mut self, report: &MonthlyReport) {
        self.heading("Lỗi tốn kém nhất");
        if report.mistakes.is_empty() {
            self.note("Tháng này chưa gắn lỗi nào cho lệnh thua.");
            return;
        }
        let privacy = self.privacy;
        let currency = self.currency.clone();
        let mut columns = vec![
            Column { title: "Lỗi", width: 200.0, right: false },
            Column { title: "Số lệnh", width: 70.0, right: true },
        ];
        if privacy.shows_r() {
            columns.push(Column { title: "Chi phí", width: 150.0, right: true });
        }
        columns.push(Column { title: "% tổng lỗ", width: 80.0, right: true });
        let used: f32 = columns.iter().map(|c| c.width).sum();
        columns[0].width += CONTENT_WIDTH - used;
        let rows: Vec<Vec<Cell>> = report
            .mistakes
            .iter()
            .map(|m| {
                let mut row = vec![plain(m.label.clone()), plain(m.trades.to_string())];
                if privacy.shows_money() {
                    row.push((format!("{} · {}", money(m.net, &currency), r_text(m.total_r, 1)), LOSS));
                } else if privacy.shows_r() {
                    row.push((r_text(m.total_r, 1), LOSS));
                }
                row.push(plain(percent(m.share_of_losses)));
                row
            })
            .collect();
        self.table(&columns, &rows);
    }

    fn missed(&mut self, report: &MonthlyReport) {
        self.heading("Lệnh bỏ lỡ");
        let missed = &report.missed;
        if missed.count == 0 {
            self.note("Tháng này không có lệnh bỏ lỡ nào được ghi.");
            return;
        }
        let summary = if self.privacy.shows_r() {
            format!(
                "{} lệnh bỏ lỡ · còn nằm lại trên bàn {} · tránh được {:.1}R lỗ",
                missed.count,
                r_text(missed.r_missed, 1),
                missed.r_avoided
            )
        } else {
            format!("{} lệnh bỏ lỡ", missed.count)
        };
        self.note(&summary);
        let privacy = self.privacy;
        let mut columns = vec![
            Column { title: "Lý do", width: 200.0, right: false },
            Column { title: "Số lệnh", width: 70.0, right: true },
        ];
        if privacy.shows_r() {
            columns.push(Column { title: "R nếu đã vào", width: 100.0, right: true });
        }
        let used: f32 = columns.iter().map(|c| c.width).sum();
        columns[0].width += CONTENT_WIDTH - used;
        let rows: Vec<Vec<Cell>> = report
            .missed_reasons
            .iter()
            .map(|(name, n, r)| {
                let mut row = vec![plain(name.clone()), plain(n.to_string())];
                if privacy.shows_r() {
                    row.push((r_text(*r, 1), tone(*r)));
                }
                row
            })
            .collect();
        self.table(&columns, &rows);
    }

    fn review(&mut self, report: &MonthlyReport) {
        self.heading("Tổng kết tháng");
        let Some(review) = &report.review else {
            self.note("Chưa viết tổng kết cho tháng này.");
            return;
        };
        if review.rating > 0 {
            self.note(&format!("Điểm kỷ luật của tháng: {} / 5", review.rating));
        }
        for (prompt, answer) in &review.answers {
            self.ensure(LEADING * 3.0);
            self.paragraph(FontFace::SemiBold, BODY, INK, prompt);
            if answer.trim().is_empty() {
                self.paragraph(FontFace::Regular, BODY, MUTED, "(chưa trả lời)");
            } else {
                self.paragraph(FontFace::Regular, BODY, INK, answer.trim());
            }
            self.y += 6.0;
        }
    }

    /// The running header on every page after the first, and the page number on all.
    fn furniture(&mut self, report: &MonthlyReport) {
        let total = self.pages.len();
        let header = format!("TradeLens · Báo cáo {}", Period::Month.title(report.month).to_lowercase());
        for index in 0..total {
            let label = format!("Trang {} / {total}", index + 1);
            let right = PAGE_WIDTH - MARGIN;
            let x = right - self.width(FontFace::Regular, 8.0, &label);
            let mut ops = Vec::new();
            if index > 0 {
                ops.push(Op::Text {
                    x: MARGIN,
                    y: 36.0,
                    size: 8.0,
                    face: FontFace::Regular,
                    color: MUTED,
                    text: header.clone(),
                });
                ops.push(Op::Line { from: (MARGIN, 44.0), to: (right, 44.0), color: RULE, width: 0.5 });
            }
            ops.push(Op::Text {
                x,
                y: PAGE_HEIGHT - 28.0,
                size: 8.0,
                face: FontFace::Regular,
                color: MUTED,
                text: label,
            });
            self.pages[index].extend(ops);
        }
    }
}

/// An `f64` fraction as the `f32` the page works in; a position needs no more precision.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a position on a page needs no more than f32 precision; there is no TryFrom<f64> for f32"
)]
fn narrow(fraction: f64) -> f32 {
    fraction as f32
}

/// The first and the last day of the month holding `day`.
fn month_bounds(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    Period::Month.bounds(day)
}

fn layout(report: &MonthlyReport, privacy: Privacy) -> Vec<Vec<Op>> {
    let metrics = Metrics::new();
    let mut layout = Layout::new(&metrics, privacy, &report.currency);
    layout.cover(report);
    layout.curve(report);
    layout.calendar(&report.weeks);
    layout.playbooks(report);
    layout.mistakes(report);
    layout.missed(report);
    layout.review(report);
    layout.furniture(report);
    layout.pages
}

/// The report as the bytes of an A4 PDF, showing what `privacy` allows.
///
/// # Errors
///
/// [`ReportError::Render`] when a font cannot be embedded or the PDF cannot be assembled.
pub fn pdf_bytes(report: &MonthlyReport, privacy: Privacy) -> Result<Vec<u8>, ReportError> {
    let pages = layout(report, privacy);
    let mut fonts: HashMap<FontFace, Font> = HashMap::new();
    for face in FontFace::ALL {
        let font = Font::new(Data::from(face.bytes()), 0)
            .ok_or_else(|| ReportError::Render(format!("không nạp được font {face:?}")))?;
        fonts.insert(face, font);
    }
    let mut document = Document::new();
    let title = format!("Báo cáo {} — {}", Period::Month.title(report.month).to_lowercase(), report.account);
    document.set_metadata(Metadata::new().title(title).creator("TradeLens".to_owned()).language("vi".to_owned()));
    let settings = PageSettings::from_wh(PAGE_WIDTH, PAGE_HEIGHT)
        .ok_or_else(|| ReportError::Render("khổ giấy không hợp lệ".to_owned()))?;
    for ops in &pages {
        let mut page = document.start_page_with(settings.clone());
        let mut surface = page.surface();
        for op in ops {
            draw(&mut surface, &fonts, op)?;
        }
        surface.finish();
        page.finish();
    }
    document.finish().map_err(|e| ReportError::Render(format!("{e:?}")))
}

/// Writes the report as an A4 PDF at `path`, showing what `privacy` allows.
///
/// # Errors
///
/// [`ReportError::Render`] when the PDF cannot be built; [`ReportError::Io`] when the file cannot
/// be written.
pub fn write_pdf(report: &MonthlyReport, privacy: Privacy, path: &Path) -> Result<(), ReportError> {
    let bytes = pdf_bytes(report, privacy)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn fill(color: Rgb, opacity: f32) -> Fill {
    Fill {
        paint: rgb::Color::new(color.0, color.1, color.2).into(),
        opacity: NormalizedF32::new(opacity).unwrap_or(NormalizedF32::ONE),
        rule: FillRule::default(),
    }
}

fn stroke(color: Rgb, width: f32) -> Stroke {
    Stroke { paint: rgb::Color::new(color.0, color.1, color.2).into(), width, ..Stroke::default() }
}

/// The corner control point distance of a quarter circle, as a share of the radius.
const KAPPA: f32 = 0.552_284_8;

fn rounded(rect: Rect, radius: f32) -> Option<krilla::geom::Path> {
    let mut path = PathBuilder::new();
    if radius <= 0.0 {
        path.push_rect(rect);
        return path.finish();
    }
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let corner = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let pull = corner * KAPPA;
    path.move_to(left + corner, top);
    path.line_to(right - corner, top);
    path.cubic_to(right - corner + pull, top, right, top + corner - pull, right, top + corner);
    path.line_to(right, bottom - corner);
    path.cubic_to(right, bottom - corner + pull, right - corner + pull, bottom, right - corner, bottom);
    path.line_to(left + corner, bottom);
    path.cubic_to(left + corner - pull, bottom, left, bottom - corner + pull, left, bottom - corner);
    path.line_to(left, top + corner);
    path.cubic_to(left, top + corner - pull, left + corner - pull, top, left + corner, top);
    path.close();
    path.finish()
}

fn polyline(points: &[(f32, f32)], base: Option<f32>) -> Option<krilla::geom::Path> {
    let mut path = PathBuilder::new();
    let (&(x0, y0), rest) = points.split_first()?;
    if let Some(base) = base {
        path.move_to(x0, base);
        path.line_to(x0, y0);
    } else {
        path.move_to(x0, y0);
    }
    for &(x, y) in rest {
        path.line_to(x, y);
    }
    if let (Some(base), Some(&(last, _))) = (base, points.last()) {
        path.line_to(last, base);
        path.close();
    }
    path.finish()
}

fn draw(surface: &mut Surface<'_>, fonts: &HashMap<FontFace, Font>, op: &Op) -> Result<(), ReportError> {
    match op {
        Op::Text { x, y, size, face, color, text } => {
            let font = fonts.get(face).cloned().ok_or_else(|| ReportError::Render(format!("thiếu font {face:?}")))?;
            surface.set_stroke(None);
            surface.set_fill(Some(fill(*color, 1.0)));
            surface.draw_text(Point::from_xy(*x, *y), font, *size, text, false, TextDirection::Auto);
        }
        Op::Rect { x, y, w, h, radius, fill: fill_color, stroke: edge } => {
            if let Some(path) = Rect::from_xywh(*x, *y, *w, *h).and_then(|rect| rounded(rect, *radius)) {
                surface.set_fill(fill_color.map(|c| fill(c, 1.0)));
                surface.set_stroke(edge.map(|(c, width)| stroke(c, width)));
                surface.draw_path(&path);
            }
        }
        Op::Line { from, to, color, width } => {
            let mut path = PathBuilder::new();
            path.move_to(from.0, from.1);
            path.line_to(to.0, to.1);
            if let Some(path) = path.finish() {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke(*color, *width)));
                surface.draw_path(&path);
            }
        }
        Op::Poly { points, color, width } => {
            if let Some(path) = polyline(points, None) {
                surface.set_fill(None);
                surface.set_stroke(Some(stroke(*color, *width)));
                surface.draw_path(&path);
            }
        }
        Op::Area { points, base, color, opacity } => {
            if let Some(path) = polyline(points, Some(*base)) {
                surface.set_stroke(None);
                surface.set_fill(Some(fill(*color, *opacity)));
                surface.draw_path(&path);
            }
        }
    }
    Ok(())
}

/// The words of every text drawn, page by page: what a reader of the PDF can find.
#[cfg(test)]
fn texts(pages: &[Vec<Op>]) -> Vec<String> {
    pages
        .iter()
        .map(|ops| {
            ops.iter()
                .filter_map(|op| match op {
                    Op::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        metrics::{self, DayStat, EquityPoint, Group, MistakeCost, Stats},
        missed::MissedSummary,
        review::PeriodReview,
    };

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("a day of November 2026")
    }

    fn sample_report() -> MonthlyReport {
        let stats = Stats {
            trades: 21,
            wins: 13,
            losses: 7,
            breakeven: 1,
            win_rate: 13.0 / 21.0,
            net: 1248.6,
            profit_factor: Some(2.314),
            payoff: Some(1.4),
            total_r: 6.4,
            r_trades: 20,
            avg_r: Some(0.32),
            max_drawdown: 310.0,
            max_drawdown_pct: Some(0.031),
            volume: 8.5,
            green_days: 9,
            trading_days: 14,
            max_win_streak: 4,
            max_loss_streak: 2,
            ..Stats::default()
        };
        let mut days = BTreeMap::new();
        days.insert(day(3), DayStat { net: 412.5, trades: 2, wins: 2, r: 2.1, r_trades: 2 });
        days.insert(day(4), DayStat { net: -95.0, trades: 1, wins: 0, r: -1.0, r_trades: 1 });
        let curve: Vec<EquityPoint> = [10_000.0, 10_412.5, 10_317.5, 11_248.6]
            .into_iter()
            .enumerate()
            .map(|(i, balance)| EquityPoint {
                time_ms: i64::try_from(i).expect("small") * 86_400_000,
                balance,
                peak: balance,
                drawdown: 0.0,
            })
            .collect();
        let group = Group {
            label: "Breakout phiên Á".into(),
            trades: 12,
            wins: 8,
            net: 987.65,
            gross_profit: 1200.0,
            gross_loss: -212.35,
            total_r: 4.1,
            r_trades: 12,
        };
        let mistake = MistakeCost {
            key: "move_sl".into(),
            label: "Dời SL".into(),
            trades: 3,
            net: -311.4,
            total_r: -2.6,
            share_of_losses: 0.42,
            expectancy_gap: -80.0,
        };
        let mut review = PeriodReview::blank(&crate::review::ReviewTemplate::default_for(Period::Month), day(1), None);
        review.rating = 4;
        review.answers[0].1 = "Đạt mục tiêu 5R, hơn kế hoạch một chút.".to_owned();
        review.answers[2].1 = "Dời SL khi đang lãi nhẹ ".repeat(60);
        MonthlyReport::new("Exness · 12345".into(), "USD".into(), day(1), stats)
            .with_curves(curve, vec![0.0, 2.1, 1.1, 6.4])
            .with_calendar(metrics::month_weeks(2026, 11, &days))
            .with_playbooks(vec![group])
            .with_mistakes(vec![mistake])
            .with_missed(
                MissedSummary { count: 2, r_missed: 3.0, r_avoided: 1.0, by_reason: vec![("fear".into(), 2, 2.0)] },
                |key| format!("Sợ vào lệnh ({key})"),
            )
            .with_review(Some(review))
    }

    #[test]
    fn the_report_lays_out_every_section_on_a_few_a4_pages() {
        let pages = layout(&sample_report(), Privacy::ShowMoney);
        assert!((2..=3).contains(&pages.len()), "{} pages", pages.len());
        let all = texts(&pages).join("\n");
        for wanted in [
            "Báo cáo tháng 11/2026",
            "Exness · 12345 · USD",
            "+$1,248.60",
            "Profit factor",
            "2.31",
            "Đường balance",
            "Lịch tháng",
            "Breakout phiên Á",
            "Dời SL",
            "Sợ vào lệnh (fear)",
            "Điểm kỷ luật của tháng: 4 / 5",
            "Kết quả so với mục tiêu tháng",
            "Trang 1 / ",
        ] {
            assert!(all.contains(wanted), "the report lacks {wanted:?}");
        }
    }

    #[test]
    fn a_long_answer_flows_onto_the_next_page_and_nothing_leaves_the_margins() {
        let pages = layout(&sample_report(), Privacy::ShowMoney);
        for ops in &pages {
            for op in ops {
                if let Op::Text { y, .. } = op {
                    assert!(*y < PAGE_HEIGHT && *y > 0.0, "a line at {y} is off the page");
                }
            }
        }
        let last = texts(&pages).pop().expect("pages");
        assert!(last.contains("Dời SL khi đang lãi nhẹ"), "the long answer ends on the last page");
    }

    #[test]
    fn only_r_and_no_figures_leave_money_out_of_the_layout() {
        let money_text = texts(&layout(&sample_report(), Privacy::ShowMoney)).join("\n");
        assert!(money_text.contains("$1,248.60") && money_text.contains("$987.65") && money_text.contains("−$311.40"));
        for privacy in [Privacy::OnlyR, Privacy::NoFigures] {
            let text = texts(&layout(&sample_report(), privacy)).join("\n");
            assert!(!text.contains('$'), "{privacy:?} shows a dollar amount:\n{text}");
            for figure in ["1,248", "1248", "987.65", "311.4", "412", "10,000"] {
                assert!(!text.contains(figure), "{privacy:?} shows {figure}");
            }
        }
        let only_r = texts(&layout(&sample_report(), Privacy::OnlyR)).join("\n");
        assert!(only_r.contains("+6.4R") && only_r.contains("Đường R tích luỹ"));
        let hidden = texts(&layout(&sample_report(), Privacy::NoFigures)).join("\n");
        assert!(!hidden.contains("+6.4R") && !hidden.contains("−2.6R"), "no R either:\n{hidden}");
        assert!(hidden.contains("Thắng") && hidden.contains("61.9%"), "ratios and the outcome stay");
    }

    #[test]
    fn an_empty_month_still_makes_a_one_page_report() {
        let report = MonthlyReport::new(String::new(), String::new(), day(1), Stats::default());
        let pages = layout(&report, Privacy::OnlyR);
        assert_eq!(pages.len(), 1);
        let text = texts(&pages).join("\n");
        assert!(text.contains("Tất cả tài khoản") && text.contains("Chưa viết tổng kết"), "{text}");
    }

    #[test]
    fn text_wraps_at_the_line_width_and_breaks_a_word_too_wide() {
        let metrics = Metrics::new();
        let layout = Layout::new(&metrics, Privacy::OnlyR, "USD");
        let lines = layout.wrap(FontFace::Regular, 10.0, 100.0, "một hai ba bốn năm sáu bảy tám chín mười");
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| layout.width(FontFace::Regular, 10.0, l) <= 100.0 + 0.01), "{lines:?}");
        let broken = layout.wrap(FontFace::Regular, 10.0, 40.0, "Xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
        assert!(broken.len() > 1 && broken.concat() == "Xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "{broken:?}");
        assert_eq!(layout.wrap(FontFace::Regular, 10.0, 100.0, "a\n\nb"), ["a", "", "b"]);
        assert!(layout.clipped(FontFace::Regular, 10.0, 30.0, "Một cái tên rất dài").ends_with('…'));
    }

    /// The text of every page of a PDF, read back with a PDF library.
    fn read_back(bytes: &[u8]) -> (usize, String) {
        let document = lopdf::Document::load_mem(bytes).expect("the PDF parses");
        let numbers: Vec<u32> = document.get_pages().keys().copied().collect();
        let text = document.extract_text(&numbers).expect("the PDF's text is readable");
        (numbers.len(), text)
    }

    #[test]
    fn the_pdf_parses_back_with_its_pages_and_text() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("thang-11.pdf");
        let report = sample_report();
        write_pdf(&report, Privacy::ShowMoney, &path).expect("the PDF is written");
        let bytes = std::fs::read(&path).expect("the file exists");
        assert!(bytes.starts_with(b"%PDF-"));
        let (pages, text) = read_back(&bytes);
        assert_eq!(pages, layout(&report, Privacy::ShowMoney).len(), "one PDF page per laid-out page");
        for wanted in ["Báo cáo tháng 11/2026", "Breakout phiên Á", "$1,248.60"] {
            assert!(text.contains(wanted), "the PDF text lacks {wanted:?}: {text}");
        }
    }

    #[test]
    fn the_pdf_hides_money_when_asked() {
        let report = sample_report();
        let (_, shown) = read_back(&pdf_bytes(&report, Privacy::ShowMoney).expect("PDF"));
        assert!(shown.contains("1,248.60"), "money is in the PDF when shown");
        for privacy in [Privacy::OnlyR, Privacy::NoFigures] {
            let (_, text) = read_back(&pdf_bytes(&report, privacy).expect("PDF"));
            assert!(text.contains("Báo cáo tháng 11/2026"), "the PDF has text at all");
            assert!(!text.contains('$') && !text.contains("1,248") && !text.contains("987.65"), "{privacy:?}:\n{text}");
        }
    }

    #[test]
    fn the_report_writes_to_a_missing_folder_as_an_io_error() {
        let report = MonthlyReport::new(String::new(), String::new(), day(1), Stats::default());
        let path = std::path::Path::new("Z:/no/such/folder/report.pdf");
        assert!(matches!(write_pdf(&report, Privacy::OnlyR, path), Err(ReportError::Io(_))));
    }
}
