//! 工具执行层：每个 ext_tool_* 工具一个文件，零 agent 归属——
//! 哪个 Agent 能用哪些工具完全由 agent_tool_assignments 在运行时决定。

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
pub(crate) mod memory;
pub(crate) mod policy;
pub(crate) mod notification;

pub(crate) use common::*;
pub(crate) use search::*;
pub(crate) use chunk_navigate::*;
pub(crate) use object::*;
pub(crate) use relations::*;
pub(crate) use manage_todo::*;
pub(crate) use manage_product::*;
pub(crate) use memory::*;
// manage_project / manage_partner / manage_contact / policy / notification
// 入口统一为 execute()，仅经限定路径 tools::xxx::execute(...) 调用，不做 glob 重导出
