pub struct CleanContext { pub file_extension: String, pub file_name: String }
pub trait TextTransformer { fn transform(&self, ctx: &CleanContext, input: String) -> String; }
pub struct HtmlToMarkdownTransformer;
impl HtmlToMarkdownTransformer { pub fn new() -> Self { Self } }
impl TextTransformer for HtmlToMarkdownTransformer { fn transform(&self, _: &CleanContext, input: String) -> String { input.replace("<br>","\n").replace("<p>","\n").replace("</p>","") } }
pub struct DataPipeline;
impl DataPipeline { pub fn process_file_stream(&self, _table: &str, _id: &str, text: String) -> String { text } }
pub fn get_pipeline() -> DataPipeline { DataPipeline }

/// 清除 HTML 标签，用于 notes/todos 等富文本内容预处理
/// 块级元素替换为换行，再剥离所有 <> 标签，合并连续空行
pub fn strip_html_for_chunking(html: &str) -> String {
    // 块级元素 → 换行（保留段落边界，否则整篇变一坨）
    let with_newlines = html
        .replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n")
        .replace("</p>", "\n").replace("<p>", "")
        .replace("</div>", "\n").replace("<div>", "")
        .replace("</h1>", "\n").replace("<h1>", "").replace("</h2>", "\n").replace("<h2>", "")
        .replace("</h3>", "\n").replace("<h3>", "").replace("</h4>", "\n").replace("<h4>", "")
        .replace("</h5>", "\n").replace("<h5>", "").replace("</h6>", "\n").replace("<h6>", "")
        .replace("</li>", "\n").replace("<li>", "")
        .replace("</blockquote>", "\n").replace("<blockquote>", "\n");
    // 去剩余 <> 标签
    let mut out = String::with_capacity(with_newlines.len());
    let mut in_tag = false;
    for c in with_newlines.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // 合并连续空行为单行
    let trimmed: String = out.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if trimmed.is_empty() { String::new() } else { trimmed }
}
