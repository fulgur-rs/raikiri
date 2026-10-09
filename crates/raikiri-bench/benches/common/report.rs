//! A report-style document for cascade measurements: pages of item tables
//! with headers, subtotal rows, notes and footers, styled the way business
//! reports usually are (custom properties, `@page` rules, zebra striping,
//! generated content and counters), on top of the user agent stylesheet.

use std::fmt::Write as _;

/// Author stylesheet of the report, one rule per entry.
// cov:ignore: bench harness — this module is shared by a bench target, which
// the coverage run never builds, and an integration test target, whose files
// it does not report on.
const REPORT_RULES: &[&str] = &[
    ":root { --accent: #1f4e79; --rule: 1px solid #999; --pad: 2px 4px; }",
    "@page { size: A4; margin: 15mm 12mm; }",
    "@page :first { margin-top: 25mm; }",
    "body { font-family: \"Noto Sans JP\", sans-serif; font-size: 10pt; line-height: 1.4; color: #222; }",
    "header.page-header { display: flex; justify-content: space-between; border-bottom: 2px solid var(--accent); margin-bottom: 4mm; }",
    "header.page-header h1 { font-size: 14pt; margin: 0; color: var(--accent); }",
    "header.page-header .meta { color: #666; font-size: 9pt; }",
    "section.page { break-after: page; counter-increment: report-page; }",
    "section.page:last-child { break-after: auto; }",
    "table.items { width: 100%; border-collapse: collapse; table-layout: fixed; }",
    "table.items th, table.items td { border: var(--rule); padding: var(--pad); vertical-align: top; }",
    "table.items th { background: #e8eef5; font-weight: bold; text-align: left; }",
    "table.items tbody tr:nth-child(even) { background-color: #f7f7f7; }",
    "table.items td.num { text-align: right; font-variant-numeric: tabular-nums; }",
    "table.items td.code { font-family: monospace; white-space: nowrap; }",
    "table.items tr.subtotal td { font-weight: bold; border-top: 2px solid #333; }",
    "table.items td.warn { color: #b00020 !important; }",
    ".note { font-size: 8pt; color: #555; margin-top: 2mm; }",
    ".note::before { content: \"※ \"; }",
    "footer.page-footer { margin-top: 4mm; font-size: 8pt; text-align: center; }",
    "footer.page-footer::after { content: \" \" counter(report-page); }",
];

/// A report of `pages` pages with `rows` item rows each.
// cov:ignore: bench harness — same reason as `REPORT_RULES` above.
pub fn report_html(pages: usize, rows: usize) -> String {
    let mut html = format!(
        "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\"><style>{}</style></head><body>",
        REPORT_RULES.join("\n")
    );
    for page in 1..=pages {
        let _ = write!(
            html,
            "<section class=\"page\"><header class=\"page-header\"><h1>請求明細 {page}</h1>\
             <span class=\"meta\">2026-10-09</span></header><table class=\"items\"><thead><tr>\
             <th>コード</th><th>品名</th><th>数量</th><th>単価</th><th>金額</th><th>備考</th>\
             </tr></thead><tbody>"
        );
        let mut subtotal = 0;
        for row in 1..=rows {
            let quantity = row % 9 + 1;
            let price = (row * 37 + page * 11) % 5000 + 100;
            subtotal += quantity * price;
            let remark = if row % 7 == 0 {
                "<td class=\"warn\">要確認</td>"
            } else {
                "<td></td>"
            };
            let _ = write!(
                html,
                "<tr><td class=\"code\">A-{page:03}{row:04}</td><td>品目 {row}</td>\
                 <td class=\"num\">{quantity}</td><td class=\"num\">{price}</td>\
                 <td class=\"num\">{}</td>{remark}</tr>",
                quantity * price
            );
        }
        let _ = write!(
            html,
            "<tr class=\"subtotal\"><td colspan=\"4\">小計</td><td class=\"num\">{subtotal}</td>\
             <td></td></tr></tbody></table><p class=\"note\">金額は税抜きです。</p>\
             <footer class=\"page-footer\">Page</footer></section>"
        );
    }
    html.push_str("</body></html>");
    html
}
