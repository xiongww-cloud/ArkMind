// 用户身份基座：系统对"我是谁"的确定性表示。
// 数据源：users 单例行（姓名/职位/公司）+ email_accounts（我的邮箱，自动推导）+ memories source='profile'（画像事实）。
// 仅供智能问答主 Agent 与通知分析师注入使用；子 Agent / 委托运行体不注入。

use serde_json::json;

const DEFAULT_NAME: &str = "本地管理员";
pub const FACT_LIMIT: usize = 20;
pub const FACT_TRUNC: usize = 200;

#[derive(Default, Clone)]
pub struct Whoami {
    pub name: String,
    pub title: String,
    pub company: String,
    pub emails: Vec<String>,
    pub facts: Vec<String>,
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

impl Whoami {
    pub fn load() -> Self {
        let mut w = Self::default();
        if let Some(r) = crate::database::get(
            "SELECT name, title, company FROM users WHERE id='local-admin' LIMIT 1",
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
        }
        w.emails = crate::database::all("SELECT email_address FROM email_accounts", &[])
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r["email_address"].as_str().map(norm))
            .filter(|s| !s.is_empty())
            .collect();
        w.facts = crate::database::all(
            "SELECT content FROM memories WHERE source='profile' ORDER BY updated_at DESC LIMIT ?1",
            &[json!(FACT_LIMIT as i64)],
        )
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r["content"].as_str().map(|s| s.to_string()))
        .map(|s| {
            let t: String = s.chars().take(FACT_TRUNC).collect();
            if t.chars().count() < s.chars().count() {
                format!("{}…", t)
            } else {
                t
            }
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
        if !self.emails.is_empty() {
            lines.push(format!("我的邮箱: {}", self.emails.join(", ")));
        }
        let mut s = String::new();
        if !lines.is_empty() {
            s.push_str(&format!("【关于我】\n{}\n", lines.join("\n")));
        }
        if !self.facts.is_empty() {
            let facts: Vec<String> = self.facts.iter().map(|f| format!("- {f}")).collect();
            s.push_str(&format!("\n【用户画像】（用户确认的个人事实）\n{}\n", facts.join("\n")));
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.title.is_empty()
            && self.company.is_empty()
            && self.emails.is_empty()
            && self.facts.is_empty()
    }
}

/// 渲染层用：档案 + 我的邮箱 + 画像事实计数
pub fn profile_snapshot() -> serde_json::Value {
    let w = Whoami::load();
    json!({
        "name": w.name,
        "title": w.title,
        "company": w.company,
        "email_addresses": w.emails,
        "profile_fact_count": w.facts.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_block_renders_all_sections_and_empty_is_blank() {
        let mut w = Whoami::default();
        assert_eq!(w.prompt_block(), "");
        w.name = "熊某某".into();
        w.title = "项目经理".into();
        w.emails = vec!["a@x.com".into()];
        w.facts = vec!["用户习惯周三前交付方案".into()];
        let b = w.prompt_block();
        assert!(b.contains("【关于我】"));
        assert!(b.contains("姓名: 熊某某"));
        assert!(b.contains("我的邮箱: a@x.com"));
        assert!(b.contains("【用户画像】"));
        assert!(b.contains("- 用户习惯周三前交付方案"));
    }

    #[test]
    fn is_my_email_normalizes_case_and_space() {
        let mut w = Whoami::default();
        w.emails = vec!["xiongww@kinghighway.com".into()];
        assert!(w.is_my_email("  XiongWW@KingHighway.com "));
        assert!(!w.is_my_email("other@kinghighway.com"));
        assert!(!w.is_my_email(""));
    }
}
