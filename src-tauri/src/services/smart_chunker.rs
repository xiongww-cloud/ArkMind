//! Structure-aware Hierarchical Chunker v2
//!
//! Document → Block → Section → Parent Chunk → Focused Child Chunk

// ── Constants ──
pub const COMPACT_THRESHOLD: usize = 350;
pub const MAX_PARENT: usize = 1800;
pub const FOCUSED_TARGET: usize = 350;

// ── Block types ──
#[derive(Debug, Clone, PartialEq)]
pub enum BlockType {
    Heading, Paragraph, Table, List, Code, Quote, HorizontalRule, Empty,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub block_type: BlockType,
    pub level: Option<u8>,
    pub text: String,
    pub chars: usize,
}

// ── Section ──
#[derive(Debug, Clone)]
pub struct Section {
    pub heading: Option<Block>,
    pub blocks: Vec<Block>,
    pub children: Vec<Section>,
}

impl Section {
    #[allow(dead_code)]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for b in &self.blocks {
            if !out.is_empty() { out.push_str("\n\n"); }
            out.push_str(&b.text);
        }
        for child in &self.children {
            if !out.is_empty() { out.push_str("\n\n"); }
            if let Some(ref h) = child.heading { out.push_str(&h.text); }
            let ct = child.text();
            if !ct.is_empty() { out.push_str("\n\n"); out.push_str(&ct); }
        }
        out
    }
    pub fn chars(&self) -> usize {
        let mut c: usize = self.blocks.iter().map(|b| b.chars).sum();
        for child in &self.children { c += child.chars(); }
        c
    }
}

// ── Line detection ──
fn is_heading(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('#') && t.chars().skip_while(|&c| c == '#').next().map_or(false, |c| c == ' ')
}
fn heading_level(line: &str) -> u8 { line.trim().chars().take_while(|&c| c == '#').count() as u8 }
fn is_code_fence(line: &str) -> bool { line.trim().starts_with("```") }
fn is_horizontal_rule(line: &str) -> bool {
    let t = line.trim(); if t.len() < 3 { return false; }
    let c = t.chars().next().unwrap();
    (c == '-' || c == '*' || c == '_') && t.chars().all(|ch| ch == c || ch == ' ')
}
fn is_table_line(line: &str) -> bool {
    let t = line.trim(); t.contains('|') && t.chars().filter(|&c| c == '|').count() >= 2
}
fn is_table_separator(line: &str) -> bool {
    let t = line.trim(); t.starts_with('|') && t.contains("---")
}
fn is_list_item(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ")
        || t.chars().next().map_or(false, |c| c.is_ascii_digit())
           && t.chars().nth(1).map_or(false, |c| c == '.')
}
fn is_quote_line(line: &str) -> bool { line.trim().starts_with('>') }

// ── Block parser ──
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            blocks.push(Block { block_type: BlockType::Empty, level: None, text: String::new(), chars: 0 });
            i += 1; continue;
        }
        if is_horizontal_rule(line) {
            blocks.push(Block { block_type: BlockType::HorizontalRule, level: None, text: line.trim().to_string(), chars: line.trim().len() });
            i += 1; continue;
        }
        if is_code_fence(line) {
            let mut code_lines = vec![line]; i += 1;
            while i < lines.len() && !is_code_fence(lines[i]) { code_lines.push(lines[i]); i += 1; }
            if i < lines.len() { code_lines.push(lines[i]); i += 1; }
            let text = code_lines.join("\n");
            blocks.push(Block { block_type: BlockType::Code, level: None, text: text.clone(), chars: text.len() });
            continue;
        }
        if is_heading(line) {
            let level = heading_level(line);
            blocks.push(Block { block_type: BlockType::Heading, level: Some(level), text: line.trim().to_string(), chars: line.trim().len() });
            i += 1; continue;
        }
        // HTML table (<table ...> ... </table>), single-line or multi-line
        if line.trim().starts_with("<table") {
            let text;
            if line.contains("</table>") {
                // Single-line table; everything is right here
                text = line.to_string();
                i += 1;
            } else {
                // Multi-line table; consume lines until </table>
                let mut html_lines: Vec<&str> = vec![line]; i += 1;
                while i < lines.len() && !lines[i].trim().starts_with("</table>") {
                    html_lines.push(lines[i]); i += 1;
                }
                if i < lines.len() { html_lines.push(lines[i]); i += 1; }
                text = html_lines.join("\n");
            }
            let tc = text.len();
            blocks.push(Block { block_type: BlockType::Table, level: None, text: text.clone(), chars: tc });
            continue;
        }
        if is_table_line(line) || is_table_separator(line) {
            let mut tbl: Vec<String> = Vec::new();
            while i < lines.len() && (is_table_line(lines[i]) || is_table_separator(lines[i])) {
                tbl.push(lines[i].trim().to_string()); i += 1;
            }
            // Auto-add separator row if missing
            if tbl.len() >= 1 && (tbl.len() < 2 || !tbl[1].contains("---")) {
                let pipes = tbl[0].chars().filter(|&c| c == '|').count();
                let cols = pipes.saturating_sub(1).max(1);
                let sep = format!("|{}|", " --- |".repeat(cols).trim_end_matches('|'));
                tbl.insert(1, sep);
            }
            let text = tbl.join("\n");
            blocks.push(Block { block_type: BlockType::Table, level: None, text: text.clone(), chars: text.len() });
            continue;
        }
        if is_quote_line(line) {
            let mut ql = Vec::new();
            while i < lines.len() && is_quote_line(lines[i]) { ql.push(lines[i].trim().trim_start_matches('>').trim()); i += 1; }
            let text = ql.join("\n");
            blocks.push(Block { block_type: BlockType::Quote, level: None, text: text.clone(), chars: text.len() });
            continue;
        }
        if is_list_item(line) {
            let mut ll = Vec::new();
            while i < lines.len() && (is_list_item(lines[i]) || lines[i].trim().is_empty() || (lines[i].starts_with(' ') && !is_heading(lines[i]) && !is_code_fence(lines[i]))) {
                let t = lines[i].trim();
                if !t.is_empty() { ll.push(t); }
                i += 1;
            }
            let text = ll.join("\n");
            blocks.push(Block { block_type: BlockType::List, level: None, text: text.clone(), chars: text.len() });
            continue;
        }
        // Paragraph
        let mut pl = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() && !is_heading(lines[i]) && !is_code_fence(lines[i]) && !is_horizontal_rule(lines[i]) && !lines[i].trim().starts_with("<table") {
            pl.push(lines[i]); i += 1;
        }
        let text = pl.join("\n").trim().to_string();
        if !text.is_empty() {
            blocks.push(Block { block_type: BlockType::Paragraph, level: None, text: text.clone(), chars: text.len() });
        } else { i += 1; }
    }
    blocks
}

// ── Section builder ──
pub fn build_sections(blocks: Vec<Block>) -> Vec<Section> {
    let mut stack: Vec<(u8, Section)> = Vec::new();
    let mut roots: Vec<Section> = Vec::new();
    for block in blocks {
        match block.block_type {
            BlockType::Heading => {
                let level = block.level.unwrap_or(1);
                while let Some(&(l, _)) = stack.last() {
                    if l >= level {
                        let (_, section) = stack.pop().unwrap();
                        if let Some(ref mut parent) = stack.last_mut() { parent.1.children.push(section); }
                        else { roots.push(section); }
                    } else { break; }
                }
                stack.push((level, Section { heading: Some(block), blocks: Vec::new(), children: Vec::new() }));
            }
            BlockType::Empty | BlockType::HorizontalRule => {}
            _ => {
                if stack.is_empty() { stack.push((0, Section { heading: None, blocks: Vec::new(), children: Vec::new() })); }
                if let Some(ref mut top) = stack.last_mut() { top.1.blocks.push(block); }
            }
        }
    }
    while let Some((_, section)) = stack.pop() {
        if let Some(ref mut parent) = stack.last_mut() { parent.1.children.push(section); }
        else { roots.push(section); }
    }
    roots
}

// ── Flatten ──
fn flatten_section(section: &Section, out: &mut Vec<Block>) {
    if let Some(ref h) = section.heading { if section.chars() > 0 { out.push(h.clone()); } }
    for b in &section.blocks { out.push(b.clone()); }
    for child in &section.children { flatten_section(child, out); }
}

// ── Short-tail merger ──
pub fn merge_short_tails(parents: &mut Vec<String>) {
    if parents.len() <= 1 { return; }
    let mut i = 0;
    while i < parents.len() - 1 {
        let cur_chars = parents[i].chars().count();
        if cur_chars <= COMPACT_THRESHOLD {
            let next_chars = parents[i+1].chars().count();
            if cur_chars + next_chars + 2 <= MAX_PARENT {
                parents[i] = format!("{}\n\n{}", parents[i], parents[i+1]);
                parents.remove(i + 1);
                continue;
            }
        }
        i += 1;
    }
}

// ── Adaptive parent accumulation ──
pub fn accumulate_adaptive_parents(sections: Vec<Section>) -> Vec<String> {
    let mut parents = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    for section in &sections { flatten_section(section, &mut blocks); }
    if blocks.is_empty() { return vec![String::new()]; }

    let mut current = String::new();
    for block in &blocks {
        let block_text = block.text.trim();
        if block_text.is_empty() { continue; }
        let block_chars = block_text.chars().count();

        if block.block_type == BlockType::Table {
            // ── Table: output KV text ──
            let cur_trim = current.trim().to_string();
            let cur_chars = cur_trim.chars().count();
            let kv = if block_text.trim().starts_with("<table") {
                crate::services::semantic_builder::html_table_to_kv(block_text)
            } else {
                crate::services::semantic_builder::markdown_table_to_kv(block_text)
            };

            if cur_chars > 0 && cur_chars <= COMPACT_THRESHOLD {
                if cur_chars > 0 { current.push_str("\n\n"); }
                current.push_str(&kv);
            } else {
                if cur_chars > 0 { parents.push(cur_trim); current = String::new(); }
                if !kv.is_empty() {
                    if !current.is_empty() { current.push_str("\n\n"); }
                    current.push_str(&kv);
                }
            }
        } else if block.block_type == BlockType::Heading {
            if !current.trim().is_empty() && current.trim().chars().count() > COMPACT_THRESHOLD {
                parents.push(current.trim().to_string());
                current = block_text.to_string();
            } else {
                if !current.is_empty() { current.push_str("\n\n"); }
                current.push_str(block_text);
            }
        } else {
            let add = if current.is_empty() { 0 } else { 2 };
            if current.chars().count() + add + block_chars > MAX_PARENT {
                if !current.trim().is_empty() { parents.push(current.trim().to_string()); current = String::new(); }
            }
            if !current.is_empty() { current.push_str("\n\n"); }
            current.push_str(block_text);
        }
    }
    if !current.trim().is_empty() { parents.push(current.trim().to_string()); }
    if parents.is_empty() { parents.push(String::new()); }
    merge_short_tails(&mut parents);
    parents
}

// ── Legacy functions ──
pub fn split_into_parents(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars { return vec![text.to_string()]; }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim();
        let add = if current.is_empty() { t.len() } else { 1 + t.len() };
        if current.chars().count() + add > max_chars {
            if !current.trim().is_empty() { out.push(current.trim().to_string()); } current.clear();
        }
        if !current.is_empty() { current.push('\n'); }
        current.push_str(t);
    }
    if !current.trim().is_empty() { out.push(current.trim().to_string()); }
    if out.is_empty() { out.push(text.to_string()); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blocks_paragraph_before_html_table_no_blank_line() {
        // Reproducing the exact bug: "业务特性" on its own line, followed by
        // <table> on the next line without a blank line between them.
        let text = "业务特性\n<table><tr><td>A</td></tr></table>";
        let blocks = parse_blocks(text);
        // Must not swallow the <table> into a paragraph
        let tables: Vec<&Block> = blocks.iter().filter(|b| b.block_type == BlockType::Table).collect();
        assert_eq!(tables.len(), 1, "should find exactly 1 Table block, got blocks: {:?}", blocks.iter().map(|b| format!("{:?}: {:.30}", b.block_type, b.text)).collect::<Vec<_>>());
        let paragraphs: Vec<&Block> = blocks.iter().filter(|b| b.block_type == BlockType::Paragraph).collect();
        assert_eq!(paragraphs.len(), 1, "should have 1 Paragraph");
        // Paragraph should only contain "业务特性", not the HTML table
        assert_eq!(paragraphs[0].text.trim(), "业务特性");
        // Table block should contain the full <table>...</table>
        assert!(tables[0].text.starts_with("<table"), "Table block should start with <table>");
        assert!(tables[0].text.contains("</table>"), "Table block should contain </table>");
    }

    #[test]
    fn test_parse_blocks_multi_html_tables_no_blanks() {
        let text = "N1模式软件包特性\n<table><tr><td>A</td></tr></table>\n业务特性\n<table><tr><td>B</td></tr></table>";
        let blocks = parse_blocks(text);
        let tables: Vec<&Block> = blocks.iter().filter(|b| b.block_type == BlockType::Table).collect();
        assert_eq!(tables.len(), 2, "should find 2 Table blocks, got: {:?}", blocks.iter().map(|b| format!("{:?}: {:.30}", b.block_type, b.text)).collect::<Vec<_>>());
    }

    #[test]
    fn test_parse_blocks_html_table_standalone() {
        let text = "<table><tr><td>A</td></tr></table>";
        let blocks = parse_blocks(text);
        let tables: Vec<&Block> = blocks.iter().filter(|b| b.block_type == BlockType::Table).collect();
        assert_eq!(tables.len(), 1, "should find exactly 1 Table block");
    }
}
