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
