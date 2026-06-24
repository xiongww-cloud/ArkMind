//! 统一事件入口 — 所有业务对象 CRUD 的唯一副作用收敛点
//!
//! 纯轮询模式：object_changed 只负责触发 knowledge + tag 管线，
//! UI 刷新由前端 setInterval 轮询负责。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Note, Product, Project, Todo, File, Memory,
}

impl ObjectType {
    pub fn to_source_table(self) -> &'static str {
        match self {
            Self::Note => "notes", Self::Product => "products",
            Self::Project => "projects", Self::Todo => "todos",
            Self::File => "files", Self::Memory => "memory",
        }
    }
    pub fn has_vector(self) -> bool {
        matches!(self, Self::Note | Self::Product | Self::Project | Self::Todo | Self::File | Self::Memory)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { Created, Updated, Deleted }

pub fn object_changed(typ: ObjectType, id: &str, action: Action) {
    match action {
        Action::Created | Action::Updated => {
            crate::services::ai_sync::trigger_sync(typ.to_source_table(), id);
            crate::services::ai_tag::trigger_tagging(typ.to_source_table(), id);
        }
        Action::Deleted => {}
    }
}
