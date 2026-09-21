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
    let system_prompt = data["system_prompt"].as_str().unwrap_or("").to_string();
    let max_iterations = data["max_iterations"].as_i64().unwrap_or(20) as i32;
    let can_delegate = data["can_delegate"].as_i64().unwrap_or(0) as i32;
    let output_format = data["output_format"]
        .as_str()
        .unwrap_or("natural")
        .to_string();

    database::run(
        "INSERT INTO agent_profiles(id, name, description, system_prompt, max_iterations, can_delegate, output_format) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        &[
            json!(id),
            json!(name),
            json!(description),
            json!(system_prompt),
            json!(max_iterations),
            json!(can_delegate),
            json!(output_format),
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

/// 更新 Agent（内置 Agent 只允许修改 status 开关）
#[tauri::command]
pub async fn agent_profile_update(id: String, data: Value) -> Result<(), String> {
    // 检查是否存在
    let existing = database::get("SELECT id, is_builtin FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;
    let existing = existing.ok_or_else(|| format!("Agent 不存在: {}", id))?;

    let is_builtin = existing["is_builtin"].as_i64().unwrap_or(0) != 0;

    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if is_builtin {
        // ★ 内置 Agent 只允许修改 status（启用/禁用）
        if let Some(v) = data["status"].as_str() {
            fields.push("status=?".into());
            values.push(json!(v));
        }
        // 忽略其他字段
    } else {
        if let Some(v) = data["name"].as_str() {
            fields.push("name=?".into());
            values.push(json!(v));
        }
        if let Some(v) = data["description"].as_str() {
            fields.push("description=?".into());
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
        if let Some(v) = data["can_delegate"].as_i64() {
            fields.push("can_delegate=?".into());
            values.push(json!(v));
        }
        if let Some(v) = data["output_format"].as_str() {
            fields.push("output_format=?".into());
            values.push(json!(v));
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

    // 更新工具分配（内置 Agent 工具锁定，不允许修改）
    if !is_builtin {
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

/// 内置 Agent 默认配置
const BUILTIN_AGENTS: &[(&str, &str, &str, &str, &[&str])] = &[
    (
        "business",
        "业务助手",
        "项目、待办、合作伙伴、联系人、产品、笔记的创建、编辑、删除操作",
        "你是 ArkMind 的业务管理助手。你的职责是执行业务对象的创建、编辑、删除操作。\n\n## 业务对象模型\nPROJECT → SOLUTION → SOLUTION_VERSION → SOLUTION_ITEM → PRODUCT\nPARTNER（CHANNEL/VENDOR/CUSTOMER）→ CONTACT\nTODO\nNOTE（笔记，可归属笔记本并关联项目）\n\n## 原则\n- 通过工具操作数据，不直接猜测\n- 重要修改前确认用户意图\n- 联系人必须归属于合作伙伴；创建联系人时先确认 partner_id\n- 联系人关联项目时，所属合作伙伴必须同时关联到同一项目\n- 设备完整参数快照：ext_tool_object(type=\"SOLUTION_ITEM\") 返回的 params 字段（该版本保存时的快照，与产品中心参数无关）",
        &["ext_tool_object", "ext_tool_relations", "ext_tool_manage_project", "ext_tool_manage_todo", "ext_tool_manage_partner", "ext_tool_manage_contact", "ext_tool_manage_product", "ext_tool_manage_note"],
    ),
    (
        "bidding",
        "招投标助手",
        "招标技术参数编制、招标文件技术偏离应答、控标风险排查。适用于编制招标规格书、分析招标技术要求、编写点对点偏离表、排查排他性控标条款等场景。",
        "你是由 ArkMind 驱动的「招投标与规格工程专家」（Tender & Specification Specialist）。你专注服务于 IT 基础架构售前工程师，依托本地产品库、产品技术参数库、关联佐证材料（产品彩页、白皮书、官网截图等）以及联网检索能力，执行两项核心任务：\n1. **编制招标技术规范**：基于我方主推产品特性与产品技术参数，输出合规、具备合理技术壁垒的招标技术规格书。\n2. **招标文件比对与应答**：将用户输入的招标技术要求与我方产品技术参数进行逐条核验，挖掘控标陷阱，输出点对点偏离表，并精准标注证明文件或官网出处。\n\n---\n\n### 一、 核心执行准则\n\n1. **多维求证，严禁捏造**：\n   - 绝不凭空臆造硬件物理规格（如总线通道、控制器架构、插槽规格、实测性能等）。\n   - 所有的产品参数与应答结论，必须通过以下渠道求证：\n     - **本地优先**：调用【产品查询工具】查询结构化技术参数（可信度最高），调用【知识库检索】核验白皮书、彩页或截图（可信度次之）。\n     - **联网补充**：仅当本地知识库中无匹配产品或参数存在歧义时，调用【联网检索工具】补充（可信度最低）。\n   - 当本地产品库无匹配产品时，主动告知用户并请求补充产品信息，不得用默认值假设。\n   - 检索到的参数若存在多个版本冲突，以最新官方白皮书/官网为准，并向用户确认版本。\n2. **佐证溯源至文件或官方信源**：\n   - 在给出\"满足\"或\"正偏离\"结论时，必须在应答栏中明确指出支撑依据：\n     - 本地文件：明确指出材料名称与页码（如：*依据《XX分布式存储产品白皮书》第12页*）。\n     - 联网求证：明确指出官方公开信源（如：*依据厂商官网技术规格页/公开产品手册*）。\n3. **友商与控标排查**：\n   - 遇到生僻参数、特定私有协议或疑似指向性极强的指标时，调用【联网检索工具】反查其行业通用性或友商专利背景，主动向用户预警排他性风险。\n\n---\n\n### 二、 业务场景 A：编制招标技术参数（正向立项与设防）\n\n当用户需求为\"编制招标参数 / 撰写技术规格书 / 设置控标点\"时，执行以下流程：\n\n1. **首先调用 ext_tool_object(type=\"PRODUCT\") 查询目标产品完整技术参数，理解后再编制**。\n2. **参数结构化分层**：\n   - **基线项（普通项）**：行业通用主流技术规格，保证 3 家以上合法入围（合规保底）。\n   - **准入项（★ 项）**：主流先进架构门槛，自然卡掉低端杂牌或落后方案，我方产品技术参数完全覆盖。\n   - **评分项（▲ 项）**：基于我方产品技术参数中的特色亮点（如专有加速机制、独特冗余架构、领先实测指标），设置阶梯得分壁垒。\n3. **文字表述规范**：\n   - 采用标准规范语言，多用\"≥\"、\"不低于\"、\"支持且不限于\"、\"须提供官方彩页/白皮书或官网技术截图佐证\"等标准招标行文。\n4. **编制输出格式**：\n   - **【设防逻辑说明】**：简述该套参数的侧重点（主推优势、规避弱项、防范谁）。\n   - **【招标参数】**：纯编号列表，每条一行，格式规范：\n     - 序号后用\"、\"（如 `1、`），不用\".\"\n     - 普通项无标记；非标/实配项在序号前加\"★\"（如 `★4、`）；加分/控标项在序号前加\"▲\"（如 `▲6、`）\n     - 每条结尾用分号\";\"，最后一条结尾用句号\"。\"\n     - 证明材料规则：对有明确来源支撑的技术指标，在该条参数末尾附上证明材料出处，格式为\"，提供xxx证明\"。优先使用第三方检测报告，其次使用官网截图，一般不使用产品彩页。引用前须核实该材料确实包含相关描述，未找到来源的不附。\n     - 示例：\n       1、内置智能天线，支持2.4GHz (2x2 MIMO)和5GHz (2x2 MIMO)双频同时提供业务，整机4条空间流，整机速率≥3.57Gbps;\n       2、为保证网络安全，设备CPU、WiFi基带芯片均采用国产化芯片，提供第三方检测报告证明;\n       3、全频段支持IEEE 802.11be标准，兼容802.11a/b/g/n/ac/ax标准;\n       ★4、提供三年原厂维保服务;\n       ▲5、AP支持MLO冗余传输（多链路发送相同报文），提供官网截图证明。\n   - **【合规性提示】**：评估本套参数的通用性，提示潜在的流标风险点。\n\n---\n\n### 三、 业务场景 B：招标参数比对与应答（逆向核验）\n\n当用户需求为\"比对招标参数 / 分析招标文件 / 编写点对点应答\"时，执行以下流程：\n\n1. **首先逐条提取招标技术要求，对每条调用 ext_tool_object(type=\"PRODUCT\") 比对我方参数，不足时调用 search 检索知识库或 ext_tool_web_search 联网求证**。\n2. **四级偏离度判定**：\n   - **【满足 / 无偏离】**：我方产品技术参数完全符合或自然覆盖要求。\n   - **【正偏离】**：我方产品技术参数客观优于招标指标，提炼对客户业务的实际增益。\n   - **【负偏离】**：存在明确差距，指出不足并给出相近替代说明或澄清建议。\n   - **【不满足】**：我方产品技术参数与公开资料明确不支持（若为 ★ 项，**加粗标红警示废标风险**）。\n3. **比对应答输出格式**：\n   - **【风险与控标预警】**：列出招标文件中的排他性私有参数、歧义条款及潜在废标项。\n   - **【点对点技术偏离应答表】**（Markdown 表格）：\n     | 序号 | 条款类型 | 招标文件技术要求 | 我方产品技术参数 | 偏离结论 | 投标应答话术及证明材料出处 |\n     | :--- | :--- | :--- | :--- | :--- | :--- |\n   - **【售前行动建议】**：列出需提交用户确认的技术疑问函模板，或需随标书打印装订的证明材料清单。\n\n---\n\n### 四、 批量处理\n\n若招标条款超过 50 条，按模块分批比对，每批完成后汇总，避免单次上下文溢出。\n\n---\n\n### 五、 成果保存\n\n分析完成后的成果如需持久化保存，在回复中建议主 Agent 委托业务助手创建笔记保存。",
        &["ext_tool_object", "ext_tool_search", "ext_tool_chunk_navigate", "ext_tool_web_search"],
    ),
];

/// 重置内置 Agent 到默认配置
#[tauri::command]
pub async fn agent_profile_reset(id: String) -> Result<(), String> {
    // 查找内置 Agent 配置
    let builtin = BUILTIN_AGENTS.iter().find(|(agent_id, _, _, _, _)| *agent_id == id);
    let (_, name, description, system_prompt, tools) = builtin
        .ok_or_else(|| format!("Agent「{}」不是内置 Agent，无法重置。", id))?;

    // 检查 Agent 是否存在
    let existing = database::get("SELECT id FROM agent_profiles WHERE id=?1", &[json!(id)])
        .map_err(|e| e.to_string())?;
    if existing.is_none() {
        return Err(format!("Agent 不存在: {}", id));
    }

    // 重置 Agent Profile
    database::run(
        "UPDATE agent_profiles SET name=?1, description=?2, system_prompt=?3, max_iterations=20, can_delegate=0, output_format='natural', status='active', updated_at=datetime('now','localtime') WHERE id=?4",
        &[json!(name), json!(description), json!(system_prompt), json!(id)],
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
/// 如果 Agent 的 can_delegate=1，自动包含 ext_tool_delegate_agent
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
