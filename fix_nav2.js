const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/services/agent_workflow.rs', 'utf8');

// Step 1: Replace function signature and head
c = c.replace(
    `// ═══════════════════════════════════════════════════════════════════

/// 从 source_type / source_id 查询所有关联元数据，返回格式化信息块
pub fn build_chunk_metadata(source_type: &str, source_id: &str) -> String {
    if source_type.is_empty() || source_id.is_empty() { return String::new(); }
    let st = source_type.to_lowercase();
    let sid = source_id;

    let mut lines: Vec<String> = Vec::new();`,
    `// ═══════════════════════════════════════════════════════════════════
// Knowledge Navigation Card — metadata + affordance guide
// ═══════════════════════════════════════════════════════════════════

pub fn build_chunk_navigation_card(source_type: &str, source_id: &str, chunk_id: &str, chunk_index: i64, total_parents: i64) -> String {
    if source_type.is_empty() || source_id.is_empty() { return String::new(); }
    let st = source_type.to_lowercase();
    let sid = source_id;

    let mut lines: Vec<String> = Vec::new();
    let mut shown: std::collections::HashSet<String> = std::collections::HashSet::new();

    let card = |t: &str| -> Vec<String> { match t {
        "files" => vec!["📋 文件信息","- 正文内容全文检索","- 上下文前后漫游","- 同项目/同产品关联文档","🔎 建议：搜索文件内容、traverse 查看上下文"],
        "projects" => vec!["📋 项目信息","- 客户 · 状态 · 预算 · 负责人","- 时间节点 · BOM 清单","🔎 建议：查看项目全景、查看项目动态"],
        "products" => vec!["📋 产品信息","- 招标参数 · 技术规格","- 产品系列 · 描述","🔎 建议：查看产品详细信息"],
        "notes" => vec!["📋 笔记信息","- 标题 · 正文","- 标签 · 笔记簿","🔎 建议：搜索知识库"],
        _ => vec![]
    }.into_iter().map(String::from).collect() };

    let mut ic = |t, l: &mut Vec<String>| { if shown.insert(t.to_string()) { l.extend(card(t)); } };`
);

// Step 2: Replace the end of the function
c = c.replace(
    `    if lines.is_empty() { return String::new(); }
    lines.join(\"\\n\")
}`,
    `    if lines.is_empty() { return String::new(); }
    if !st.is_empty() && !sid.is_empty() { lines.push(format!("源数据：{}/{}", st, sid)); }
    if total_parents > 1 { lines.push(format!("父块：{}/{}", chunk_index + 1, total_parents)); }
    if !chunk_id.is_empty() { let short: String = chunk_id.chars().take(8).collect(); lines.push(format!("ChunkID：{}", short)); }
    lines.join("\\n")
}`
);

// Step 3: Replace the internal call site
c = c.replace(`build_chunk_metadata(st, sid)`, `build_chunk_navigation_card(st, sid, "", 0, 0)`);

fs.writeFileSync('src-tauri/src/services/agent_workflow.rs', c, 'utf8');
console.log('Step 1-3 done. Checking if insert_card calls are needed...');

// Count occurrences
let matches1 = (c.match(/insert_card/g) || []).length;
console.log('insert_card occurrences:', matches1);
let matches2 = (c.match(/build_chunk/g) || []).length;
console.log('build_chunk occurrences:', matches2);
