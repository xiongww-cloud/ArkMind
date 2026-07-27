use crate::database;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

/// 循环运行标志
static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);
/// 本次启动是否已执行过（startup_only 模式用）
static STARTUP_DONE: AtomicBool = AtomicBool::new(false);

/// 循环状态信息
pub struct LoopStatus {
    pub running: bool,
    pub last_check_at: String,
    pub next_check_at: String,
}

/// 启动周期通知分析循环
pub fn start_notification_loop(app_handle: tauri::AppHandle) {
    if LOOP_RUNNING.swap(true, Ordering::AcqRel) {
        log::info!("[通知循环] 已在运行，跳过重复启动");
        return;
    }

    log::info!("[通知循环] 启动周期分析循环");

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to create notification loop runtime");

        rt.block_on(async {
            // 启动后等待 15 秒再开始，让应用完成初始化
            tokio::time::sleep(Duration::from_secs(15)).await;

            loop {
                if !LOOP_RUNNING.load(Ordering::Relaxed) {
                    log::info!("[通知循环] 收到停止信号，退出循环");
                    break;
                }

                match tick(&app_handle) {
                    Ok(wait_secs) => {
                        tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    }
                    Err(e) => {
                        log::error!("[通知循环] tick 异常: {}，30秒后重试", e);
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
        });
    });
}

/// 获取循环状态
pub fn get_loop_status() -> LoopStatus {
    let running = LOOP_RUNNING.load(Ordering::Relaxed);
    let last_check_at = get_config("last_check_at").unwrap_or_default();
    let next_check_at = if running && !last_check_at.is_empty() {
        let interval: i64 = get_config("interval_minutes")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        calc_next_check(&last_check_at, interval)
    } else {
        String::new()
    };
    LoopStatus { running, last_check_at, next_check_at }
}

/// 单次 tick：检查配置，决定是否执行分析，返回下次 sleep 秒数
fn tick(app_handle: &tauri::AppHandle) -> Result<u64, String> {
    // 1. 检查是否启用
    let enabled = get_config("enabled").unwrap_or_else(|| "1".to_string());
    if enabled != "1" {
        log::debug!("[通知循环] 后台分析已禁用，60秒后重试");
        return Ok(60);
    }

    // 2. 检查 startup_only
    let startup_only = get_config("startup_only").unwrap_or_else(|| "0".to_string());
    if startup_only == "1" {
        if STARTUP_DONE.load(Ordering::Relaxed) {
            log::debug!("[通知循环] startup_only 模式，本次已执行，60秒后重试");
            return Ok(60);
        }
        // 首次执行
        log::info!("[通知循环] startup_only 模式，执行首次分析...");
        run_analysis(app_handle);
        STARTUP_DONE.store(true, Ordering::Relaxed);
        return Ok(60);
    }

    // 3. 周期模式：检查间隔
    let last_check_at = get_config("last_check_at").unwrap_or_default();
    let interval: i64 = get_config("interval_minutes")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);

    if !last_check_at.is_empty() {
        let now = chrono::Local::now();
        if let Ok(last_time) = chrono::NaiveDateTime::parse_from_str(&last_check_at, "%Y-%m-%d %H:%M") {
            let diff = now.naive_local().signed_duration_since(last_time);
            let diff_minutes = diff.num_minutes();
            if diff_minutes < interval {
                let wait = (interval - diff_minutes) as u64 * 60;
                log::debug!("[通知循环] 距上次分析{}分钟，需等待{}秒", diff_minutes, wait);
                return Ok(wait);
            }
        }
    }

    // 4. 执行分析
    log::info!("[通知循环] 开始周期分析...");
    run_analysis(app_handle);

    // 5. 等待一个完整间隔后再检查
    Ok(interval as u64 * 60)
}

/// 执行 LLM 分析
fn run_analysis(app_handle: &tauri::AppHandle) {
    let since = get_config("last_check_at").unwrap_or_else(|| {
        chrono::Local::now().checked_sub_signed(chrono::Duration::days(7))
            .unwrap_or_else(chrono::Local::now)
            .format("%Y-%m-%d %H:%M").to_string()
    });
    let context = build_initial_context(&since);

    // 无新数据时跳过 LLM 调用，直接更新检查时间
    if context == "距上次分析以来无业务数据变化。" {
        log::info!("[通知循环] 无新业务数据，跳过 LLM 调用");
        update_call_stats();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        set_config("last_check_at", &now);
        return;
    }

    let recent = get_recent_notifications(3);

    let prompt = format!(
        "## 后台分析任务\n\
         你正在执行后台智能分析任务。分析业务数据后，如发现需要用户关注的事项，\n\
         必须调用 ext_tool_manage_notification 创建通知，而不是直接在回复中输出。\n\
         \n\
         典型触发条件（不限于此，自行判断）：\n\
         - 待办即将到期或已逾期\n\
         - 项目长期停滞、状态异常\n\
         - 投标截止临近\n\
         - 关键数据变化趋势、异常波动\n\
         - 多个项目/待办之间的冲突或风险\n\
         - 用户可能忽略的重要信息\n\
         \n\
         如果数据中没有值得用户关注的事项，回复\"暂无新发现\"。\n\
         \n\
         以下数据是距上次分析之后发生变化的业务数据，每行开头的ID是对象ID。\n\
         如需查看某个对象的完整详情，可调用 ext_tool_object(id=[\"对象ID\"])。\n\
         例如：ext_tool_object(id=[\"abc123def456\"]) 查看项目详情。\n\
         如需其他业务数据（合作伙伴、联系人、文件、知识库等）辅助分析，可调用工具检索。\n\
         {}\n\
         \n\
         最近通知：\n{}",
        context, recent
    );

    let handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(
            prompt,
            Some(6),
            Some(handle.clone()),
            true,
            None,
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

        let _ = handle.emit("agent:done", serde_json::json!({}));

        update_call_stats();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        set_config("last_check_at", &now);
    });
}

/// 根据上次检查时间和间隔计算下次检查时间
fn calc_next_check(last_check_at: &str, interval_minutes: i64) -> String {
    if let Ok(last_time) = chrono::NaiveDateTime::parse_from_str(last_check_at, "%Y-%m-%d %H:%M") {
        let next = last_time + chrono::Duration::minutes(interval_minutes);
        next.format("%Y-%m-%d %H:%M").to_string()
    } else {
        String::new()
    }
}

/// 构造初始上下文（增量数据：仅返回上次分析后有变化的对象）
fn build_initial_context(since: &str) -> String {
    let mut parts = Vec::new();

    if let Ok(rows) = database::all(
        "SELECT id, name, status, budget, bid_time, expected_order_date, updated_at FROM projects \
         WHERE updated_at > ?1 AND status NOT IN ('Won','Ordered','Lost','Archived') \
         ORDER BY updated_at DESC",
        &[json!(since)],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let id = r["id"].as_str().unwrap_or("");
                let name = r["name"].as_str().unwrap_or("");
                let status = r["status"].as_str().unwrap_or("");
                let budget = r["budget"].as_f64().unwrap_or(0.0);
                format!("  - {} {} (状态:{}, 预算:{:.0}万)", id, name, status, budget)
            }).collect();
            parts.push(format!("活跃项目 ({}个):\n{}", rows.len(), items.join("\n")));
        }
    }

    if let Ok(rows) = database::all(
        "SELECT id, title, priority, due_date, done, updated_at FROM todos \
         WHERE done=0 AND updated_at > ?1 \
         ORDER BY due_date ASC",
        &[json!(since)],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let id = r["id"].as_str().unwrap_or("");
                let title = r["title"].as_str().unwrap_or("");
                let priority = r["priority"].as_str().unwrap_or("");
                let due = r["due_date"].as_str().unwrap_or("无");
                format!("  - {} [{}] {} (截止:{})", id, priority, title, due)
            }).collect();
            parts.push(format!("未完成待办 ({}个):\n{}", rows.len(), items.join("\n")));
        }
    }

    if let Ok(rows) = database::all(
        "SELECT id, name, budget, updated_at FROM projects \
         WHERE updated_at > ?1 AND status='Won' \
         ORDER BY updated_at DESC",
        &[json!(since)],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let id = r["id"].as_str().unwrap_or("");
                let name = r["name"].as_str().unwrap_or("");
                let budget = r["budget"].as_f64().unwrap_or(0.0);
                let updated = r["updated_at"].as_str().unwrap_or("");
                format!("  - {} {} ({:.0}万, {})", id, name, budget, &updated[..10.min(updated.len())])
            }).collect();
            parts.push(format!("近期中标:\n{}", items.join("\n")));
        }
    }

    if parts.is_empty() {
        "距上次分析以来无业务数据变化。".to_string()
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
