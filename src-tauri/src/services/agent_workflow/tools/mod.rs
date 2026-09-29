//! 工具执行层：每个 ext_tool_* 工具一个文件，零 agent 归属——
//! 哪个 Agent 能用哪些工具完全由 agent_tool_assignments 在运行时决定。

/// 主 Agent（认知协调层）专属工具：委托给子 Agent 时一律剥离，
/// 前端工具分配列表也不展示。消费方：delegate_agent（运行时过滤）、
/// agent_commands（UI 列表过滤 + 分配写入过滤）。
pub(crate) const MAIN_AGENT_ONLY: &[&str] = &[
    "ext_tool_delegate_agent",
    "ext_tool_memory",
    "ext_tool_ask_user",
];

pub(crate) mod common;
pub(crate) mod search;
pub(crate) mod chunk_navigate;
pub(crate) mod object;
pub(crate) mod relations;
pub(crate) mod manage_todo;
pub(crate) mod manage_project;
pub(crate) mod manage_partner;
pub(crate) mod manage_contact;
pub(crate) mod manage_product;
pub(crate) mod manage_note;
pub(crate) mod memory;
pub(crate) mod notification;
pub(crate) mod write_document;

pub(crate) use common::*;
pub(crate) use search::*;
pub(crate) use chunk_navigate::*;
pub(crate) use object::*;
pub(crate) use relations::*;
pub(crate) use manage_todo::*;
pub(crate) use manage_product::*;
pub(crate) use manage_note::*;
pub(crate) use memory::*;
// manage_project / manage_partner / manage_contact / notification
// 入口统一为 execute()，仅经限定路径 tools::xxx::execute(...) 调用，不做 glob 重导出

#[cfg(test)]
mod dedup_tests {
    use crate::database;
    use serde_json::{json, Value};

    /// 同名复用回归：预置已存在行 → create 应返回 duplicate=true + 已有 ID，不再插入。
    ///（只测命中路径：未命中会走 block_on_async 的完整创建链，需要 tokio 运行时与更多依赖表）
    fn setup_with_existing() -> (
        tempfile::TempDir,
        std::sync::MutexGuard<'static, ()>,
    ) {
        let (dir, guard) = database::db_core::test_util::setup_test_db();
        database::with_db(|conn| crate::database::db_schema::create_schema(conn)).unwrap();
        (dir, guard)
    }

    fn envelope(res: &str) -> Value {
        serde_json::from_str(res).expect("envelope must be valid JSON")
    }

    #[test]
    fn todo_create_reuses_exact_title() {
        let (_d, _g) = setup_with_existing();
        database::run(
            "INSERT INTO todos(id, title) VALUES('T1', '买服务器')",
            &[],
        ).unwrap();
        let res = super::manage_todo::ext_tool_manage_todo(
            "create", "买服务器", "新内容", "", "", "", "", None, "",
        );
        let v = envelope(&res);
        assert_eq!(v["result_type"], "object_create_result");
        assert_eq!(v["data"]["success"], true);
        assert_eq!(v["data"]["duplicate"], true);
        assert_eq!(v["data"]["id"], "T1");
        let summary = v["summary"].as_str().unwrap();
        assert!(summary.contains("已复用"), "summary={}", summary);
        // 未新增行
        let count: i64 = database::get("SELECT COUNT(*) AS c FROM todos", &[])
            .unwrap().unwrap()["c"].as_i64().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn note_create_same_content_reuses_but_conflict_on_diff() {
        let (_d, _g) = setup_with_existing();
        database::run(
            "INSERT INTO notes(note_id, title, content) VALUES('N1', '周报', '一样的正文')",
            &[],
        ).unwrap();
        // 同名+同内容 → 复用
        let res = super::manage_note::ext_tool_manage_note(
            "create", "周报", "一样的正文", "", "", "",
        );
        let v = envelope(&res);
        assert_eq!(v["data"]["success"], true);
        assert_eq!(v["data"]["duplicate"], true);
        assert_eq!(v["data"]["id"], "N1");
        // 同名+不同内容 → 拒绝并给出已有 ID（防新内容静默丢失）
        let res2 = super::manage_note::ext_tool_manage_note(
            "create", "周报", "另一份正文", "", "", "",
        );
        let v2 = envelope(&res2);
        assert_eq!(v2["data"]["success"], false);
        assert_eq!(v2["data"]["duplicate_conflict"], true);
        assert_eq!(v2["data"]["id"], "N1");
        let s2 = v2["summary"].as_str().unwrap();
        assert!(s2.contains("update"), "应引导 update: {}", s2);
        // 未新增行
        let count: i64 = database::get("SELECT COUNT(*) AS c FROM notes", &[])
            .unwrap().unwrap()["c"].as_i64().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn partner_create_reuses_same_type_and_name() {
        let (_d, _g) = setup_with_existing();
        database::run(
            "INSERT INTO partners(id, type, name) VALUES('P1', 'VENDOR', '华为')",
            &[],
        ).unwrap();
        let res = super::manage_partner::execute(&json!({
            "action": "create", "type": "vendor", "name": "华为"
        }));
        let v = envelope(&res);
        assert_eq!(v["data"]["success"], true);
        assert_eq!(v["data"]["duplicate"], true);
        assert_eq!(v["data"]["id"], "P1");
        let count: i64 = database::get("SELECT COUNT(*) AS c FROM partners", &[])
            .unwrap().unwrap()["c"].as_i64().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn project_create_reuses_exact_name() {
        let (_d, _g) = setup_with_existing();
        database::run(
            "INSERT INTO projects(id, name) VALUES('PJ1', '智慧城市一期')",
            &[],
        ).unwrap();
        let res = super::manage_project::execute(&json!({
            "action": "create", "name": "智慧城市一期"
        }));
        let v = envelope(&res);
        assert_eq!(v["data"]["success"], true);
        assert_eq!(v["data"]["duplicate"], true);
        assert_eq!(v["data"]["id"], "PJ1");
        let count: i64 = database::get("SELECT COUNT(*) AS c FROM projects", &[])
            .unwrap().unwrap()["c"].as_i64().unwrap();
        assert_eq!(count, 1);
    }
}
