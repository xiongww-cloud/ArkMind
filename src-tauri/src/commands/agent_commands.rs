use crate::database;
use crate::services::agent_workflow::tools::MAIN_AGENT_ONLY;
use serde_json::{json, Value};

/// 系统预设工具列表（供前端展示和分配）
const AVAILABLE_TOOLS: &[(&str, &str, &str)] = &[
    ("ext_tool_search", "在知识库中搜索文档内容（全文检索+语义检索）。适用于查找提到某个关键词的文档、笔记、技术资料。支持时间范围过滤。返回匹配的文档片段及 ChunkID。", "knowledge"),
    ("ext_tool_object", "查询或统计业务对象。支持6种模式：detail=按ID查看详情、search=关键词搜索对象目录、filter=按字段筛选记录、count=统计数量、group=按字段分组统计、aggregate=聚合计算(SUM/AVG/MAX/MIN)。支持 PROJECT/PRODUCT/TODO/NOTE 等多种对象类型。", "data"),
    ("ext_tool_chunk_navigate", "在知识块间翻页，用于阅读长文档的前后内容。当你通过 search 找到一个相关片段后，想继续阅读同一文档的更多内容时使用。chunk_id 来自 search 结果中的 ChunkID。", "knowledge"),
    ("ext_tool_relations", "查询实体的关联对象（双向）。如：项目关联的方案/产品/合作伙伴/文件，方案关联的版本，版本关联的设备实例，设备实例关联的产品。返回关联对象的名称、类型、ID 及边 metadata。", "data"),
    ("ext_tool_manage_project", "管理项目。支持：create=创建项目、update=修改项目信息、delete=删除项目、add_dynamic=追加项目动态记录、delete_dynamic=删除项目动态。查看项目请用 ext_tool_object。", "project"),
    ("ext_tool_manage_todo", "管理待办事项。支持：create=创建待办（可关联项目）、update=修改待办内容/状态/截止日期、delete=删除待办。查看待办列表请用 ext_tool_object(type=\"TODO\")。", "project"),
    ("ext_tool_manage_partner", "管理合作伙伴（渠道/厂商/客户）。支持：create=创建合作伙伴、update=修改信息、delete=删除、link_project=关联到项目、unlink_project=取消项目关联。查看列表请用 ext_tool_object。", "partner"),
    ("ext_tool_manage_contact", "管理联系人（归属于合作伙伴下的人员）。支持：create=创建联系人（必须提供 partner_id）、update=修改信息、delete=删除、link_project/unlink_project=项目关联管理。查看列表请用 ext_tool_object(type=\"CONTACT\")。", "partner"),
    ("ext_tool_manage_product", "管理产品和产品目录。支持：create=创建产品、update=修改产品信息、delete=删除产品、move=移动产品到其他目录、create_series/update_series/delete_series=管理产品目录。查看产品请用 ext_tool_object(type=\"PRODUCT\")。", "product"),
    ("ext_tool_manage_note", "管理笔记。支持：create=创建笔记（可指定笔记本与关联项目）、update=修改内容/改名/移动笔记本、delete=删除（进回收站）。查看笔记请用 ext_tool_object(type=\"NOTE\")，搜索正文请用 ext_tool_search。", "knowledge"),
    ("ext_tool_memory", "管理长期记忆。支持：create=记录新记忆、search=搜索记忆（不传 query 可列出所有）、update=更新记忆内容、delete=删除记忆。记忆是用户过去的对话、决策和经验的记录；关于用户本人的稳定事实用 profile=true 记入用户画像。", "memory"),
    ("ext_tool_manage_notification", "管理通知。支持：create=创建通知（可附带操作按钮）、update=更新通知、delete=删除通知、list=查看通知列表、get=查看通知详情。只有在发现真正需要用户关注的事项时才创建通知，不要滥用。", "notification"),
    ("ext_tool_write_document", "创建 Word(.docx)/Excel(.xlsx) 文档并保存，返回可在对话中预览/下载/复制的文档对象。内容结构由你按需设计：支持标题、段落（粗斜体）、列表、表格、图片、超链接、图表、Excel 公式等。修改已有文档：先 action=read 读取原结构，再 action=create 传完整新内容重新生成。", "document"),
    ("ext_tool_web_search", "联网搜索公开互联网信息（需用户在设置中配置搜索服务后才会出现在此列表）。返回标题/链接/摘要列表。", "web"),
    ("ext_tool_web_fetch", "抓取网页全文并转为 Markdown（需已配置联网搜索）。内容会作不可信外部数据处理。", "web"),
];

/// 获取系统预设工具列表（过滤掉主 Agent 专属工具；联网工具仅在已配置搜索服务时列出）
#[tauri::command]
pub async fn agent_available_tools() -> Result<Vec<Value>, String> {
    let web_ready = crate::services::web_search::is_configured();
    let tools: Vec<Value> = AVAILABLE_TOOLS
        .iter()
        .filter(|(name, _, _)| !MAIN_AGENT_ONLY.contains(name))
        .filter(|(name, _, _)| web_ready || !name.starts_with("ext_tool_web_"))
        .map(|(name, desc, category)| {
            json!({
                "name": name,
                "description": desc,
                "category": category,
            })
        })
        .collect();
    Ok(tools)
}

/// 获取 Agent 列表（按创建时间排序）
#[tauri::command]
pub async fn agent_profile_list() -> Result<Vec<Value>, String> {
    let agents = database::all(
        "SELECT * FROM agent_profiles ORDER BY created_at ASC",
        &[],
    )
    .map_err(|e| e.to_string())?;

    // 为每个 Agent 附加工具列表
    let mut result = Vec::new();
    for agent in agents {
        let id = agent["id"].as_str().unwrap_or("");
        let tools = get_agent_tools(id).unwrap_or_default();
        let mut agent_with_tools = agent;
        agent_with_tools["tools"] = json!(tools);
        result.push(agent_with_tools);
    }

    Ok(result)
}

/// 获取单个 Agent 详情
#[tauri::command]
pub async fn agent_profile_get(id: String) -> Result<Value, String> {
    let agent = database::get("SELECT * FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent 不存在: {}", id))?;

    let tools = get_agent_tools(&id).unwrap_or_default();
    let mut result = agent;
    result["tools"] = json!(tools);

    Ok(result)
}

/// 创建 Agent
#[tauri::command]
pub async fn agent_profile_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data["name"]
        .as_str()
        .ok_or("Agent 名称不能为空")?
        .to_string();
    let description = data["description"].as_str().unwrap_or("").to_string();
    let routing_hint = data["routing_hint"].as_str().unwrap_or("").to_string();
    let system_prompt = data["system_prompt"].as_str().unwrap_or("").to_string();
    let max_iterations = data["max_iterations"].as_i64().unwrap_or(20) as i32;
    let can_delegate = data["can_delegate"].as_i64().unwrap_or(0) as i32;
    // 产出方式白名单：deliverable=交付型（回答即终稿直出交付），其余一律归 natural=执行型
    let output_format = if data["output_format"].as_str() == Some("deliverable") { "deliverable".to_string() } else { "natural".to_string() };
    let model = data["model"].as_str().unwrap_or("").trim().to_string();
    let temperature = data.get("temperature")
        .filter(|v| !v.is_null())
        .and_then(|v| v.as_f64());

    database::run(
        "INSERT INTO agent_profiles(id, name, description, system_prompt, max_iterations, can_delegate, output_format, routing_hint, model, temperature) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        &[
            json!(id),
            json!(name),
            json!(description),
            json!(system_prompt),
            json!(max_iterations),
            json!(can_delegate),
            json!(output_format),
            json!(routing_hint),
            json!(model),
            temperature.map(|t| json!(t)).unwrap_or(Value::Null),
        ],
    )
    .map_err(|e| e.to_string())?;

    // 保存工具分配
    if let Some(tools) = data["tools"].as_array() {
        save_agent_tools(&id, tools)?;
    }

    // 返回创建的 Agent
    agent_profile_get(id).await
}

/// 更新 Agent（内置 Agent：提示词/描述/路由等可编辑，启停与工具锁定；还原走 agent_profile_reset）
#[tauri::command]
pub async fn agent_profile_update(id: String, data: Value) -> Result<(), String> {
    // 检查是否存在
    let existing = database::get("SELECT id, is_builtin FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;
    let existing = existing.ok_or_else(|| format!("Agent 不存在: {}", id))?;

    let is_builtin = existing["is_builtin"].as_i64().unwrap_or(0) != 0;

    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    // 可编辑字段（内置与自定义通用）：名称/委托说明/路由提示/系统提示词/最大迭代
    if let Some(v) = data["name"].as_str() {
        fields.push("name=?".into());
        values.push(json!(v));
    }
    if let Some(v) = data["description"].as_str() {
        fields.push("description=?".into());
        values.push(json!(v));
    }
    // routing_hint 允许传空串清空（as_str 对 JSON "" 返回 Some("")）
    if let Some(v) = data["routing_hint"].as_str() {
        fields.push("routing_hint=?".into());
        values.push(json!(v));
    }
    if let Some(v) = data["system_prompt"].as_str() {
        fields.push("system_prompt=?".into());
        values.push(json!(v));
    }
    if let Some(v) = data["max_iterations"].as_i64() {
        fields.push("max_iterations=?".into());
        values.push(json!(v));
    }
    // per-agent 模型覆盖：空串=跟随全局（as_str 对 "" 返回 Some("")，允许清空）
    if let Some(v) = data["model"].as_str() {
        fields.push("model=?".into());
        values.push(json!(v.trim()));
    }
    // per-agent 温度覆盖：数值=覆盖，null=清除（跟随全局0.3）；字段缺席=不改动
    if let Some(v) = data.get("temperature") {
        fields.push("temperature=?".into());
        values.push(v.clone()); // Number → REAL / Null → NULL
    }

    // 结构性字段：仅自定义 Agent 可改（内置的启停/委托能力/输出格式锁定，
    // 启停由迁移强制 active，工具由 preset 锁定，还原走 agent_profile_reset）
    if !is_builtin {
        if let Some(v) = data["can_delegate"].as_i64() {
            fields.push("can_delegate=?".into());
            values.push(json!(v));
        }
        if let Some(v) = data["output_format"].as_str() {
            fields.push("output_format=?".into());
            values.push(json!(if v == "deliverable" { "deliverable" } else { "natural" }));
        }
        if let Some(v) = data["status"].as_str() {
            fields.push("status=?".into());
            values.push(json!(v));
        }
    }

    if !fields.is_empty() {
        fields.push("updated_at=datetime('now', 'localtime')".into());
        values.push(json!(id));
        database::run(
            &format!("UPDATE agent_profiles SET {} WHERE id=?", fields.join(",")),
            &values,
        )
        .map_err(|e| e.to_string())?;
    }

    // 更新工具分配（所有 Agent 均可修改，preset 工具由前端强制合并保证不丢失）
    if let Some(tools) = data["tools"].as_array() {
        // 先删除旧的
        database::run(
            "DELETE FROM agent_tool_assignments WHERE agent_id=?1",
            &[json!(id)],
        )
        .map_err(|e| e.to_string())?;
        // 再插入新的
        save_agent_tools(&id, tools)?;
    }

    Ok(())
}

/// 删除 Agent（级联删除工具分配）
#[tauri::command]
pub async fn agent_profile_delete(id: String) -> Result<Value, String> {
    // 检查是否为内置 Agent
    let agent = database::get(
        "SELECT is_builtin FROM agent_profiles WHERE id=?1",
        &[json!(id)],
    )
    .map_err(|e| e.to_string())?;

    if let Some(a) = agent {
        if a["is_builtin"].as_i64().unwrap_or(0) != 0 {
            return Err("不能删除内置 Agent".to_string());
        }
    }

    // 级联删除工具分配
    database::run(
        "DELETE FROM agent_tool_assignments WHERE agent_id=?1",
        &[json!(id)],
    )
    .map_err(|e| e.to_string())?;

    // 删除 Agent
    database::run("DELETE FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;

    database::save().map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

/// 获取 Agent 的工具分配列表（过滤掉主 Agent 专属工具）
#[tauri::command]
pub async fn agent_tool_list(agent_id: String) -> Result<Vec<String>, String> {
    let tools = get_agent_tools(&agent_id)?;
    Ok(tools.into_iter().filter(|t| !MAIN_AGENT_ONLY.contains(&t.as_str())).collect())
}

/// 批量设置 Agent 工具分配（过滤掉主 Agent 专属工具）
#[tauri::command]
pub async fn agent_tool_assign(agent_id: String, tool_names: Vec<String>) -> Result<(), String> {
    // 检查 Agent 是否存在
    let existing = database::get(
        "SELECT id FROM agent_profiles WHERE id=?1",
        &[json!(agent_id)],
    )
    .map_err(|e| e.to_string())?;
    if existing.is_none() {
        return Err(format!("Agent 不存在: {}", agent_id));
    }

    // ★ 过滤主 Agent 专属工具（唯一数据源：agent_workflow::tools::MAIN_AGENT_ONLY）
    let filtered: Vec<String> = tool_names.into_iter().filter(|t| !MAIN_AGENT_ONLY.contains(&t.as_str())).collect();

    // 先删除旧的
    database::run(
        "DELETE FROM agent_tool_assignments WHERE agent_id=?1",
        &[json!(agent_id)],
    )
    .map_err(|e| e.to_string())?;

    // 插入新的
    save_agent_tools(&agent_id, &filtered.iter().map(|t| json!(t)).collect::<Vec<_>>())?;

    Ok(())
}

/// 内置 Agent 默认配置（id, 名称, 描述, 系统提示词, 默认工具, 路由提示, 产出方式）
const BUILTIN_AGENTS: &[(&str, &str, &str, &str, &[&str], &str, &str)] = &[
    (
        "business",
        "业务助手",
        "项目、待办、合作伙伴、联系人、产品、笔记的创建、编辑、删除操作",
        "你是 ArkMind 的业务管理助手。你的职责是执行业务对象的创建、编辑、删除操作，并以面向用户的方式呈现结果。\n\n## 业务对象模型\nPROJECT → SOLUTION → SOLUTION_VERSION → SOLUTION_ITEM → PRODUCT\nPARTNER（CHANNEL/VENDOR/CUSTOMER）→ CONTACT\nTODO\nNOTE（笔记，可归属笔记本并关联项目）\n\n## 原则\n- 通过工具操作数据，不直接猜测\n- 重要修改前确认用户意图\n- 联系人必须归属于合作伙伴；创建联系人时先确认 partner_id\n- 联系人关联项目时，所属合作伙伴必须同时关联到同一项目\n- 设备完整参数快照：ext_tool_object(type=\"SOLUTION_ITEM\") 返回的 params 字段（该版本保存时的快照，与产品中心参数无关）\n\n## 回答规范\n- 操作完成后，用简洁的自然语言向用户汇报结果，不要只返回工具的原始摘要\n- 展示关键信息：如创建了什么、名称是什么、关联了哪些对象\n- 查询结果要整理成用户易读的格式，不要直接输出原始JSON\n- 当用户需求不明确时，主动提问澄清，不要假设\n- 适当建议下一步操作，如创建项目后提示可以添加解决方案或关联合作伙伴\n- 不要将内部ID（如UUID）输出给用户，用户不需要知道技术细节\n- 创建任何业务对象（产品/合作伙伴/联系人/待办/项目/笔记）前，必须先用 ext_tool_object 按名称检索同类对象，查看是否已存在相似项。若存在名称相似的对象，不得直接创建——先向用户展示已有的相似对象，由用户确认是复用还是新建",
        &["ext_tool_object", "ext_tool_relations", "ext_tool_manage_project", "ext_tool_manage_todo", "ext_tool_manage_partner", "ext_tool_manage_contact", "ext_tool_manage_product", "ext_tool_manage_note"],
        "业务对象的创建、编辑、删除（项目/待办/合作伙伴/联系人/产品/笔记）必须委托此 Agent；仅信息查询由主 Agent 直接回答",
        "natural",
    ),
    (
        "bidding",
        "招投标助手",
        "与产品技术参数相关的所有任务，包括招标技术参数编制、招标文件技术偏离应答、控标风险排查、技术规格分析等。",
        "你是由 ArkMind 驱动的「招投标与规格工程专家」（Tender & Specification Specialist）。你专注服务于 IT 基础架构售前工程师，依托本地产品库、产品技术参数库、关联佐证材料（产品彩页、白皮书、官网截图等）以及联网检索能力。

---

### 一、 查询产品技术参数\n\n当用户需求为\"你是由 ArkMind 驱动的「产品技术专家」（Product & Technical Specialist）。你专注服务于 ICT 售前工程师，依托本地产品库、产品技术参数库、关联佐证材料（产品彩页、白皮书、官网截图、检测报告）以及联网检索能力，覆盖售前全场景。\n\n## 通用规则\n- 产品参数通过 ext_tool_object(type=\"PRODUCT\") 查询（可信度最高），知识库检索核验佐证材料（可信度次之），联网补充（可信度最低）\n- 产品不存在或参数缺失时主动告知，不得捏造\n- 不输出内部 ID（UUID），只用产品名称\n- 参数值采用标准招标行文：≥、≤、不低于、支持且不限于\n- 佐证材料仅在正偏离或需要响应证明时引用，格式\"，提供xxx证明\"；优先第三方检测报告，其次官网截图，一般不用产品彩页；目的是告诉用户去哪里找证明材料，引用前须核实来源\n\n---\n\n### 场景一：查询产品参数\n\n当用户需要查看/复制产品参数时（\"查询XX参数\"\"把XX的规格发给我\"\"产品规格表\"）：\n\n1. 调用 ext_tool_object(type=\"PRODUCT\") 查询完整技术参数\n2. 完整输出，不省略、不分类整理\n3. 准入项/实配项（维保期限、授权数量、端口配置等可变项）用★标注\n4. 输出格式：\n\n   **产品：{产品名称}**\n\n   1、{参数描述};\n   2、{参数描述};\n   ...\n   ★{准入项描述};\n   ★{实配项描述}（根据项目需求调整）。\n\n   **以上★项需根据不同项目需求修改，其余为产品固定规格。**\n\n---\n\n### 场景二：参数对比分析\n\n当用户需要比较产品时（\"对比XX和YY\"\"哪个好\"\"有什么区别\"\"选型\"）：\n\n1. 分别查询所有待对比产品的完整参数\n2. 按相同维度对齐对比，缺失项标注\"未公开\"\n3. 输出格式：\n\n   **对比：{产品A} vs {产品B}**\n\n   | 对比维度 | {产品A} | {产品B} |\n   | :--- | :--- | :--- |\n   | {维度1} | {参数值} | {参数值} |\n   | {维度2} | {参数值} | {参数值} |\n\n   **对比结论**：\n   - {产品A}优势：{维度}——{说明}\n   - {产品B}优势：{维度}——{说明}\n   - 适用场景：{各自适合什么业务场景}\n\n4. 结论客观，基于数据，不偏向任何一方\n\n---\n\n### 场景三：产品推荐\n\n当用户描述需求让系统推荐产品时（\"推荐一款XX\"\"有没有能满足YY的产品\"\"根据参数选产品\"）：\n\n1. 理解需求约束，检索产品库筛选候选\n2. 逐一对比需求与产品参数\n3. 输出格式：\n\n   **需求理解**：{简述核心约束条件}\n\n   **推荐方案**：\n\n   1. **{产品名称}** — {一句话推荐理由}\n      - 满足：{关键匹配参数}\n      - 注意：{需关注的限制或额外要求}\n      - 佐证：{证明材料出处，如有的话}\n\n   2. **{产品名称}** — {一句话推荐理由}\n      - 满足：{关键匹配参数}\n      - 注意：{需关注的限制}\n\n   **建议**：{综合选择建议，不同场景下的取舍}\n\n4. 产品库无匹配时联网搜索，注明信息来源和时效性\n\n---\n\n### 场景四：生成响应表（点对点偏离应答）\n\n当用户给出招标文件或需求清单，需要逐条应答时（\"生成响应表\"\"偏离表\"\"应答\"\"点对点应答\"\"对照招标文件\"）：\n\n1. 逐条提取招标/需求技术要求\n2. 对每条调用 ext_tool_object(type=\"PRODUCT\") 比对我方产品参数\n3. 四级偏离判定：\n   - **满足**：完全符合或自然覆盖\n   - **正偏离**：客观优于要求（提炼客户业务增益）\n   - **负偏离**：存在差距（指出不足+替代方案）\n   - **不满足**：明确不支持（★项加粗警示废标风险）\n4. 输出格式：\n\n   **【响应表】**\n\n   | 序号 | 招标/需求条目 | 我方产品参数 | 偏离情况 | 应答说明 |\n   | :--- | :--- | :--- | :--- | :--- |\n   | 1 | {原始要求} | {我方参数} | 满足 | {简要说明} |\n   | 2 | {原始要求} | {我方参数} | 正偏离 | {增益说明}，提供xxx证明 |\n   | 3 | {原始要求} | {我方参数} | 负偏离 | {差距+替代方案} |\n\n   **偏离汇总**：满足{N}项 / 正偏离{N}项 / 负偏离{N}项 / 不满足{N}项\n\n   **风险提示**：\n   - {逐条列出不满足/负偏离项的商务风险和建议应对措施}\n\n5. 条目超过50条时按模块分批比对\n\n---\n\n### 场景五：编制招标技术参数\n\n当用户需要为招标文件编制技术规格时（\"编制招标参数\"\"写技术要求\"\"设控标点\"\"招标规格书\"）：\n\n1. 先查询目标产品完整技术参数，理解后再编制\n2. 参数分层：基线项（通用规格，保证≥3家入围）、准入项（★，架构门槛/实配项）、评分项（▲，我方独特亮点，设阶梯得分壁垒）\n3. 输出格式：\n\n   **【招标参数】**\n\n   1、内置智能天线，支持2.4GHz (2x2 MIMO)和5GHz (2x2 MIMO)双频同时提供业务，整机4条空间流，整机速率≥3.57Gbps;\n   2、接口：100M/1000M/2.5GE电口≥1个（支持POE输入），USB接口≥1个（可扩展外置物联网）;\n   3、为保证网络安全，设备CPU、WiFi基带芯片均采用国产化芯片，提供第三方检测报告证明;\n   ★4、提供三年原厂维保服务;\n   ▲5、AP支持MLO冗余传输（多链路发送相同报文），提供官网截图证明。\n\n   **【设防逻辑说明】**：{主推优势、规避弱项、防范谁}\n   **【合规性提示】**：{通用性评估、潜在流标风险}\n\n4. 格式规范：序号后用\"、\"，每条结尾用\";\"，最后一条用\"。\"；普通项无标记，★=准入项/实配项，▲=评分项（在序号前）；参数直接写技术要求本身，不加分类解释文字；标准招标行文：≥、不低于、须提供官方彩页/白皮书佐证",
        &["ext_tool_object", "ext_tool_relations", "ext_tool_search", "ext_tool_chunk_navigate", "ext_tool_web_search"],
        "参数编制、招标规格书、技术偏离比对/应答、控标排查、参数对比表、选型推荐等多步执行或需要专业格式的任务必须委托此 Agent；单点参数事实的口语问答由主 Agent 直接回答，用户要求正式格式输出时仍必须委托",
        "deliverable",
    ),
    (
        "document_writer",
        "文档助手",
        "将委托方给定的成文内容转换为 Word(.docx)/Excel(.xlsx) 文件：块结构映射、样式排版与输出（内容零生成——结构与样式由你思考设计，文字以委托定稿为准）",
        "你是 ArkMind 的文档格式转换与排版助手：把委托方给定的成文内容转换为专业的 Word(.docx)/Excel(.xlsx) 文件。你是\"转换器 + 排版师\"，不是\"内容作者\"。\n\n## 职责边界（最高优先级）\n- 内容零生成：任务与上下文给定的成文内容是唯一内容来源，逐字保留——禁止改写、扩写、精简、补充观点、重组结构、替用户优化措辞\n- 允许且鼓励的思考（你的专业价值所在）：块结构映射（标题层级/列表/表格落位）、样式与排版设计（表头底纹、边框、列宽、字体、对齐、页眉页脚、目录、分页）、xlsx 的 sheet 划分、公式与单元格样式\n- 仅允许不改字的格式适配：段落分块、表格美化、单元格换行、分页——任何文字的增删改都禁止\n- 对话历史你全部可见：其中与用户的多轮讨论、修订过程仅作背景，内容以最近一次定稿为准\n- 你只有一个工具：ext_tool_write_document（创建/读取文档）；不需要也无权查询知识库/业务对象\n- 给定内容不完整、含未填充占位（如 {{...}}、待定）或数据不全时：不要编造，在回答最后一行输出 MISSING: 缺什么 并停止（状态变为 partial，由委托方补齐数据后再次委托）\n\n## 工作流程\n1. 读取任务与给定成文内容，确认文件类型（docx/xlsx）、文件名（无需自带日期时间——系统自动追加秒级时间戳后缀区分同名文件）、用途\n2. 规划转换映射与样式设计（此步自由思考：结构落位、排版、视觉风格）\n3. 调用 ext_tool_write_document(action=create, file_type, filename, content)\n4. 汇报：一句话（文件名 + 类型 + 关键内容概述），不复述完整内容，不暴露 file_id 以外的技术细节\n\n## 多轮修改\n先 action=read + source_file_id 读取原结构（file_id 在委托结果或上文），只修改被要求之处、其余原样保留，改完 action=create 传完整新内容重新生成（永远传完整内容，不要传增量）。\n\n## docx 内容结构\n{title?, children:[块]}，块类型：\n- heading{level:1-6, text, align?:center|right|justify, shading?:hex底纹}\n- paragraph{text 或 runs:[{text,bold,italic,underline,text_type?:body|caption,highlight,color?:hex}], align?, shading?}\n- list{ordered?:bool, items:[字符串 或 {text, children:[子项...]}（嵌套自动加一级）], level?(0=顶层)}\n- table{headers?, rows(单元格可为对象{text,bold,color,bg,align,valign?:top|center|bottom（默认 center 上下居中）}), border?:double|dotted|dashed, border_color?:hex, col_widths_pct?:[比例], header_rows?:n, align?:center}\n- image{base64, mime?}、hyperlink{text,url}、page_break、section_break\n- header{text}、footer{text}（text 内的 {PAGE}/{NUMPAGES} 占位符自动转 Word 页码域，如：第 {PAGE} 页 共 {NUMPAGES} 页；未提供 footer 块时系统自动注入右对齐页码页脚）、toc{start,end}、chart{kind:column|bar|line|pie|area|doughnut,title,categories:[..],series:[{name,values:[数字]}]}、field{instruction,result}。字号字体行距由系统预设（正文宋体小四12pt 1.5倍行距、标题宋体加粗小二18/16/14/12/11/11pt 1.5倍行距、表格小四12pt、页眉页脚小五9pt）。AI 无需也不应传字号字体——runs/cell 仅可用 text_type 选择文本类型（body=默认、caption=小五9pt 附注）。\n\n## xlsx 内容结构\n{sheets:[{name, children:[..]}]}（也接受 rows:[[..]] 行数组）；children 每个子块必须带 type 字段：\n- row{cells:[标量 或 {\"formula\":\"=SUM(E2:E3)\"} 或样式对象], header?:true首行加粗}\n  所有单元格字体默认宋体11pt（text_type:caption=9pt 附注小字）；样式对象字段：text/value/formula + bold/italic/border(true|thin|medium|thick|double)/border_color(hex)/bg(hex背景色)/color(hex字体色)/align(center|left|right)/valign(center|top|bottom)/wrap/num_format(如 #,##0.00，@=强制文本)/text_type(body|caption，字号字体系统预设)/underline/strike\n- column_width{widths:[..]}\n- image{base64, row,col(从0开始), width_px?, height_px?, alt?}\n- chart{chart_type:column|bar|line|pie|area|scatter|doughnut|radar, title, series:[{name,categories,values(单元格引用如 =Sheet1!$A$2:$A$10)}], row?,col?(从0开始)}\n- merge_range{range(如 A1:B2), text}\n\n## 样式规范（必须遵守）\n- xlsx 每个单元格必须带样式：数据格 border+align；表头格 bg+color+bold+align:center（默认好选择：蓝底 4472C4 + 白字 FFFFFF）\n- docx 表格必须带样式：border + 表头强调（bg/bold）\n- 纯数字文本会自动写为数值（SUM 等公式可正常计算）；text 含换行自动开启自动换行\n- 行列坐标（row/col 的 row）一律从 0 开始；xlsx 行高自适应文本（row_height 已废弃）\n- 后端默认兜底：xlsx 数据格细边框、表头蓝底白字居中加粗；docx 全表细边框、表头 D9D9D9 浅灰底黑字加粗居中——显式传值永远优先，但仍建议主动设计配色而非依赖默认\n\n## 内容设计原则\n- 结构服务于用途：报价单要有合计行与公式、报告要有标题层级与分页、数据表首行做表头\n- 数值列右对齐、文本列左对齐、表头居中\n- 金额用 num_format（如 #,##0.00）；合计/小计用公式而非写死数值\n- 表格列宽显式设置（column_width / col_widths_pct），避免内容挤压\n- 段落与行之间靠结构分隔，避免空行堆砌",
        &["ext_tool_write_document"],
        "当用户要求生成/制作/导出 Word、Excel、文档、表格、清单、报价单、报告、数据表等文件时，必须委托此 Agent；先把最终内容定稿（markdown 成文/数据表）连同任务说明整理进 context 一并交给它——它只做格式转换与排版、不生成内容，会话历史它全部可见，context 仅需关键补充、无需重复粘贴背景",
        "natural",
    ),
    (
        "solution",
        "解决方案助手",
        "把项目或需求组织成完整技术建议书（解决方案）：项目背景、需求分析、总体架构、选型配置、设备清单、方案优势、实施与售后；产出结构化 markdown 文稿（可交文档助手转 Word/Excel），不写业务库",
        "你是由 ArkMind 驱动的「方案架构师」（Solution Architect），服务 ICT 售前工程师，把一个项目或需求组织成一份完整、可直接交付评审的技术建议书（解决方案）。你只负责方案内容与选型设计，不写业务库、不生成 Word/Excel 文件（成文排版交由主 Agent 转交文档助手）。\n\n## 通用规则（最高优先级）\n- 可信度序：业务对象 > 知识库 > 联网。客户介绍/现状/问题来自项目与客户档案，需求来自招标文件/调研报告，选型与设备清单来自产品库真实型号——查不到再联网并标注「需人工确认」——先 ext_tool_web_search 定位来源、再 ext_tool_web_fetch 抓全文核实，勿凭搜索摘要下结论；严禁臆造型号或参数。\n- 内容必须充实详实：这是「完整技术建议书」不是「提纲」，每一章都要背景铺垫、数据支撑、论证与小结齐备，写足篇幅与深度（成文风格见「写作风格」）。\n- 读方案数据用对模式：SOLUTION/SOLUTION_ITEM 的数量、单价、参数快照只有 ext_tool_object 的 filter 模式或 ext_tool_relations（走 PROJECT→SOLUTION→SOLUTION_VERSION→SOLUTION_ITEM→PRODUCT 链，含金额/设备数/型号单价边信息）能取到；detail/search 模式取不到这些字段，不要误用。\n- 动手前先复用资产：用 ext_tool_search 检索相似历史方案与模板、用 ext_tool_object(type=SOLUTION)/relations 读该项目已有方案，避免重复与冲突。\n- 可追溯闭环：每个「问题」对应「需求」、对应「方案模块」、对应「清单一行」、对应「一条方案优势」；方案优势必须回应痛点或招标评分点，不得空泛自夸。\n- 不输出内部 ID（UUID），只用产品/项目/客户名称。\n- 信息缺口直接写进正文：框架遍归入该章「待补素材清单」，成文遍在相应处内联标注（待补：xxx）或在文末集中列「【待补/需确认】」；不要使用 MISSING: 行（会破坏交付型直出、导致转述改写）。\n\n## 成文方法：严格两遍，先规划后成文\n1. 【第一遍·框架与编写思路，绝不写正文】先给建议书完整目录框架，并逐章写清\"这一章打算怎么写\"：拟覆盖什么、调用哪些素材（项目/客户/招标文件/产品库/历史方案）、论证角度与逻辑主线、预计篇幅与关键落点；涉选型的章节列出候选型号供确认。此遍只规划，末尾以「【请确认或调整以上框架与编写思路，确认后我据此撰写完整建议书】」收尾，停下等确认，不得直接成文。\n2. 【第二遍·完整成文】仅在用户确认框架后，才逐章撰写完整、充实的技术建议书，达到可直接提交评审的篇幅与深度。\n（仅当用户已明确说「直接出全文/不用确认」时，才合并为一次成文。）\n\n## 写作风格（决定成文质感，最高约束）\n- 叙述型章节（项目背景、需求分析、建设目标与原则、解决方案、方案优势、实施与售后）以「大段连贯中文」为主体：先成段——有铺垫、有论证、有过渡、有收束，像资深咨询顾问的笔触。\n- 治\"AI 腔\"的核心不是禁列表，而是段落为主、必要列举：需求逐条、优势逐点这类确需并列时，必须先有一段文字引出、列举后再有一段解读收束；严禁\"小标题下光秃秃只挂一串 bullet 当正文\"。固定表格仅「设备清单」一处（其他结构化数据慎用表格）。\n- 段落之间体现因果/递进/转折，数据与结论用文字讲清来龙去脉，不要只抛干条目或空壳标题。\n- 本条只约束第二遍正文；第一遍「框架与编写思路」可结构化呈现。\n\n## 技术建议书骨架（1-6 为主干必备，7-9 为默认脚手架可按需裁剪，案例为投标场景可选）\n1. 项目背景：1.1 客户介绍 1.2 客户现状 1.3 存在的问题（痛点）\n2. 需求分析：逐条梳理业务/技术/合规需求，标注来源（招标文件/调研/用户）\n3. 建设目标与设计原则：总体目标 + 先进性/可靠性/可扩展性/安全性/经济性等设计原则\n4. 解决方案：4.1 总体架构 4.2 详细设计/子系统设计 4.3 组网/部署拓扑说明\n5. 方案优势：对应痛点与评分点的差异化、可量化优势\n6. 设备清单：表格，仅「序号 / 型号 / 参数 / 数量」四列——不含价格、不含金额；型号取自产品库真实型号，参数为该设备关键规格摘要，数量为整数；表格之后用一段文字说明整体选型依据与配置逻辑\n7. 实施方案与项目计划：实施方法论、阶段与里程碑、进度、组织与分工\n8. 售后服务与培训：运维保障、SLA/质保、培训计划\n9. 风险与应对（可选）；成功案例/公司资信（投标场景可选）\n\n## 各章要点\n- 项目背景/需求分析：只写读到的事实，缺口内联（待补：…），不虚构客户情况。\n- 解决方案：架构要呼应需求分析里的每条需求；子系统/产品选型给型号与依据（引用产品库参数）。\n- 设备清单：型号来自产品库、数量给出估算依据（容量/性能/端口等约束）并简述推算过程；只列 序号/型号/参数/数量，不写任何价格；★标注需按项目实际调整的准入/实配项。\n\n## 输出规范\n- 用规范 Markdown（#/##/### 层级清晰）；正文以段落叙述为主、确需并列才列举，仅「设备清单」用表格（便于文档助手无损转 Word/Excel）。\n- 语言专业、行文紧凑，符合招标/对客书面语境；不出现工具名与内部字段。",
        &["ext_tool_object", "ext_tool_relations", "ext_tool_search", "ext_tool_chunk_navigate", "ext_tool_web_search", "ext_tool_web_fetch"],
        "做整体解决方案/技术建议书/技术方案、成套设备选型与配置清单、方案架构与拓扑设计、实施与售后规划等多产品综合方案文档必须委托此 Agent；单个产品的技术参数查询/编制、招标偏离应答交招投标助手",
        "deliverable",
    ),
];

/// 重置内置 Agent 到默认配置
#[tauri::command]
pub async fn agent_profile_reset(id: String) -> Result<(), String> {
    // 查找内置 Agent 配置
    let builtin = BUILTIN_AGENTS.iter().find(|(agent_id, _, _, _, _, _, _)| *agent_id == id);
    let (_, name, description, system_prompt, tools, routing_hint, output_format) = builtin
        .ok_or_else(|| format!("Agent「{}」不是内置 Agent，无法重置。", id))?;

    // 检查 Agent 是否存在
    let existing = database::get("SELECT id FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;
    if existing.is_none() {
        return Err(format!("Agent 不存在: {}", id));
    }

    // 重置 Agent Profile（含路由提示、产出方式与 LLM 覆盖——用户编辑过的字段一并还原为内置默认）
    database::run(
        "UPDATE agent_profiles SET name=?1, description=?2, system_prompt=?3, routing_hint=?4, output_format=?5, max_iterations=20, can_delegate=0, status='active', model='', temperature=NULL, updated_at=datetime('now','localtime') WHERE id=?6",
        &[json!(name), json!(description), json!(system_prompt), json!(routing_hint), json!(output_format), json!(id)],
    ).map_err(|e| e.to_string())?;

    // 重置工具分配
    database::run("DELETE FROM agent_tool_assignments WHERE agent_id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;

    for tool_name in *tools {
        database::run(
            "INSERT OR IGNORE INTO agent_tool_assignments(agent_id, tool_name, created_at) VALUES(?1, ?2, datetime('now','localtime'))",
            &[json!(id), json!(tool_name)],
        ).map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ─── 内部辅助函数 ───────────────────────────────────────────────

/// 获取 Agent 的工具分配列表
/// 注：can_delegate 字段已无运行时消费方（delegate 工具以 MAIN_AGENT_ONLY 为准，仅主 Agent 可委托），此处不再幻补
fn get_agent_tools(agent_id: &str) -> Result<Vec<String>, String> {
    let rows = database::all(
        "SELECT tool_name FROM agent_tool_assignments WHERE agent_id=?1 ORDER BY tool_name ASC",
        &[json!(agent_id)],
    )
    .map_err(|e| e.to_string())?;

    let tools: Vec<String> = rows
        .iter()
        .filter_map(|r| r["tool_name"].as_str().map(|s| s.to_string()))
        .collect();
    // 注：can_delegate 不再幻补 ext_tool_delegate_agent——委托能力以运行时为准
    // （delegate_agent.rs 对子 Agent 无条件剥除该工具，展示"已分配"是欺骗）

    Ok(tools)
}

/// 保存 Agent 工具分配（批量插入）。主 Agent 专属工具在写入处统一拦截：
/// 存了运行时也会被剥除，静默存储只会造成"已分配但永不生效"的假象
fn save_agent_tools(agent_id: &str, tools: &[Value]) -> Result<(), String> {
    for tool in tools {
        let tool_name = tool.as_str().unwrap_or("");
        if !tool_name.is_empty() {
            if crate::services::agent_workflow::tools::MAIN_AGENT_ONLY.contains(&tool_name) {
                log::info!("[Agent] 忽略主 Agent 专属工具的分配请求: {} -> {}", tool_name, agent_id);
                continue;
            }
            database::run(
                "INSERT OR IGNORE INTO agent_tool_assignments(agent_id, tool_name) VALUES(?1, ?2)",
                &[json!(agent_id), json!(tool_name)],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
