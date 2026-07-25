use crate::database;
use serde_json::json;
use tauri::Emitter;

/// App 启动时执行一次后台 LLM 分析，生成通知
pub fn run_startup_check(app_handle: tauri::AppHandle) {
    // 检查是否启用
    let enabled = get_config("enabled").unwrap_or("1".to_string());
    if enabled != "1" {
        log::info!("[通知循环] 后台分析已禁用，跳过");
        return;
    }

    log::info!("[通知循环] 启动时执行后台分析...");

    // 构造上下文
    let context = build_initial_context();

    // 查询最近5条通知
    let recent = get_recent_notifications(5);

    // 构造模拟消息
    let prompt = format!(
        "你是 ArkMind 的后台智能分析助手。请分析以下业务数据，发现需要用户关注的事项。\n\n\
         当前业务快照：\n{}\n\n\
         最近通知：\n{}\n\n\
         请分析以上数据，发现需要用户关注的事项。\n\
         以下情况请调用 ext_tool_manage_notification 创建通知：\n\
         - 待办即将到期（24小时内）\n\
         - 项目停滞超过7天\n\
         - 投标截止临近（3天内）\n\
         - 发现业务数据中的重要趋势或异常\n\
         通知 content 请使用 Markdown 格式（**加粗**、- 列表、> 引用等），让内容结构清晰。\n\
         如有跟进之前通知的事项，请设置 follow_up_of 为对应通知ID。\n\
         如果没有新发现，回复\"暂无新发现\"。",
        context, recent
    );

    // 使用 tauri::async_runtime::spawn 调用 agent_chat_workflow
    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(
            prompt,
            Some(4),  // 限制迭代次数
            vec![],
            Some(app_handle.clone()),
            true,  // silent=true，不发 UI event
            None,  // 不使用 session 缓存
        ).await;

        match result {
            Ok(v) => {
                let answer = v["answer"].as_str().unwrap_or("");
                let iters = v["iterations"].as_i64().unwrap_or(0);
                let preview: String = answer.chars().take(100).collect();
                log::info!("[通知循环] 分析完成，迭代{}次，回复: {}", iters, preview);
            }
            Err(e) => {
                log::error!("[通知循环] 分析失败: {}", e);
            }
        }

        // 通知循环结束，通知前端恢复 mascot 状态
        let _ = app_handle.emit("agent:done", serde_json::json!({}));

        // 更新统计
        update_call_stats();
        // 更新最后检查时间
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        set_config("last_check_at", &now);
    });
}

/// 构造初始上下文（全量快照）
fn build_initial_context() -> String {
    let mut parts = Vec::new();

    // 活跃项目
    if let Ok(rows) = database::all(
        "SELECT name, status, budget, bid_time, expected_order_date, updated_at FROM projects \
         WHERE status NOT IN ('Won','Ordered','Lost','Archived') ORDER BY updated_at DESC LIMIT 10",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let name = r["name"].as_str().unwrap_or("");
                let status = r["status"].as_str().unwrap_or("");
                let budget = r["budget"].as_f64().unwrap_or(0.0);
                format!("  - {} (状态:{}, 预算:{:.0}万)", name, status, budget)
            }).collect();
            parts.push(format!("活跃项目 ({}个):\n{}", rows.len(), items.join("\n")));
        }
    }

    // 待办
    if let Ok(rows) = database::all(
        "SELECT title, priority, due_date, done FROM todos WHERE done=0 ORDER BY due_date ASC LIMIT 10",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let title = r["title"].as_str().unwrap_or("");
                let priority = r["priority"].as_str().unwrap_or("");
                let due = r["due_date"].as_str().unwrap_or("无");
                format!("  - [{}] {} (截止:{})", priority, title, due)
            }).collect();
            parts.push(format!("未完成待办 ({}个):\n{}", rows.len(), items.join("\n")));
        }
    }

    // 近期中标
    if let Ok(rows) = database::all(
        "SELECT name, budget, updated_at FROM projects WHERE status='Won' ORDER BY updated_at DESC LIMIT 5",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let name = r["name"].as_str().unwrap_or("");
                let budget = r["budget"].as_f64().unwrap_or(0.0);
                let updated = r["updated_at"].as_str().unwrap_or("");
                format!("  - {} ({:.0}万, {})", name, budget, &updated[..10.min(updated.len())])
            }).collect();
            parts.push(format!("近期中标:\n{}", items.join("\n")));
        }
    }

    if parts.is_empty() {
        "暂无业务数据".to_string()
    } else {
        parts.join("\n\n")
    }
}

/// 查询最近N条通知
fn get_recent_notifications(limit: i64) -> String {
    let rows = database::all(
        "SELECT type, title, created_at, read FROM notifications ORDER BY created_at DESC LIMIT ?1",
        &[json!(limit)],
    ).unwrap_or_default();

    if rows.is_empty() {
        return "无".to_string();
    }

    rows.iter().map(|r| {
        let time = r["created_at"].as_str().unwrap_or("");
        let title = r["title"].as_str().unwrap_or("");
        let read = r["read"].as_i64().unwrap_or(0);
        let status = if read == 0 { "未读" } else { "已读" };
        format!("[{}] {} ({})", &time[..16.min(time.len())], title, status)
    }).collect::<Vec<_>>().join("\n")
}

/// 读取配置
fn get_config(key: &str) -> Option<String> {
    database::with_db(|c| c.query_row(
        "SELECT value FROM notification_config WHERE key=?1",
        [&key.to_string()],
        |r| r.get::<_, String>(0),
    )).ok()
}

/// 写入配置
fn set_config(key: &str, value: &str) {
    let _ = database::run(
        "INSERT OR REPLACE INTO notification_config(key, value) VALUES(?1, ?2)",
        &[json!(key), json!(value)],
    );
}

/// 更新调用统计
fn update_call_stats() {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let stored_date = get_config("today_date").unwrap_or_default();

    if stored_date != today {
        set_config("today_date", &today);
        set_config("today_calls", "1");
    } else {
        let calls: i64 = get_config("today_calls")
            .and_then(|v| v.parse().ok()).unwrap_or(0);
        set_config("today_calls", &(calls + 1).to_string());
    }
}
