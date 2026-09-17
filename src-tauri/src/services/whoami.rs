// 用户身份基座：系统对"我是谁"的确定性表示 + 身份层自动维护。
// 身份层（姓名/职位/公司）：从用户自己发出的邮件显示名与签名档自动提取（证据是用户自身行为，注入面小），
//   用户撤销 = 负反馈记入 rejected，同值不再覆盖，直到用户手动编辑该字段。
// 画像层（习惯/承诺/偏好事实）：第三方语料存在注入面，保持用户点击确认（profile_fact_save）。
// 数据源：users 单例行 + email_accounts（我的邮箱，自动推导）+ memories source='profile'。
// 仅供智能问答主 Agent 与通知分析师注入使用；子 Agent / 委托运行体不注入。

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::Emitter;

const DEFAULT_NAME: &str = "本地管理员";
pub const FACT_LIMIT: usize = 20;
pub const FACT_TRUNC: usize = 200;
const META_KEY: &str = "profile_identity_meta";
pub const IDENTITY_FIELDS: [&str; 4] = ["name", "title", "company", "work_city"];

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn get_setting(key: &str) -> Option<String> {
    crate::database::get(
        "SELECT value FROM system_settings WHERE key=?1",
        &[json!(key)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["value"].as_str().map(String::from))
}

fn set_setting(key: &str, value: &str) {
    let _ = crate::database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES(?1, ?2, datetime('now','localtime'))",
        &[json!(key), json!(value)],
    );
}

fn load_meta() -> Value {
    get_setting(META_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn save_meta(m: &Value) {
    set_setting(META_KEY, &m.to_string());
}

#[derive(Default, Clone)]
pub struct Whoami {
    pub name: String,
    pub title: String,
    pub company: String,
    pub work_city: String,
    pub emails: Vec<String>,
    /// (memory_id, 陈述文本)
    pub facts: Vec<(String, String)>,
}

impl Whoami {
    pub fn load() -> Self {
        let mut w = Self::default();
        if let Some(r) = crate::database::get(
            "SELECT name, title, company, work_city FROM users WHERE id='local-admin' LIMIT 1",
            &[],
        )
        .ok()
        .flatten()
        {
            let name = r["name"].as_str().unwrap_or("").trim().to_string();
            if !name.is_empty() && name != DEFAULT_NAME {
                w.name = name;
            }
            w.title = r["title"].as_str().unwrap_or("").trim().to_string();
            w.company = r["company"].as_str().unwrap_or("").trim().to_string();
            w.work_city = r["work_city"].as_str().unwrap_or("").trim().to_string();
        }
        w.emails = crate::database::all("SELECT email_address FROM email_accounts", &[])
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r["email_address"].as_str().map(norm))
            .filter(|s| !s.is_empty())
            .collect();
        w.facts = crate::database::all(
            "SELECT id, content FROM memories WHERE source='profile' ORDER BY updated_at DESC LIMIT ?1",
            &[json!(FACT_LIMIT as i64)],
        )
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let id = r["id"].as_str()?.to_string();
            let c = r["content"].as_str()?.to_string();
            let t: String = c.chars().take(FACT_TRUNC).collect();
            let text = if t.chars().count() < c.chars().count() { format!("{}…", t) } else { t };
            Some((id, text))
        })
        .collect();
        w
    }

    pub fn is_my_email(&self, addr: &str) -> bool {
        let a = norm(addr);
        !a.is_empty() && self.emails.iter().any(|e| e == &a)
    }

    /// 【关于我】+【用户画像】注入块；主 Agent 与分析师共用同一口径。无任何信息时返回空串。
    pub fn prompt_block(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.name.is_empty() {
            lines.push(format!("姓名: {}", self.name));
        }
        if !self.title.is_empty() {
            lines.push(format!("职位: {}", self.title));
        }
        if !self.company.is_empty() {
            lines.push(format!("公司: {}", self.company));
        }
        if !self.work_city.is_empty() {
            lines.push(format!("工作城市: {}", self.work_city));
        }
        if !self.emails.is_empty() {
            lines.push(format!("我的邮箱: {}", self.emails.join(", ")));
        }
        let mut s = String::new();
        if !lines.is_empty() {
            s.push_str(&format!("【关于我】\n{}\n", lines.join("\n")));
        }
        if !self.facts.is_empty() {
            let facts: Vec<String> = self.facts.iter().map(|(id, f)| format!("- {} (id:{})", f, id)).collect();
            s.push_str(&format!(
                "\n【用户画像】（用户确认的个人事实；括号内 id 可用于 ext_tool_memory 的 update/delete 定位）\n{}\n",
                facts.join("\n")
            ));
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.title.is_empty()
            && self.company.is_empty()
            && self.work_city.is_empty()
            && self.emails.is_empty()
            && self.facts.is_empty()
    }
}

// ─── 证据收集与解析（纯逻辑，可测）─────────────────────────

/// 收集"用户是谁"的确定性证据：本人发出邮件的显示名分布 + 签名档样本（正文尾部）
pub fn collect_profile_evidence(w: &Whoami) -> String {
    let mut ev: Vec<String> = Vec::new();
    let mut sig_lines: Vec<String> = Vec::new();
    for me in &w.emails {
        let rows = crate::database::all(
            "SELECT from_name, COUNT(*) AS n FROM emails WHERE lower(from_addr)=?1 AND from_name<>'' GROUP BY from_name ORDER BY n DESC LIMIT 3",
            &[json!(me)],
        )
        .unwrap_or_default();
        for r in rows {
            ev.push(format!(
                "- 发件显示名「{}」出现 {} 次（邮箱 {}）",
                r["from_name"].as_str().unwrap_or(""),
                r["n"].as_i64().unwrap_or(0),
                me
            ));
        }
        let bodies = crate::database::all(
            "SELECT body_text FROM emails WHERE lower(from_addr)=?1 AND body_text<>'' ORDER BY received_at DESC LIMIT 5",
            &[json!(me)],
        )
        .unwrap_or_default();
        for r in bodies.iter().take(3) {
            let b = r["body_text"].as_str().unwrap_or("");
            let tail: String = b.chars().rev().take(260).collect::<Vec<_>>().into_iter().rev().collect();
            let tail = tail.trim();
            if !tail.is_empty() {
                sig_lines.push(format!("- 「…{}」", tail.replace('\n', " / ")));
            }
        }
    }
    if ev.is_empty() && sig_lines.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if !ev.is_empty() {
        out.push_str(&format!("发件人显示名统计：\n{}\n", ev.join("\n")));
    }
    if !sig_lines.is_empty() {
        out.push_str(&format!(
            "\n本人发出邮件的正文尾部片段（签名档常在末尾）：\n{}\n",
            sig_lines.join("\n")
        ));
    }
    out
}

/// 从 LLM 回复中提取首个 JSON 对象（容忍 ```json 围栏与前后闲文；按括号平衡扫描，不被后续对象干扰）
pub fn extract_json_object(s: &str) -> Option<Value> {
    let bytes: Vec<char> = s.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c != '{' {
            continue;
        }
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        for (j, d) in bytes.iter().enumerate().skip(i) {
            if in_str {
                if esc {
                    esc = false;
                } else if *d == '\\' {
                    esc = true;
                } else if *d == '"' {
                    in_str = false;
                }
                continue;
            }
            match d {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) =
                            serde_json::from_str::<Value>(&bytes[i..=j].iter().collect::<String>())
                        {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// 规划身份层更新：跳过空值、与当前值相同项、用户曾撤销过的值（rejected）
pub fn plan_identity_update(
    cur: &Map<String, Value>,
    extracted: &Value,
    rejected: &Value,
) -> Vec<(String, String)> {
    let mut ups = Vec::new();
    for f in IDENTITY_FIELDS {
        let nv = extracted[f].as_str().unwrap_or("").trim().to_string();
        if nv.is_empty() {
            continue;
        }
        if cur.get(f).and_then(|v| v.as_str()).unwrap_or("") == nv {
            continue;
        }
        let is_rejected = rejected[f]
            .as_array()
            .map_or(false, |a| a.iter().any(|x| x.as_str() == Some(nv.as_str())));
        if is_rejected {
            continue;
        }
        ups.push((f.to_string(), nv));
    }
    ups
}

// ─── 自动维护主流程（host 直调，不经分析师的手；证据只来自用户自身发出的邮件）────

pub async fn auto_update_identity(app: &tauri::AppHandle) {
    let w = Whoami::load();
    let evidence = collect_profile_evidence(&w);
    if evidence.is_empty() {
        return; // 尚无发件证据
    }
    // v2 指纹：新增 work_city 字段后 bump 一次，让存量档案下轮补提取城市（之后重新稳定）
    let fp = sha256_hex(&format!("{}|v2", evidence));
    let mut meta = load_meta();
    if meta["extract_fp"].as_str() == Some(fp.as_str()) {
        return; // 证据未变，零成本跳过
    }
    let msgs = vec![
        crate::services::agent_workflow::ChatMessage {
            role: crate::services::agent_workflow::MessageRole::System,
            content: Some("你是用户档案提取器。根据用户自己发出邮件的证据（显示名统计、正文尾部签名档），提取用户的姓名、职位、公司、工作城市。只输出一个JSON对象：{\"name\":\"\",\"title\":\"\",\"company\":\"\",\"work_city\":\"\"}。工作城市仅到城市级（如\"上海\"），从签名档地址或办公地点判断；提取不到的字段必须留空字符串，严禁猜测编造。注意：签名档证据可能混有正文尾巴或免责声明，从中辨别真正的姓名/职位/公司/地址。".to_string()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        },
        crate::services::agent_workflow::ChatMessage {
            role: crate::services::agent_workflow::MessageRole::User,
            content: Some(evidence.clone()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        },
    ];
    let raw = match crate::services::agent_workflow::llm::call_llm_without_tools(
        &msgs,
        &Some(app.clone()),
        true,
        false,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[whoami] 身份提取 LLM 调用失败: {}", e);
            return; // 证据指纹不推进，下轮重试
        }
    };
    let Some(parsed) = extract_json_object(&raw) else {
        log::warn!("[whoami] 身份提取结果无法解析: {}", raw.chars().take(120).collect::<String>());
        return;
    };
    meta["extract_fp"] = json!(fp); // 证据已消费（无论有无变化），避免每轮重复调 LLM

    let mut cur = Map::new();
    cur.insert("name".into(), json!(w.name));
    cur.insert("title".into(), json!(w.title));
    cur.insert("company".into(), json!(w.company));
    cur.insert("work_city".into(), json!(w.work_city));
    let ups = plan_identity_update(&cur, &parsed, &meta["rejected"]);
    if ups.is_empty() {
        save_meta(&meta);
        return;
    }

    // before 快照 + 本轮采用的自动值（撤销时才转 rejected；未撤销时 plan 因"同值"天然跳过，不重复覆盖）
    let mut before = Map::new();
    let mut set_fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let mut diff_lines: Vec<String> = Vec::new();
    let mut rejected_add = Map::new();
    let labels = [("name", "姓名"), ("title", "职位"), ("company", "公司"), ("work_city", "工作城市")];
    for (f, nv) in &ups {
        let old = cur.get(f).and_then(|v| v.as_str()).unwrap_or("").to_string();
        before.insert(f.clone(), json!(old));
        set_fields.push(format!("{f}=?"));
        values.push(json!(nv));
        let zh = labels.iter().find(|(k, _)| k == f).map(|(_, l)| *l).unwrap_or(f);
        diff_lines.push(format!(
            "- {}：{} → **{}**",
            zh,
            if old.is_empty() { "（未识别）" } else { &old },
            nv
        ));
        rejected_add.insert(f.clone(), json!(nv));
    }
    let _ = crate::database::run(
        "INSERT OR IGNORE INTO users (id,name,username,role) VALUES ('local-admin','本地管理员','local-admin','OWNER')",
        &[],
    );
    values.push(json!("local-admin"));
    if crate::database::run(
        &format!(
            "UPDATE users SET {}, updated_at=datetime('now','localtime') WHERE id=?{}",
            set_fields.join(","),
            values.len() - 1
        ),
        &values,
    )
    .is_err()
    {
        save_meta(&meta);
        return;
    }
    meta["updated_at"] = json!(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    meta["source"] = json!("auto");
    save_meta(&meta);

    // 轻反馈通知（可撤销；撤销走 profile_identity_revert，恢复 before 并解除本轮 rejected 预置）
    let id = uuid::Uuid::new_v4().to_string();
    let title = "档案已自动更新".to_string();
    let content = format!(
        "我从你发出的邮件里更新了你的档案：\n{}\n\n若不符合，点「撤销」恢复；撤销后我不会再用相同结果覆盖。也可以到 设置-我的档案 手动纠正。",
        diff_lines.join("\n")
    );
    let actions = json!([{
        "type": "revert_profile",
        "label": "撤销",
        "params": { "before": Value::Object(before), "revert_rejected": Value::Object(rejected_add) }
    }]);
    let _ = crate::database::run(
        "INSERT INTO notifications(id,type,title,content,entity_type,entity_id,priority,source,actions) VALUES(?1,'profile_updated',?2,?3,NULL,NULL,'low','system',?4)",
        &[json!(&id), json!(title), json!(content), json!(actions.to_string())],
    );
    let _ = app.emit("notification:new", &id);
    log::info!("[whoami] 身份层自动更新: {:?}", ups.iter().map(|(f, _)| f).collect::<Vec<_>>());
}

/// 用户撤销自动更新：回写 before + 把被撤销的自动值记入 rejected（不再用相同值覆盖）+ 标记 source=manual
pub fn revert_identity(before: &Value, revert_rejected: &Value) -> Result<(), String> {
    let mut set_fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    for f in IDENTITY_FIELDS {
        if let Some(v) = before[f].as_str() {
            set_fields.push(format!("{f}=?"));
            values.push(json!(v));
        }
    }
    if set_fields.is_empty() {
        return Ok(());
    }
    let _ = crate::database::run(
        "INSERT OR IGNORE INTO users (id,name,username,role) VALUES ('local-admin','本地管理员','local-admin','OWNER')",
        &[],
    );
    values.push(json!("local-admin"));
    crate::database::run(
        &format!(
            "UPDATE users SET {}, updated_at=datetime('now','localtime') WHERE id=?{}",
            set_fields.join(","),
            values.len() - 1
        ),
        &values,
    )
    .map_err(|e| e.to_string())?;
    let mut meta = load_meta();
    if let Some(rr) = revert_rejected.as_object() {
        if !meta["rejected"].is_object() {
            meta["rejected"] = json!({});
        }
        let obj = meta["rejected"].as_object_mut().unwrap();
        for (f, v) in rr {
            let arr = obj.entry(f.clone()).or_insert_with(|| json!([]));
            if let Some(a) = arr.as_array_mut() {
                if !a.iter().any(|x| x == v) {
                    a.push(v.clone());
                }
            }
        }
    }
    meta["updated_at"] = json!(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    meta["source"] = json!("manual");
    save_meta(&meta);
    Ok(())
}

/// 用户手动编辑档案后：清除对应字段的 rejected（用户主动改变认知，允许 AI 重新参与维护）
pub fn clear_rejected_for(fields: &[&str]) {
    let mut meta = load_meta();
    if let Some(obj) = meta["rejected"].as_object_mut() {
        for f in fields {
            obj.remove(*f);
        }
    }
    meta["updated_at"] = json!(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    meta["source"] = json!("manual");
    save_meta(&meta);
}

pub fn identity_meta() -> Value {
    load_meta()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> Map<String, Value> {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), json!(*v));
        }
        m
    }

    #[test]
    fn prompt_block_renders_all_sections_and_empty_is_blank() {
        let mut w = Whoami::default();
        assert_eq!(w.prompt_block(), "");
        w.name = "熊某某".into();
        w.title = "项目经理".into();
        w.emails = vec!["a@x.com".into()];
        w.facts = vec![("m1".into(), "用户习惯周三前交付方案".into())];
        let b = w.prompt_block();
        assert!(b.contains("【关于我】"));
        assert!(b.contains("姓名: 熊某某"));
        assert!(b.contains("我的邮箱: a@x.com"));
        assert!(b.contains("【用户画像】"));
        assert!(b.contains("- 用户习惯周三前交付方案 (id:m1)"));
    }

    #[test]
    fn is_my_email_normalizes_case_and_space() {
        let mut w = Whoami::default();
        w.emails = vec!["xiongww@kinghighway.com".into()];
        assert!(w.is_my_email("  XiongWW@KingHighway.com "));
        assert!(!w.is_my_email("other@kinghighway.com"));
        assert!(!w.is_my_email(""));
    }

    #[test]
    fn plan_skips_empty_identical_and_rejected() {
        let cur = map(&[("name", "熊某"), ("title", ""), ("company", "旧公司"), ("work_city", "上海")]);
        let extracted = json!({"name": "熊某", "title": "项目经理", "company": "新公司", "work_city": "上海"});
        let rejected = json!({"company": ["新公司"]});
        let ups = plan_identity_update(&cur, &extracted, &rejected);
        assert_eq!(ups, vec![("title".to_string(), "项目经理".to_string())]);
    }

    #[test]
    fn plan_writes_missing_fields_and_ignores_garbage() {
        let cur = map(&[]);
        let ups = plan_identity_update(
            &cur,
            &json!({"name": " 熊某 ", "title": "", "nope": "x"}),
            &json!({}),
        );
        assert_eq!(ups, vec![("name".to_string(), "熊某".to_string())]);
    }

    #[test]
    fn extract_json_object_tolerates_fences_and_prose() {
        let v = extract_json_object("好的：\n```json\n{\"name\":\"熊某\",\"title\":\"\"}\n```").unwrap();
        assert_eq!(v["name"], json!("熊某"));
        assert_eq!(v["title"], json!(""));
        assert!(extract_json_object("没有大括号").is_none());
        assert!(extract_json_object("} 先闭后开 {").is_none());
        assert!(extract_json_object("{ 残缺不闭").is_none());
    }

    #[test]
    fn extract_json_object_picks_first_balanced() {
        let v = extract_json_object("前置闲话 {\"a\":1} 尾巴 {\"b\":2}").unwrap();
        assert_eq!(v["a"], json!(1));
    }
}
