//! Semantic Builder — converts structured content (HTML tables, etc.) into
//! dense semantic text for embedding / sub-chunk retrieval.

/// Convert an HTML `<table>` string to KV (key=value) text.
///
/// Scans character-by-character for `<tr>`, `<td>`, `<th>` tags.
/// Handles: rowspan via simple inheritance, colspan → expand, `<br>` → space, HTML entities.
///
/// Output (header mode — first row looks like column names):
///   类别=MPLS；业务特性=支持 MPLS OAM；S12700E-4=✓。
///
/// Output (record mode — first row is data):
///   记录1：
///   交换处理单元
///   LST7SFUEX100
///   S12700E交换网单元E(X1)
pub fn html_table_to_kv(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // Find <table> start
    let tbl_start = loop {
        if i >= len { return html.to_string(); }
        if chars[i] == '<' && tag_matches(&chars, i, len, "table") { break i; }
        i += 1;
    };
    // Find </table> end
    let tbl_end = loop {
        if i >= len { return html.to_string(); }
        if chars[i] == '<' && close_tag_matches(&chars, i, len, "table") { break i; }
        i += 1;
    };

    // ── 1. Parse rows with rowspan/colspan info ──
    // raw_rows: Vec<Vec<(text, rowspan)>>
    let mut raw_rows: Vec<Vec<(String, usize)>> = Vec::new();
    let mut cur_row: Vec<(String, usize)> = Vec::new();
    let mut in_cell = false;
    let mut cell_text = String::new();
    let mut cell_attrs: (usize, usize) = (1, 1); // (rowspan, colspan)

    i = tbl_start;
    while i < tbl_end {
        if chars[i] == '<' {
            let end = chars[i..].iter().position(|&c| c == '>').map(|p| i + p + 1).unwrap_or(tbl_end);
            let tag_str: String = chars[i+1..end-1].iter().collect();
            let tag_lower = tag_str.to_lowercase();
            let is_close = tag_lower.starts_with('/');
            let name = if is_close { &tag_lower[1..] } else { &tag_lower.split_whitespace().next().unwrap_or(&tag_lower) };

            match name {
                "tr" if is_close => {
                    if !cur_row.is_empty() {
                        raw_rows.push(std::mem::take(&mut cur_row));
                    }
                }
                "td" | "th" if !is_close => {
                    cell_text.clear();
                    in_cell = true;
                    // Parse rowspan/colspan from attributes
                    cell_attrs = (get_attr(&tag_str, "rowspan"), get_attr(&tag_str, "colspan"));
                }
                "td" | "th" if is_close => {
                    in_cell = false;
                    let text = clean_br_text(&decode_entities(&cell_text));
                    let (rs, cs) = cell_attrs;
                    // Expand colspan: push the cell value cs times
                    for _ in 0..cs {
                        cur_row.push((text.clone(), rs));
                    }
                }
                "br" | "br/" => { if in_cell { cell_text.push('\x01'); } }
                _ => {}
            }
            i = end;
        } else if in_cell {
            cell_text.push(chars[i]);
            i += 1;
        } else {
            i += 1;
        }
    }
    if !cur_row.is_empty() { raw_rows.push(cur_row); }

    // ── 2. Apply rowspan inheritance ──
    // Build flat grid: Vec<Vec<String>>
    use std::collections::HashMap;
    let max_cols = raw_rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut all_rows: Vec<Vec<String>> = Vec::new();
    // carry: column_index → (value, remaining_rows)
    let mut carry: HashMap<usize, (String, usize)> = HashMap::new();

    for row in &raw_rows {
        let mut flat = vec![String::new(); max_cols.max(carry.len() + row.len())];
        let mut next_carry: HashMap<usize, (String, usize)> = HashMap::new();

        // Fill in carry-over values from previous row's rowspans
        for (&col, (val, remaining)) in &carry {
            while col >= flat.len() {
                flat.push(String::new());
            }
            flat[col] = val.clone();
            if *remaining > 1 {
                next_carry.insert(col, (val.clone(), remaining - 1));
            }
        }

        // Place current row's cells into remaining (non-carry) columns
        let mut col = 0usize;
        for (text, rs) in row {
            while col < flat.len() && carry.contains_key(&col) {
                col += 1;
            }
            if col >= flat.len() {
                flat.push(String::new());
            }
            flat[col] = text.clone();
            if *rs > 1 {
                next_carry.insert(col, (text.clone(), *rs - 1));
            }
            col += 1;
        }

        carry = next_carry;
        all_rows.push(flat);
    }

    if all_rows.is_empty() { return html.to_string(); }

    // ── 3. Scan first few rows to find the real header ──
    // Some Excel tables have a section-title row before the actual column names.
    // We only skip a row if it's confirmed to be a section title (all identical).
    let max_scan = 2.min(all_rows.len().saturating_sub(2));
    let header_row = (0..=max_scan).find(|&ri| {
        // Only consider row ri if all rows above it are section titles
        // (every non-empty cell in those rows has the same value).
        for prev in 0..ri {
            let prev_row = &all_rows[prev];
            let non_empty: Vec<&str> = prev_row.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            if non_empty.is_empty() || non_empty.iter().any(|s| *s != non_empty[0]) {
                return false; // previous row is NOT a section title → can't skip it
            }
        }
        looks_like_header_row(&all_rows, ri)
    });

    if let Some(ri) = header_row {
        // Rebuild grid: ri becomes header, everything below is data
        let mut sliced = vec![all_rows[ri].clone()];
        sliced.extend_from_slice(&all_rows[ri + 1..]);
        header_kv_output(&sliced, html)
    } else {
        record_output(&all_rows, html)
    }
}

/// Heuristic: does row `ri` look like a header (column names)?
fn looks_like_header_row(all_rows: &[Vec<String>], ri: usize) -> bool {
    let Some(row) = all_rows.get(ri) else { return false };
    // Only scan if there are data rows below
    if ri + 1 >= all_rows.len() { return false; }
    let ncols = row.len();

    // Rule 1: If any cell contains check/cross marks, it's data (not a header)
    if row.iter().any(|c| c.contains('✓') || c.contains('×')) {
        return false;
    }

    // Rule 2: If >50% of cells are empty, they can't be column names
    let empty_count = row.iter().filter(|c| c.trim().is_empty()).count();
    if empty_count > ncols / 2 {
        return false;
    }

    // Rule 3: If all non-empty cells are identical, it's a section title, not a header.
    {
        let non_empty: Vec<&str> = row.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !non_empty.is_empty() && non_empty.iter().all(|s| *s == non_empty[0]) {
            return false;
        }
    }

    // Rule 4: If the first column has a value that repeats in >50% of remaining rows,
    // this is a category column (first column is data, not a column name)
    let data_rows = &all_rows[(ri + 1)..];
    if !data_rows.is_empty() {
        let first_val = row.first().map(|s| s.trim()).unwrap_or("");
        if !first_val.is_empty() {
            let repeat = data_rows.iter()
                .filter(|r| r.first().map(|s| s.trim() == first_val).unwrap_or(false))
                .count();
            if repeat > data_rows.len() / 2 {
                return false;
            }
        }
    }

    // Rule 5: For non-row-0 candidates, the first column should contain meaningful
    // Chinese text (column names), not ASCII-heavy model numbers/codes.
    if ri > 0 {
        let first_val = row.first().map(|s| s.trim()).unwrap_or("");
        if !first_val.is_empty() {
            let ascii_alpha: usize = first_val.chars().filter(|c| c.is_ascii_alphabetic()).count();
            let non_ascii: usize = first_val.chars().filter(|c| !c.is_ascii()).count();
            // If ASCII letters dominate over non-ASCII (Chinese), it's likely a code/model number
            if non_ascii < 2 && ascii_alpha > non_ascii {
                return false;
            }
        }
    }

    true
}

/// KV format output: col=val；col=val。
fn header_kv_output(all_rows: &[Vec<String>], html: &str) -> String {
    let headers = &all_rows[0];
    if headers.is_empty() { return html.to_string(); }
    let hcount = headers.len();

    let mut out = String::new();
    for row in &all_rows[1..] {
        let mut cells = row.clone();
        while cells.len() < hcount { cells.push(String::new()); }
        if cells.len() > hcount { cells.truncate(hcount); }

        let pairs: Vec<String> = headers.iter().enumerate()
            .map(|(i, h)| {
                let val = cells[i].trim();
                if val.is_empty() {
                    format!("{}=", h)
                } else {
                    format!("{}={}", h, val)
                }
            })
            .collect();
        if pairs.is_empty() { continue; }
        if !out.is_empty() { out.push('\n'); }
        out.push_str(&pairs.join("；"));
        out.push('。');
    }

    if out.is_empty() { html.to_string() } else { out }
}

/// Record-style output for data-only tables (no column names).
/// Each row becomes: first_col：其余列，用，连接。
fn record_output(all_rows: &[Vec<String>], html: &str) -> String {
    let mut out = String::new();
    for row in all_rows {
        let trimmed: Vec<&str> = row.iter().map(|s| s.trim()).collect();
        // Collapse consecutive identical values (from colspan expansion in headerless tables)
        let folded: Vec<&str> = {
            let mut v = Vec::new();
            for val in &trimmed {
                if v.last().map(|last| *last == *val).unwrap_or(false) {
                    continue;
                }
                v.push(*val);
            }
            v
        };
        if !out.is_empty() { out.push('\n'); }
        let key = folded.first().copied().unwrap_or("");
        let rest: Vec<&str> = if folded.len() > 1 {
            folded[1..].iter().filter(|s| !s.is_empty()).copied().collect()
        } else {
            vec![]
        };
        if !key.is_empty() && !rest.is_empty() {
            out.push_str(&format!("{}：{}", key, rest.join("，")));
        } else if !key.is_empty() {
            out.push_str(&format!("{}：", key));
        } else {
            out.push_str(&rest.join("，"));
        }
    }
    if out.is_empty() { html.to_string() } else { out }
}

/// Convert a markdown table (pipe-delimited) to KV text.
///
/// Markdown format:
///   | 列名1 | 列名2 | 列名3 |
///   | --- | --- | --- |
///   | 值1 | 值2 | 值3 |
///
/// Output (header mode):
///   列名1=值1；列名2=值2；列名3=值3。
///
/// Output (record mode):
///   值1：值2，值3
pub fn markdown_table_to_kv(md: &str) -> String {
    // 1. Parse rows: split by lines, split each line by |
    let raw_lines: Vec<&str> = md.lines().collect();
    let mut all_rows: Vec<Vec<String>> = Vec::new();

    for line in &raw_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        // Must contain | to be a table line
        if !trimmed.contains('|') { continue; }
        // Skip separator row (| --- | --- |)
        if trimmed.chars().all(|c| c == '|' || c == '-' || c == ' ' || c == ':') { continue; }

        let cells: Vec<String> = trimmed
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if cells.iter().all(|c| c.trim().is_empty()) { continue; }
        all_rows.push(cells);
    }

    if all_rows.is_empty() { return md.to_string(); }

    // 2. Normalize column counts
    let max_cols = all_rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut all_rows {
        while row.len() < max_cols { row.push(String::new()); }
    }

    // 3. Detect header (reuse same heuristic as html_table_to_kv)
    let max_scan = 2.min(all_rows.len().saturating_sub(2));
    let header_row = (0..=max_scan).find(|&ri| {
        for prev in 0..ri {
            let prev_row = &all_rows[prev];
            let non_empty: Vec<&str> = prev_row.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            if non_empty.is_empty() || non_empty.iter().any(|s| *s != non_empty[0]) {
                return false;
            }
        }
        looks_like_header_row_md(&all_rows, ri)
    });

    if let Some(ri) = header_row {
        let mut sliced = vec![all_rows[ri].clone()];
        sliced.extend_from_slice(&all_rows[ri + 1..]);
        header_kv_output(&sliced, md)
    } else {
        record_output(&all_rows, md)
    }
}

/// Heuristic for markdown table header detection (same logic as looks_like_header_row)
fn looks_like_header_row_md(all_rows: &[Vec<String>], ri: usize) -> bool {
    let Some(row) = all_rows.get(ri) else { return false };
    if ri + 1 >= all_rows.len() { return false; }
    let ncols = row.len();

    // Rule 1: If any cell contains check/cross marks, it's data
    if row.iter().any(|c| c.contains('✓') || c.contains('×')) { return false; }

    // Rule 2: If >50% of cells are empty, can't be column names
    let empty_count = row.iter().filter(|c| c.trim().is_empty()).count();
    if empty_count > ncols / 2 { return false; }

    // Rule 3: If all non-empty cells are identical, it's a section title
    let non_empty: Vec<&str> = row.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if !non_empty.is_empty() && non_empty.iter().all(|s| *s == non_empty[0]) { return false; }

    // Rule 4: If first column value repeats in >50% of data rows, it's a category column
    let data_rows = &all_rows[(ri + 1)..];
    if !data_rows.is_empty() {
        let first_val = row.first().map(|s| s.trim()).unwrap_or("");
        if !first_val.is_empty() {
            let repeat = data_rows.iter()
                .filter(|r| r.first().map(|s| s.trim() == first_val).unwrap_or(false))
                .count();
            if repeat > data_rows.len() / 2 { return false; }
        }
    }

    // Rule 5: For non-row-0, first column should be Chinese text
    if ri > 0 {
        let first_val = row.first().map(|s| s.trim()).unwrap_or("");
        if !first_val.is_empty() {
            let ascii_alpha: usize = first_val.chars().filter(|c| c.is_ascii_alphabetic()).count();
            let non_ascii: usize = first_val.chars().filter(|c| !c.is_ascii()).count();
            if non_ascii < 2 && ascii_alpha > non_ascii { return false; }
        }
    }

    true
}

fn get_attr(tag_str: &str, attr: &str) -> usize {
    let search = format!("{}=\"", attr);
    tag_str.find(&search)
        .and_then(|pos| {
            let start = pos + search.len();
            tag_str[start..].find('"')
                .and_then(|end| tag_str[start..start+end].parse().ok())
        })
        .unwrap_or(1)
}

fn tag_matches(chars: &[char], pos: usize, _len: usize, name: &str) -> bool {
    let mut j = pos + 1;
    let name_chars: Vec<char> = name.chars().collect();
    for &c in &name_chars {
        if j >= chars.len() { return false; }
        if chars[j].to_ascii_lowercase() != c { return false; }
        j += 1;
    }
    // Next char should be space, >, or end of tag
    j < chars.len() && (chars[j] == '>' || chars[j] == ' ' || chars[j] == '/')
}

fn close_tag_matches(chars: &[char], pos: usize, _len: usize, name: &str) -> bool {
    if pos + 3 + name.len() > chars.len() { return false; }
    if chars[pos] != '<' || chars[pos+1] != '/' { return false; }
    let mut j = pos + 2;
    for c in name.chars() {
        if chars[j].to_ascii_lowercase() != c { return false; }
        j += 1;
    }
    chars[j] == '>'
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let c: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '&' {
            let end = c[i..].iter().position(|&x| x == ';').map(|p| i + p + 1).unwrap_or(c.len());
            let ent: String = c[i+1..end-1].iter().collect();
            let d = match ent.as_str() {
                "amp" => "&", "lt" => "<", "gt" => ">",
                "quot" => "\"", "apos" | "#39" => "'", "nbsp" => " ",
                _ => { i = end; continue; }
            };
            out.push_str(d);
            i = end;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// Post-process cell text containing `\x01` sentinel markers (placed at `<br>` breaks).
/// If the characters on both sides of the marker are both lowercase ASCII letters,
/// the marker is removed to join a split English word (e.g. `CloudEngi\x01ne` → `CloudEngine`).
/// Otherwise the marker is replaced with a space (e.g. `功能：\x01二层` → `功能： 二层`).
fn clean_br_text(text: &str) -> String {
    if !text.contains('\x01') {
        return text.to_string();
    }
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\x01' {
            let prev = i.checked_sub(1).and_then(|p| c.get(p)).copied();
            let next = c.get(i + 1).copied();
            let join = prev.map_or(false, |p| p.is_ascii_lowercase())
                && next.map_or(false, |n| n.is_ascii_lowercase());
            if !join {
                out.push(' ');
            }
            // join = remove marker entirely, no char pushed
            i += 1;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_attr() {
        assert_eq!(get_attr("td rowspan=\"4\"", "rowspan"), 4);
        assert_eq!(get_attr("td colspan=\"3\"", "colspan"), 3);
        assert_eq!(get_attr("td", "rowspan"), 1);
        assert_eq!(get_attr("th colspan=\"15\"", "colspan"), 15);
    }

    #[test]
    fn test_html_table_to_kv_simple() {
        let html = "<table><tr><th>名称</th><th>数量</th></tr><tr><td>苹果</td><td>10</td></tr></table>";
        let out = html_table_to_kv(html);
        assert!(out.contains("名称=苹果"), "got: {}", out);
        assert!(out.contains("数量=10"), "got: {}", out);
    }

    #[test]
    fn test_html_table_to_kv_colspan_header() {
        let html = "<table><tr><th colspan=\"2\">医院</th><th>价格</th></tr><tr><th>产品</th><th>数量</th><th>价格</th></tr><tr><td>药</td><td>10</td><td>100</td></tr></table>";
        let out = html_table_to_kv(html);
        // colspan=2 expands "医院" into 2 columns, but the 2nd row overwrites with "产品"/"数量"
        assert!(out.contains("医院=药"), "医院 col should persist from row0 header, got: {}", out);
        assert!(out.contains("医院=10"), "got: {}", out);
        assert!(out.contains("价格=100"), "got: {}", out);
    }

    #[test]
    fn test_html_table_to_kv_rowspan() {
        let html = concat!(
            "<table>",
            "<tr><th>类别</th><th>项目</th><th>值</th></tr>",
            "<tr><td rowspan=\"2\">水果</td><td>苹果</td><td>10</td></tr>",
            "<tr><td>香蕉</td><td>20</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        assert!(out.contains("类别=水果"), "rowspan: 类别 should carry, got: {}", out);
        assert!(out.contains("项目=苹果"), "got: {}", out);
        assert!(out.contains("值=10"), "got: {}", out);
        assert!(out.contains("项目=香蕉"), "got: {}", out);
        assert!(out.contains("值=20"), "got: {}", out);
    }

    #[test]
    fn test_html_table_to_kv_empty_cell() {
        let html = concat!(
            "<table>",
            "<tr><th>A</th><th>B</th><th>C</th></tr>",
            "<tr><td>x</td><td></td><td>z</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        // empty cell should produce A=x; B=; C=z
        assert!(out.contains("A=x"), "got: {}", out);
        assert!(out.contains("B="), "got: {}", out);
        assert!(out.contains("C=z"), "got: {}", out);
    }

    /// 用户贴出的完整产品规格表（全 <td>，但首行全局唯一 → 自动识别为表头输出 KV）
    #[test]
    fn test_product_spec_table() {
        let html = concat!(
            "<table>",
            "<tr><td>类别</td><td>业务特性</td><td>CloudEngi<br>ne<br>S12700E-4</td><td>CloudEngi<br>e S12700E-8</td><td>CloudEngi<br>ne<br>S12700E-12</td></tr>",
            "<tr><td rowspan=\"4\">用户管理</td><td>支持统一用户管理</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持PPPoE、802.1X、MAC、Portal 认证方式</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持基于流量和时长计费方式</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持分组分域分时授权方式</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"4\">MAC</td><td>支持MAC 地址自动学习和老化</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持静态、动态、黑洞MAC 表项</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持源MAC 地址过滤</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持基于端口和VLAN 的MAC 地址学习限制</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"6\">VLAN</td><td>支持4K 个VLAN</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持Access、Trunk、Hybrid 方式，支持LNP 链路类型自协商</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持default VLAN</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持VLAN 交换</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持QinQ、增强型灵活QinQ</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持基于MAC 的动态VLAN 分配</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>ARP</td><td>支持ARP Snooping</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"3\">IP 路由</td><td>支持RIP、OSPF、ISIS、BGP 等IPv4 动态路由协议</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持RIPng、OSPFv3、ISISv6、BGP4＋等IPv6 动态路由协议</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>FIBv4 路由规格</td><td>3,072,000(M ax)</td><td>3,072,000(Ma x)</td><td>3,072,000(M ax)</td></tr>",
            "<tr><td></td><td>FIBv6 路由规格</td><td>1,048,576(M ax)</td><td>1,048,576(Ma x)</td><td>1,048,576(M ax)</td></tr>",
            "<tr><td>组播</td><td>支持IGMPv1/v2/v3、IGMP v1/v2/v3 Snooping</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_product_spec_table] KV output:\n{}", out);

        // Must NOT be raw HTML
        assert!(!out.contains("<table>"), "should NOT contain raw HTML, got: {}", out);

        // Now correctly treated as KV: 类别=用户管理；业务特性=支持统一用户管理；
        assert!(out.contains("类别=用户管理"), "got: {}", out);
        assert!(out.contains("业务特性=支持统一用户管理"), "got: {}", out);

        // Rowspan carries: 类别 should still be 用户管理 for following rows
        assert!(out.contains("类别=用户管理"), "rowspan should still carry, got: {}", out);

        // MAC rowspan
        assert!(out.contains("类别=MAC"), "got: {}", out);

        // FIBv6 row has empty first cell → 类别= (kv format keeps column alignment)
        assert!(out.contains("类别=；业务特性=FIBv6 路由规格"), "got: {}", out);

        // CloudEngine should be joined correctly
        assert!(out.contains("CloudEngine S12700E-4"), "got: {}", out);
        assert!(!out.contains("CloudEngi ne S12700E-4"), "got: {}", out);
    }

    /// OCR PDF table: all <td>, first row values globally unique → treated as header KV
    #[test]
    fn test_ocr_pdf_header_table() {
        let html = concat!(
            "<table>",
            "<tr><td>类别</td><td>业务特性</td><td>CloudEngi<br/>ne<br/>S12700E-4</td><td>CloudEngi<br/>e S12700E-8</td><td>CloudEngi<br/>ne<br/>S12700E-12</td></tr>",
            "<tr><td rowspan=\"8\"></td><td>支持 PIM DM、PIM SM、PIM SSM</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 MSDP、MBGP</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持用户快速离开机制</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持组播流量控制</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持组播查询器</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持组播协议报文抑制功能</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持组播 CAC</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持组播 ACL</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"4\">MPLS</td><td>支持 MPLS 基本功能</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 MPLS OAM</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 MPLS TE</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 MPLS VPN/VLL/VPLS</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"3\">VXLAN</td><td>支持 VXLAN 分布式网关，集中式网关</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 BGP-EVPN</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持通过 Netconf 配置 VXLAN</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"5\">Qos</td><td>支持基于 Layer2 协议头、Layer3 协议、Layer4 协议、802.1p 优先级等的组合流分类</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 ACL、CAR、Remark、Schedule 等动作</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 PQ、WRR、DRR、PQ+WRR、PQ+DRR 等队列调度方式</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 WRED、尾丢弃等拥塞避免机制</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持 HQs</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td></td><td>支持流量整形</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td rowspan=\"2\">iPCA</td><td>支持直接对业务报文标记以获得丢包数量和丢包率的实时统计</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>支持二三层网络网络级和设备级丢包数量和丢包率统计</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "<tr><td>SVF 2.0</td><td>支持将多达 10K 个 Client 节点（接入交换机与 AP）虚拟为一台设备管理</td><td>✓</td><td>✓</td><td>✓</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_ocr_pdf_header_table] output:\n{}", out);

        // Must be KV format, not raw HTML
        assert!(!out.contains("<table>"), "should not be raw HTML, got: {}", out);

        // First data row: 类别=；业务特性=支持 PIM DM...
        assert!(out.contains("业务特性=支持 PIM DM"), "got: {}", out);
        assert!(out.contains("CloudEngine S12700E-4=✓"), "got: {}", out);

        // MPLS category appears
        assert!(out.contains("类别=MPLS"), "got: {}", out);

        // KV preserves column alignment for empty first cell
        assert!(out.contains("类别=；业务特性=支持流量整形"), "got: {}", out);
    }

    #[test]
    fn test_headerless_high_repetition() {
        let html = concat!(
            "<table>",
            "<tr><td>交换处理单元</td><td>LST7SFUEX100</td><td>S12700E交换网单元E(X1)</td></tr>",
            "<tr><td>交换处理单元</td><td>LST7SFUHX100</td><td>S12700E交换网单元H(X1)</td></tr>",
            "<tr><td>交换处理单元</td><td>LST7SFUMX100</td><td>S12700E交换网单元M(X1)</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_headerless_high_repetition] output:\n{}", out);
        assert!(out.contains("交换处理单元：LST7SFUEX100，S12700E交换网单元E(X1)"), "got: {}", out);
        assert!(out.contains("交换处理单元：LST7SFUHX100，S12700E交换网单元H(X1)"), "got: {}", out);
        assert!(out.contains("交换处理单元：LST7SFUMX100，S12700E交换网单元M(X1)"), "got: {}", out);
        assert!(!out.contains("记录1："), "should not have record prefix, got: {}", out);
        assert!(!out.contains("<table>"), "should not be raw HTML, got: {}", out);
    }

    /// 无表头 + 首列全部唯一 → header KV 输出
    #[test]
    fn test_headerless_low_repetition() {
        let html = concat!(
            "<table>",
            "<tr><td>项目</td><td>S12700E-4</td><td>S12700E-8</td><td>S12700E-12</td></tr>",
            "<tr><td>交换容量</td><td>952/2876 Tbps</td><td>1904/5752 Tbps</td><td>2856/8628 Tbps</td></tr>",
            "<tr><td>包转发率</td><td>230400 Mpps</td><td>460800 Mpps</td><td>921600 Mpps</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_headerless_low_repetition] output:\n{}", out);
        // First row values ("项目", "S12700E-4", ...) are globally unique,
        // so the heuristic detects it as a header table → KV format
        assert!(out.contains("项目=交换容量"), "got: {}", out);
        assert!(out.contains("项目=包转发率"), "got: {}", out);
        assert!(out.contains("S12700E-4=952/2876 Tbps"), "got: {}", out);
        assert!(out.contains("S12700E-4=230400 Mpps"), "got: {}", out);
        assert!(!out.contains("<table>"), "should not be raw HTML");
    }

    /// 无表头 + 中等重复率（≥20% <60%），含 colspan
    #[test]
    fn test_headerless_medium_repetition() {
        // A table with colspan in the header-equivalent row
        let html = concat!(
            "<table>",
            "<tr><td colspan=\"3\">CloudEngine S12700E 基本配置</td></tr>",
            "<tr><td>LE2BN66ED000</td><td>N66E 直流总装机柜</td><td>600×600×2200mm</td></tr>",
            "<tr><td>ET1BS12704E0</td><td>S12700E-4 总装机箱</td><td>10U</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_headerless_medium_repetition] output:\n{}", out);
        assert!(!out.contains("<table>"), "should not be raw HTML");
        // First row has colspan=3, so it's 1 cell "CloudEngine S12700E 基本配置"
        assert!(out.contains("CloudEngine S12700E 基本配置"), "got: {}", out);
        assert!(out.contains("LE2BN66ED000"), "got: {}", out);
        assert!(out.contains("ET1BS12704E0"), "got: {}", out);
    }

    /// 订购表：节标题行 (colspan=3，全格同一值) → 不是表头，走 record 模式
    #[test]
    fn test_section_title_colspan_not_header() {
        let html = concat!(
            "<table>",
            "<tr><td colspan=\"3\">40GE 以太网光接口板</td></tr>",
            "<tr><td>LST7L12QX6E0</td><td>12 端口 40GE 以太网光接口板(X6E,QSPF+)</td><td></td></tr>",
            "<tr><td>LST7L32QX6E0*</td><td>32 端口 40GE 以太网光接口板(X6E,QSPF+)</td><td></td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_section_title_colspan_not_header] output:\n{}", out);
        assert!(!out.contains("<table>"), "should not be raw HTML, got: {}", out);
        // must NOT treat as header (would produce 40GE 以太网光接口板=LST7L12QX6E0；...)
        assert!(!out.contains("40GE 以太网光接口板=LST7L12QX6E0"), "should NOT be KV format, got: {}", out);
        // must use record mode: key：val1，val2 format
        assert!(!out.contains("记录1："), "should NOT have record prefix, got: {}", out);
        assert!(out.contains("LST7L12QX6E0：12 端口 40GE"), "got: {}", out);
        assert!(out.contains("LST7L32QX6E0*"), "got: {}", out);
    }

    /// 节标题 + 真实表头（第二行是真正的列名）
    #[test]
    fn test_section_title_then_real_header() {
        let html = concat!(
            "<table>",
            "<tr><td colspan=\"3\">产品规格表</td></tr>",
            "<tr><td>类别</td><td>型号</td><td>描述</td></tr>",
            "<tr><td>交换单元</td><td>LST7SFUEX100</td><td>S12700E交换网单元E(X1)</td></tr>",
            "<tr><td>交换单元</td><td>LST7SFUHX100</td><td>S12700E交换网单元H(X1)</td></tr>",
            "</table>"
        );
        let out = html_table_to_kv(html);
        eprintln!("[test_section_title_then_real_header] output:\n{}", out);
        assert!(!out.contains("<table>"), "should not be raw HTML, got: {}", out);
        // Row 0 is section title (all same), row 1 is real header (类别/型号/描述)
        // Should output: 类别=交换单元；型号=LST7SFUEX100；描述=S12700E...
        assert!(out.contains("类别=交换单元"), "should use row 1 as header, got: {}", out);
        assert!(out.contains("型号=LST7SFUEX100"), "got: {}", out);
        assert!(out.contains("描述=S12700E交换网单元E(X1)"), "got: {}", out);
        assert!(out.contains("型号=LST7SFUHX100"), "got: {}", out);
        // Should NOT contain section title in KV output
        assert!(!out.contains("产品规格表"), "should NOT include section title, got: {}", out);
    }

    #[test]
    fn test_markdown_table_to_kv_simple() {
        let md = "| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 10 |\n| 香蕉 | 20 |";
        let out = markdown_table_to_kv(md);
        assert!(out.contains("名称=苹果"), "got: {}", out);
        assert!(out.contains("数量=10"), "got: {}", out);
        assert!(out.contains("名称=香蕉"), "got: {}", out);
        assert!(out.contains("数量=20"), "got: {}", out);
    }

    #[test]
    fn test_markdown_table_to_kv_no_separator() {
        let md = "| 名称 | 数量 |\n| 苹果 | 10 |";
        let out = markdown_table_to_kv(md);
        assert!(out.contains("名称=苹果"), "got: {}", out);
        assert!(out.contains("数量=10"), "got: {}", out);
    }

    #[test]
    fn test_markdown_table_to_kv_record_mode() {
        // All rows are data (no distinct header)
        let md = "| LST7SFUEX100 | S12700E交换网单元E(X1) |\n| LST7SFUHX100 | S12700E交换网单元E(X2) |";
        let out = markdown_table_to_kv(md);
        eprintln!("[test_markdown_table_to_kv_record_mode] output:\n{}", out);
        // Should produce record-style output
        assert!(!out.contains("<table>"), "should not be HTML, got: {}", out);
        assert!(out.contains("LST7SFUEX100"), "got: {}", out);
    }

    #[test]
    fn test_markdown_table_to_kv_empty_cells() {
        let md = "| 类别 | 说明 |\n| --- | --- |\n| MPLS | |\n| | 支持 QoS |";
        let out = markdown_table_to_kv(md);
        assert!(out.contains("类别=MPLS"), "got: {}", out);
        assert!(out.contains("说明="), "got: {}", out);
    }
}
