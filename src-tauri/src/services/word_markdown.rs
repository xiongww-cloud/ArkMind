//! Word (`.doc`/`.docx`) 文档模型 → Markdown 渲染器。
//!
//! rwml 自带的 `to_markdown()` 会产出较多格式噪音：
//! 相邻加粗 run 渲染成 `**a****b**`、列表项之间强制空行、有序列表编号全变 `1.`、
//! 页/分节符输出字面量 `\pagebreak`、图片输出空链接 `![image]()`、表格首行被强制当表头。
//! 本模块基于 rwml 公开的 `DocModel` 自行渲染 GFM，面向知识库入库与阅读优化：
//!
//!   - 合并相邻同样式 run 后再包强调（字体/字号等未渲染元数据差异不再打断加粗）
//!   - 列表项之间单换行紧凑排列；有序列表保留 Word 编号（`ListInfo.label`）
//!   - 页/分节符 → `---`；图表 → `【图表：…】`；图片 → 占位符（由调用方替换为注记）
//!   - 表格恒输出 HTML `<table>`（与 Excel 输出及下游 `html_table_to_kv` 主链路统一）：
//!     colspan/rowspan 无损；按 `header_rows` / `is_header` / 首行加粗判定 `<th>` 与 `<thead>`；
//!     单元格内多段落/列表以 `<br>` 压平（列表符号保留为文本）
//!   - 转义克制：正文不转义 `<` `>` `|`，减少反斜杠噪音

use rwml::{Block, Cell, Chart, DocModel, FieldRole, Image, ListInfo, Paragraph, Run, Table};

/// 图片占位信息（按渲染顺序登记，由调用方替换为注记）。
#[derive(Debug, Clone)]
pub struct PendingImage {
    /// Markdown 中的占位符（如 `@@IMG0@@`），调用方按此精确替换。
    pub placeholder: String,
    /// 原始 alt 文本，注记兜底说明用。
    pub alt: Option<String>,
}

/// 渲染结果：Markdown 正文 + 图片登记表。
#[derive(Debug, Clone)]
pub struct RenderedWord {
    /// GFM 正文（图片处为占位符）。
    pub markdown: String,
    /// 按出现顺序登记的图片。
    pub images: Vec<PendingImage>,
}

/// 渲染入口。
pub fn render(model: &DocModel) -> RenderedWord {
    let mut r = Renderer::default();
    r.render_blocks(&model.blocks);
    let images = std::mem::take(&mut r.images);
    RenderedWord {
        markdown: r.finish(),
        images,
    }
}

/// 无 OCR 结果时的图片注记（有实质 alt 则带上）。
pub fn image_note_fallback(alt: Option<&str>) -> String {
    match alt.map(str::trim) {
        Some(a) if !a.is_empty() && a != "image" => {
            format!("【图片：{}】", a.replace('|', "丨"))
        }
        _ => "【图片】".to_string(),
    }
}

fn chart_note(chart: &Chart) -> String {
    match chart.title.as_deref().or(chart.alt.as_deref()).map(str::trim) {
        Some(t) if !t.is_empty() && t != "chart" => {
            format!("【图表：{}】", t.replace('|', "丨"))
        }
        _ => "【图表】".to_string(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Rule,
    ListItem,
    Other,
}

#[derive(Clone)]
struct Fragment {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    field: FieldRole,
    image: Option<Image>,
}

impl Fragment {
    /// 强调产生的 `*` 定界符数量（0 = 无星号强调）。
    fn star_len(&self) -> usize {
        match (self.bold, self.italic) {
            (true, true) => 3,
            (true, false) => 2,
            (false, true) => 1,
            (false, false) => 0,
        }
    }

    fn style_key(&self) -> (bool, bool, bool) {
        (self.bold, self.italic, self.strike)
    }
}

struct ListLevel {
    ordered: bool,
    counter: u32,
}

#[derive(Default)]
struct Renderer {
    out: String,
    images: Vec<PendingImage>,
    img_seq: usize,
    stack: Vec<ListLevel>,
    last_kind: Option<BlockKind>,
}

impl Renderer {
    fn render_blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Paragraph(p) => self.render_paragraph(p),
                Block::Table(t) => {
                    self.close_lists();
                    let chunk = self.render_table_html(t);
                    self.push_block(&chunk, BlockKind::Other);
                }
                Block::Image(img) => {
                    self.close_lists();
                    let token = self.image_token(img);
                    self.push_block(&token, BlockKind::Other);
                }
                Block::Chart(chart) => {
                    self.close_lists();
                    let note = chart_note(chart);
                    self.push_block(&note, BlockKind::Other);
                }
                Block::PageBreak | Block::SectionBreak(_) => {
                    self.close_lists();
                    if self.out.is_empty() || self.last_kind == Some(BlockKind::Rule) {
                        continue;
                    }
                    self.push_block("---", BlockKind::Rule);
                }
            }
        }
    }

    fn render_paragraph(&mut self, p: &Paragraph) {
        if p.is_blank() {
            return;
        }
        if p.props.page_break_before
            && !self.out.is_empty()
            && self.last_kind != Some(BlockKind::Rule)
        {
            self.close_lists();
            self.push_block("---", BlockKind::Rule);
        }

        let is_list = p.props.list.is_some();
        if !is_list {
            self.close_lists();
        }

        // 标题：优先样式映射的 heading_level，其次 Word 大纲级别（0=h1）兜底（列表项除外）
        let heading = p.props.heading_level.or_else(|| {
            if p.props.list.is_some() {
                return None;
            }
            p.props
                .outline_level
                .filter(|o| *o <= 8)
                .map(|o| (o + 1).min(6))
        });
        if let Some(level) = heading {
            let content = self.render_inline(&p.runs);
            if content.trim().is_empty() {
                return;
            }
            let hashes = "#".repeat(level.clamp(1, 6) as usize);
            self.push_block(&format!("{hashes} {content}"), BlockKind::Other);
            return;
        }

        if let Some(list) = &p.props.list {
            let mut content = self.render_inline(&p.runs);
            guard_block_start(&mut content);
            self.enter_list(list);
            if content.trim().is_empty() {
                return;
            }
            // 非数字编号（一、/가. 等）作为文本前缀保留，避免丢失 Word 编号样式
            if list.ordered && label_number(&list.label).is_none() {
                let label = list.label.trim();
                if !label.is_empty() {
                    content = format!("{label} {content}");
                }
            }
            let marker = self.list_marker(list);
            let indent = "  ".repeat(list.level as usize);
            self.push_block(&format!("{indent}{marker}{content}"), BlockKind::ListItem);
            return;
        }

        let mut content = self.render_inline(&p.runs);
        guard_block_start(&mut content);
        if content.trim().is_empty() {
            return;
        }
        self.push_block(&content, BlockKind::Other);
    }

    fn push_block(&mut self, chunk: &str, kind: BlockKind) {
        if chunk.is_empty() {
            return;
        }
        if self.out.is_empty() {
            self.out.push_str(chunk);
        } else if kind == BlockKind::ListItem && self.last_kind == Some(BlockKind::ListItem) {
            self.out.push('\n');
            self.out.push_str(chunk);
        } else {
            self.out.push_str("\n\n");
            self.out.push_str(chunk);
        }
        self.last_kind = Some(kind);
    }

    fn close_lists(&mut self) {
        self.stack.clear();
    }

    fn enter_list(&mut self, list: &ListInfo) {
        let level = list.level as usize;
        if level < self.stack.len() {
            self.stack.truncate(level + 1);
            if self.stack[level].ordered != list.ordered {
                self.stack[level] = ListLevel {
                    ordered: list.ordered,
                    counter: 0,
                };
            }
        } else {
            while self.stack.len() < level {
                self.stack.push(ListLevel {
                    ordered: false,
                    counter: 0,
                });
            }
            self.stack.push(ListLevel {
                ordered: list.ordered,
                counter: 0,
            });
        }
        self.stack[level].counter += 1;
    }

    fn list_marker(&self, list: &ListInfo) -> String {
        if !list.ordered {
            return "- ".to_string();
        }
        if let Some(n) = label_number(&list.label) {
            return format!("{n}. ");
        }
        if !list.label.trim().is_empty() {
            // 非数字编号（一、/가. 等）→ 项目符号，编号文本作为内容前缀保留
            return "- ".to_string();
        }
        let counter = self.stack.last().map(|l| l.counter).unwrap_or(1);
        format!("{counter}. ")
    }

    fn render_inline(&mut self, runs: &[Run]) -> String {
        let frags = merge_runs(runs);
        let mut out = String::new();
        for (i, frag) in frags.iter().enumerate() {
            // 星号强调定界符相接（如 **a** 紧跟 ***b***）会产生 4+ 个连续 `*`，
            // CommonMark 解析歧义 → 后一段改用 HTML 标签
            let collides = i > 0
                && frags[i - 1].star_len() + frag.star_len() >= 4
                && frag.image.is_none();
            out.push_str(&self.render_fragment(frag, collides));
        }
        out
    }

    fn render_fragment(&mut self, frag: &Fragment, html_emphasis: bool) -> String {
        let inner = match &frag.image {
            Some(img) => self.image_token(img),
            None => {
                let escaped = escape_inline(&frag.text);
                if frag.style_key() == (false, false, false) || frag.text.trim().is_empty() {
                    escaped
                } else if html_emphasis {
                    wrap_emphasis_html(&escaped, frag)
                } else {
                    wrap_emphasis(&escaped, frag)
                }
            }
        };
        match &frag.field {
            FieldRole::Hyperlink { url } if !url.is_empty() => {
                let text = if inner.trim().is_empty() {
                    escape_url(url)
                } else {
                    inner
                };
                format!("[{}]({})", text, escape_url(url))
            }
            _ => inner,
        }
    }

    fn image_token(&mut self, img: &Image) -> String {
        if img.bytes.is_none() {
            return image_note_fallback(img.alt.as_deref());
        }
        let placeholder = format!("@@IMG{}@@", self.img_seq);
        self.img_seq += 1;
        self.images.push(PendingImage {
            placeholder: placeholder.clone(),
            alt: img.alt.clone(),
        });
        placeholder
    }

    // ── 表格（恒 HTML，与 Excel 输出及 html_table_to_kv 主链路统一）──

    fn render_table_html(&mut self, t: &Table) -> String {
        // 表头行数：Word 标记的重复表头行；否则按首行加粗/is_header 启发式判定
        let header_rows = if t.header_rows > 0 {
            t.header_rows.min(t.rows.len())
        } else if row0_looks_like_header(t) {
            1
        } else {
            0
        };
        let mut out = String::from("<table>");
        for (i, row) in t.rows.iter().enumerate() {
            if i == 0 && header_rows > 0 {
                out.push_str("<thead>");
            }
            if i == header_rows {
                out.push_str(if header_rows > 0 {
                    "</thead><tbody>"
                } else {
                    "<tbody>"
                });
            }
            out.push_str("<tr>");
            for cell in &row.cells {
                let header = i < header_rows || cell.is_header;
                let tag = if header { "th" } else { "td" };
                let mut attrs = String::new();
                if cell.col_span > 1 {
                    attrs.push_str(&format!(" colspan=\"{}\"", cell.col_span));
                }
                if cell.row_span > 1 {
                    attrs.push_str(&format!(" rowspan=\"{}\"", cell.row_span));
                }
                out.push_str(&format!("<{tag}{attrs}>{}</{tag}>", self.cell_html(cell)));
            }
            out.push_str("</tr>");
        }
        if t.rows.len() <= header_rows && header_rows > 0 {
            out.push_str("</thead>");
        } else if !t.rows.is_empty() {
            out.push_str("</tbody>");
        }
        out.push_str("</table>");
        out
    }

    /// 单元格内容：块间以 `<br>` 压平（多段落/列表项各占一段，列表符号保留为文本）；
    /// 嵌套表递归输出 HTML。
    fn cell_html(&mut self, cell: &Cell) -> String {
        let mut parts = Vec::new();
        for block in &cell.blocks {
            match block {
                Block::Paragraph(p) => {
                    if p.is_blank() {
                        continue;
                    }
                    let mut s = self.runs_html(&p.runs);
                    if let Some(list) = &p.props.list {
                        s = format!("{}{s}", cell_list_marker(list));
                    }
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                }
                Block::Table(t) => parts.push(self.render_table_html(t)),
                Block::Image(img) => parts.push(self.image_token(img)),
                Block::Chart(chart) => parts.push(chart_note(chart)),
                _ => {}
            }
        }
        parts.join("<br>")
    }

    fn runs_html(&mut self, runs: &[Run]) -> String {
        let frags = merge_runs(runs);
        let mut out = String::new();
        for frag in &frags {
            let inner = match &frag.image {
                Some(img) => self.image_token(img),
                None => {
                    let escaped = escape_html(&frag.text).replace('\n', "<br>");
                    if frag.style_key() == (false, false, false) || frag.text.trim().is_empty() {
                        escaped
                    } else {
                        wrap_emphasis_html(&escaped, frag)
                    }
                }
            };
            match &frag.field {
                FieldRole::Hyperlink { url } if !url.is_empty() => {
                    out.push_str(&format!(
                        "<a href=\"{}\">{}</a>",
                        escape_html(url),
                        inner
                    ));
                }
                _ => out.push_str(&inner),
            }
        }
        out
    }

    fn finish(self) -> String {
        // 行尾空白清理 + 连续空行压成一行 + 去首尾空行
        let mut lines: Vec<String> = self
            .out
            .lines()
            .map(|l| l.trim_end().to_string())
            .collect();
        let mut res: Vec<String> = Vec::with_capacity(lines.len());
        let mut blank = 0usize;
        for line in lines.drain(..) {
            if line.is_empty() {
                blank += 1;
                if blank == 1 {
                    res.push(String::new());
                }
            } else {
                blank = 0;
                res.push(line);
            }
        }
        while res.first().map(|l| l.is_empty()).unwrap_or(false) {
            res.remove(0);
        }
        while res.last().map(|l| l.is_empty()).unwrap_or(false) {
            res.pop();
        }
        // 文末残留分页线无意义
        while res.last().map(|l| l.as_str()) == Some("---") {
            res.pop();
            if res.last().map(|l| l.is_empty()).unwrap_or(false) {
                res.pop();
            }
        }
        res.join("\n")
    }
}

// ── run 合并与强调 ──

/// 合并相邻同样式 run。样式键只看会影响 Markdown 渲染的属性
/// （粗/斜/删除线/字段角色），字体、字号、颜色等元数据差异不打断合并 ——
/// 这正是 Word 把同一段加粗文本拆成多个 run 后产生 `**a****b**` 的根源。
fn merge_runs(runs: &[Run]) -> Vec<Fragment> {
    let mut frags: Vec<Fragment> = Vec::with_capacity(runs.len());
    for run in runs {
        if let Some(image) = &run.image {
            frags.push(Fragment {
                text: String::new(),
                bold: false,
                italic: false,
                strike: false,
                field: FieldRole::None,
                image: Some(image.clone()),
            });
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let frag = Fragment {
            text: run.text.clone(),
            bold: run.props.bold,
            italic: run.props.italic,
            strike: run.props.strike,
            field: run.field.clone(),
            image: None,
        };
        let mergeable = frags
            .last_mut()
            .filter(|last| {
                last.image.is_none()
                    && last.style_key() == frag.style_key()
                    && last.field == frag.field
            });
        match mergeable {
            Some(last) => last.text.push_str(&frag.text),
            None => frags.push(frag),
        }
    }
    frags
}

fn split_outer_whitespace(text: &str) -> (&str, &str, &str) {
    let start = text.len() - text.trim_start().len();
    let (lead, rest) = text.split_at(start);
    let core_len = rest.trim_end().len();
    let (core, tail) = rest.split_at(core_len);
    (lead, core, tail)
}

fn wrap_emphasis(escaped: &str, frag: &Fragment) -> String {
    let (lead, core, tail) = split_outer_whitespace(escaped);
    if core.is_empty() {
        return escaped.to_string();
    }
    let mut s = String::with_capacity(escaped.len() + 8);
    s.push_str(lead);
    if frag.strike {
        s.push_str("~~");
    }
    match (frag.bold, frag.italic) {
        (true, true) => s.push_str("***"),
        (true, false) => s.push_str("**"),
        (false, true) => s.push('*'),
        (false, false) => {}
    }
    s.push_str(core);
    match (frag.bold, frag.italic) {
        (true, true) => s.push_str("***"),
        (true, false) => s.push_str("**"),
        (false, true) => s.push('*'),
        (false, false) => {}
    }
    if frag.strike {
        s.push_str("~~");
    }
    s.push_str(tail);
    s
}

fn wrap_emphasis_html(escaped: &str, frag: &Fragment) -> String {
    let (lead, core, tail) = split_outer_whitespace(escaped);
    if core.is_empty() {
        return escaped.to_string();
    }
    let mut s = core.to_string();
    if frag.strike {
        s = format!("<s>{s}</s>");
    }
    if frag.bold {
        s = format!("<strong>{s}</strong>");
    }
    if frag.italic {
        s = format!("<em>{s}</em>");
    }
    format!("{lead}{s}{tail}")
}

// ── 表格辅助 ──

fn row0_looks_like_header(t: &Table) -> bool {
    let Some(row) = t.rows.first() else { return false };
    if row.cells.is_empty() {
        return false;
    }
    let mut seen = 0usize;
    for cell in &row.cells {
        let text = cell.text();
        if text.trim().is_empty() {
            continue;
        }
        seen += 1;
        if !cell.is_header && !cell_text_all_bold(cell) {
            return false;
        }
    }
    seen > 0 && seen * 2 >= row.cells.len()
}

fn cell_text_all_bold(cell: &Cell) -> bool {
    let mut has_text = false;
    for block in &cell.blocks {
        if let Block::Paragraph(p) = block {
            for run in &p.runs {
                if run.text.trim().is_empty() {
                    continue;
                }
                has_text = true;
                if !run.props.bold {
                    return false;
                }
            }
        }
    }
    has_text
}

/// 从列表标签提取纯数字编号（`1.` / `2)` / `3、` → 3）。
fn label_number(label: &str) -> Option<u32> {
    let trimmed = label.trim().trim_end_matches(['.', ')', '、', '．', ':', '：', '）', '】', '」']);
    if trimmed.is_empty() || trimmed.chars().count() > 9 {
        return None;
    }
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    trimmed.parse().ok()
}

/// 表格单元格内的列表符号文本（HTML `<br>` 压平后保留为纯文本前缀）。
fn cell_list_marker(list: &ListInfo) -> String {
    if !list.ordered {
        "- ".to_string()
    } else if let Some(n) = label_number(&list.label) {
        format!("{n}. ")
    } else if !list.label.trim().is_empty() {
        format!("{} ", list.label.trim())
    } else {
        "1. ".to_string()
    }
}

// ── 转义 ──

/// 行内转义。只转义真正会产生 Markdown 语义的字符：
/// `\` `` ` `` `*` `[` `]` 必转；`_` 仅在词边界转义（`max_size` 保持原样）；
/// `<` 仅在形如标签开头时转义（`a < b` 保持原样）。
/// 换行/分栏符 → `<br>`，制表符 → 空格。
fn escape_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        match c {
            '\\' | '`' | '*' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '_' => {
                let intraword = prev.map(|p| p.is_alphanumeric()).unwrap_or(false)
                    && next.map(|n| n.is_alphanumeric()).unwrap_or(false);
                if !intraword {
                    out.push('\\');
                }
                out.push('_');
            }
            '<' => {
                let taggy = matches!(
                    next,
                    Some(n) if n.is_ascii_alphabetic() || n == '/' || n == '!' || n == '?'
                );
                if taggy {
                    out.push('\\');
                }
                out.push('<');
            }
            '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{0E}' => out.push_str("<br>"),
            '\t' => out.push(' '),
            '\u{FEFF}' | '\u{FFFE}' => {}
            c if c.is_ascii_control() => {}
            _ => out.push(c),
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push(' '),
            c if c.is_ascii_control() && c != '\n' && c != '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

fn escape_url(s: &str) -> String {
    s.replace(' ', "%20").replace(')', "%29").replace('(', "%28")
}

/// 普通段落首字符若会被解析成块级语法（标题/列表/引用/分隔线…）则转义，
/// 避免正文被 Markdown 渲染器误认。
fn guard_block_start(content: &mut String) {
    let t = content.trim_start();
    if t.is_empty() {
        return;
    }
    let head = t.as_bytes();
    let mut dangerous = false;
    if head[0] == b'#' {
        let rest = &t[1..];
        dangerous = rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t');
    } else if head[0] == b'>' || head[0] == b'|' {
        dangerous = true;
    } else if matches!(head[0], b'-' | b'*' | b'+') {
        let rest = &t[1..];
        dangerous = rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t');
    } else if head[0].is_ascii_digit() {
        let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() <= 9 {
            let after = &t[digits.len()..];
            if after.starts_with('.') || after.starts_with(')') {
                let rest = &after[1..];
                dangerous = rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t');
            }
        }
    }
    if !dangerous {
        // 整行只有 - _ * =（分隔线 / setext 下划线），或 ` - - - ` 样式分隔线
        let trimmed = t.trim();
        if trimmed.chars().count() >= 3 {
            let first = trimmed.chars().next().unwrap();
            let all_same = trimmed.chars().all(|c| c == first)
                && matches!(first, '-' | '_' | '*' | '=');
            let dash_rule = trimmed.chars().all(|c| c == '-' || c == ' ' || c == '\t')
                && trimmed.chars().filter(|c| *c == '-').count() >= 3;
            dangerous = all_same || dash_rule;
        }
    }
    if dangerous {
        let offset = content.len() - t.len();
        content.insert(offset, '\\');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rwml::{CharProps, DocMeta, ParaProps, Row};

    fn text_run(text: &str) -> Run {
        Run {
            text: text.to_string(),
            ..Default::default()
        }
    }

    fn styled_run(text: &str, props: CharProps) -> Run {
        Run {
            text: text.to_string(),
            props,
            ..Default::default()
        }
    }

    fn bold_run(text: &str) -> Run {
        styled_run(
            text,
            CharProps {
                bold: true,
                ..Default::default()
            },
        )
    }

    fn para(runs: Vec<Run>) -> Block {
        Block::Paragraph(Paragraph {
            props: ParaProps::default(),
            runs,
        })
    }

    fn list_item(text: &str, level: u8, ordered: bool, label: &str) -> Block {
        Block::Paragraph(Paragraph {
            props: ParaProps {
                list: Some(ListInfo {
                    level,
                    ordered,
                    label: label.to_string(),
                }),
                ..Default::default()
            },
            runs: vec![text_run(text)],
        })
    }

    fn cell(text: &str) -> Cell {
        Cell {
            blocks: vec![para(vec![text_run(text)])],
            ..Default::default()
        }
    }

    fn render_blocks(blocks: Vec<Block>) -> String {
        render(&DocModel {
            blocks,
            meta: DocMeta::default(),
            ..Default::default()
        })
        .markdown
    }

    #[test]
    fn adjacent_bold_runs_merge_without_star_glue() {
        let md = render_blocks(vec![para(vec![
            bold_run("加粗一"),
            bold_run("加粗二"),
            text_run("普通"),
            bold_run("再加粗"),
        ])]);
        assert_eq!(md, "**加粗一加粗二**普通**再加粗**");
    }

    #[test]
    fn font_metadata_gap_does_not_split_emphasis() {
        let a = styled_run(
            "连贯",
            CharProps {
                bold: true,
                font: Some("宋体".into()),
                size_half_pt: Some(24),
                ..Default::default()
            },
        );
        let b = styled_run(
            "加粗",
            CharProps {
                bold: true,
                font: Some("黑体".into()),
                size_half_pt: Some(21),
                ..Default::default()
            },
        );
        assert_eq!(render_blocks(vec![para(vec![a, b])]), "**连贯加粗**");
    }

    #[test]
    fn mixed_emphasis_without_four_star_run() {
        let md = render_blocks(vec![para(vec![
            bold_run("粗"),
            styled_run(
                "粗斜",
                CharProps {
                    bold: true,
                    italic: true,
                    ..Default::default()
                },
            ),
        ])]);
        assert!(!md.contains("****"), "{md}");
        assert!(md.contains("<strong>"), "{md}");
    }

    #[test]
    fn tight_nested_lists() {
        let md = render_blocks(vec![
            para(vec![text_run("前言")]),
            list_item("甲", 0, false, ""),
            list_item("乙", 0, false, ""),
            list_item("乙一", 1, false, ""),
            list_item("丙", 0, false, ""),
        ]);
        assert_eq!(md, "前言\n\n- 甲\n- 乙\n  - 乙一\n- 丙");
    }

    #[test]
    fn ordered_list_keeps_word_numbers() {
        let md = render_blocks(vec![
            list_item("一", 0, true, "3."),
            list_item("二", 0, true, "4."),
        ]);
        assert_eq!(md, "3. 一\n4. 二");
    }

    #[test]
    fn ordered_list_falls_back_to_counter() {
        let md = render_blocks(vec![
            list_item("一", 0, true, ""),
            list_item("二", 0, true, ""),
        ]);
        assert_eq!(md, "1. 一\n2. 二");
    }

    #[test]
    fn non_numeric_label_preserved_as_prefix() {
        let md = render_blocks(vec![list_item("条目", 0, true, "一、")]);
        assert_eq!(md, "- 一、 条目");
    }

    #[test]
    fn table_uses_declared_header_rows() {
        let t = Table {
            rows: vec![
                Row {
                    cells: vec![cell("名"), cell("值")],
                },
                Row {
                    cells: vec![cell("容量"), cell("128GB")],
                },
            ],
            header_rows: 1,
            ..Default::default()
        };
        assert_eq!(
            render_blocks(vec![Block::Table(t)]),
            "<table><thead><tr><th>名</th><th>值</th></tr></thead>\
             <tbody><tr><td>容量</td><td>128GB</td></tr></tbody></table>"
        );
    }

    #[test]
    fn table_without_header_is_all_td() {
        let t = Table {
            rows: vec![
                Row {
                    cells: vec![cell("甲"), cell("1")],
                },
                Row {
                    cells: vec![cell("乙"), cell("2")],
                },
            ],
            header_rows: 0,
            ..Default::default()
        };
        let md = render_blocks(vec![Block::Table(t)]);
        assert_eq!(
            md,
            "<table><tbody><tr><td>甲</td><td>1</td></tr><tr><td>乙</td><td>2</td></tr></tbody></table>"
        );
    }

    #[test]
    fn bold_first_row_detected_as_header() {
        let header = |t: &str| Cell {
            blocks: vec![para(vec![bold_run(t)])],
            ..Default::default()
        };
        let t = Table {
            rows: vec![
                Row {
                    cells: vec![header("名"), header("值")],
                },
                Row {
                    cells: vec![cell("甲"), cell("1")],
                },
            ],
            header_rows: 0,
            ..Default::default()
        };
        let md = render_blocks(vec![Block::Table(t)]);
        assert_eq!(
            md,
            "<table><thead><tr><th><strong>名</strong></th><th><strong>值</strong></th></tr></thead>\
             <tbody><tr><td>甲</td><td>1</td></tr></tbody></table>"
        );
    }

    #[test]
    fn merged_cell_preserves_spans() {
        let wide = Cell {
            blocks: vec![para(vec![text_run("横跨")])],
            col_span: 2,
            ..Default::default()
        };
        let t = Table {
            rows: vec![Row {
                cells: vec![wide, cell("尾")],
            }],
            header_rows: 0,
            ..Default::default()
        };
        let md = render_blocks(vec![Block::Table(t)]);
        assert!(md.contains("<table>"), "{md}");
        assert!(md.contains("colspan=\"2\""), "{md}");
        assert!(md.contains("</table>"), "{md}");
    }

    #[test]
    fn cell_multiline_flattens_with_br_and_list_markers() {
        let multi = Cell {
            blocks: vec![
                para(vec![text_run("第一行")]),
                para(vec![text_run("第二行")]),
                Block::Paragraph(Paragraph {
                    props: ParaProps {
                        list: Some(ListInfo {
                            level: 0,
                            ordered: false,
                            label: String::new(),
                        }),
                        ..Default::default()
                    },
                    runs: vec![text_run("要点A")],
                }),
                Block::Paragraph(Paragraph {
                    props: ParaProps {
                        list: Some(ListInfo {
                            level: 1,
                            ordered: true,
                            label: "2.".into(),
                        }),
                        ..Default::default()
                    },
                    runs: vec![text_run("要点B")],
                }),
            ],
            ..Default::default()
        };
        let t = Table {
            rows: vec![Row { cells: vec![multi] }],
            header_rows: 0,
            ..Default::default()
        };
        let md = render_blocks(vec![Block::Table(t)]);
        assert!(md.contains("第一行<br>第二行<br>- 要点A<br>2. 要点B"), "{md}");
    }

    #[test]
    fn page_break_becomes_rule_and_dedupes() {
        use rwml::ParaProps as PP;
        let md = render_blocks(vec![
            para(vec![text_run("封面")]),
            Block::PageBreak,
            Block::PageBreak,
            Block::Paragraph(Paragraph {
                props: PP {
                    page_break_before: true,
                    ..Default::default()
                },
                runs: vec![text_run("正文")],
            }),
        ]);
        assert_eq!(md, "封面\n\n---\n\n正文");
    }

    #[test]
    fn images_register_placeholders_in_order() {
        let img = Image {
            bytes: Some(vec![1, 2, 3]),
            mime: Some("image/png".into()),
            alt: Some("图一".into()),
            ..Default::default()
        };
        let out = render(&DocModel {
            blocks: vec![
                Block::Image(img.clone()),
                para(vec![Run {
                    image: Some(img),
                    ..Default::default()
                }]),
            ],
            meta: DocMeta::default(),
            ..Default::default()
        });
        assert_eq!(out.markdown, "@@IMG0@@\n\n@@IMG1@@");
        assert_eq!(out.images.len(), 2);
        assert_eq!(out.images[0].placeholder, "@@IMG0@@");
        assert_eq!(out.images[1].placeholder, "@@IMG1@@");
        assert_eq!(image_note_fallback(Some("图一")), "【图片：图一】");
        assert_eq!(image_note_fallback(Some("image")), "【图片】");
    }

    #[test]
    fn escaping_is_minimal() {
        let md = render_blocks(vec![para(vec![text_run("比较 a < b 与 max_size，引用 <tag> 与 _em_")])]);
        assert!(md.contains("a < b"), "{md}");
        assert!(md.contains("max_size"), "{md}");
        assert!(md.contains("\\<tag>"), "{md}");
        assert!(md.contains("\\_em\\_"), "{md}");
    }

    #[test]
    fn paragraph_starting_with_block_syntax_is_escaped() {
        let md = render_blocks(vec![
            para(vec![text_run("- 看起来像列表")]),
            para(vec![text_run("# 看起来像标题")]),
            para(vec![text_run("---")]),
        ]);
        assert_eq!(md, "\\- 看起来像列表\n\n\\# 看起来像标题\n\n\\---");
    }

    #[test]
    fn hyperlink_merges_runs_and_renders_link() {
        let link = |t: &str| Run {
            text: t.to_string(),
            field: FieldRole::Hyperlink {
                url: "https://x.io".to_string(),
            },
            ..Default::default()
        };
        let md = render_blocks(vec![para(vec![link("网"), link("站")])]);
        assert_eq!(md, "[网站](https://x.io)");
    }

    #[test]
    fn chart_becomes_note() {
        let chart = Chart {
            title: Some("趋势".into()),
            ..Default::default()
        };
        assert_eq!(render_blocks(vec![Block::Chart(chart)]), "【图表：趋势】");
    }

    /// 端到端冒烟：手工构造最小合法 .docx（zip + document.xml），
    /// 验证 rwml 解析 → 模型 → 本渲染器全链路。
    #[test]
    fn docx_bytes_roundtrip_smoke() {
        use std::io::Write;

        let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        let document = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>第一章</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>连贯</w:t></w:r><w:r><w:rPr><w:b/><w:i/></w:rPr><w:t>加粗</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>条目一</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>条目二</w:t></w:r></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>名</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>值</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>容量</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>128GB</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
</w:body>
</w:document>"#;

        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::FileOptions::default();
            zw.start_file("[Content_Types].xml", opts).unwrap();
            zw.write_all(content_types.as_bytes()).unwrap();
            zw.start_file("_rels/.rels", opts).unwrap();
            zw.write_all(rels.as_bytes()).unwrap();
            zw.start_file("word/document.xml", opts).unwrap();
            zw.write_all(document.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let bytes = cursor.into_inner();

        let doc = rwml::Document::open(&bytes).expect("最小 docx 应可解析");
        let out = render(&doc.model());
        // 标题（outlineLvl 0 → #）；样式表齐全时走 heading_level，此处验证大纲兜底
        assert!(out.markdown.contains("# 第一章"), "{}", out.markdown);
        // 相邻强调 run 干净衔接（不产生 ****，星号碰撞自动转 HTML 标签）
        assert!(!out.markdown.contains("****"), "{}", out.markdown);
        assert!(out.markdown.contains("连贯"), "{}", out.markdown);
        // 列表紧凑（两项之间无空行）
        assert!(out.markdown.contains("条目一\n"), "{}", out.markdown);
        // 表格恒 HTML（无表头 → 纯 td，与 Excel 输出一致）
        assert!(
            out.markdown
                .contains("<table><tbody><tr><td>名</td><td>值</td></tr>"),
            "{}",
            out.markdown
        );
        assert!(out.markdown.contains("<td>128GB</td>"), "{}", out.markdown);
    }
}
