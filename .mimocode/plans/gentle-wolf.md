# ArkMind Phase 1: Planner 注入 — 从 ReAct 到 Cognitive Pipeline

## 目标

在现有 ReAct 循环前增加一个 **Planner 层**，让 LLM 输出"认知意图"而非"工具调用"，Runtime 自动编译为并行 Task DAG 执行。

**核心变化**：LLM 从"我先搜什么、再查什么"变成"我需要哪些信息"。

---

## 架构变更

```
当前：
User → LLM → tool_call(search) → result → LLM → tool_call(object) → result → LLM → answer
       └─────────── 串行循环 ──────────────────────────────────────────────────┘

Phase 1：
User → LLM(Planner) → TaskPlan → Compiler → DAG → join_all(Workers) → Merger → LLM(Answer)
       └── 1 轮 ──┘              └── 并行执行 ──┘         └── 1 轮 ──┘
```

---

## 数据结构

### TaskPlan（LLM 输出）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskPlan {
    goal: String,                    // 用户意图
    requirements: Vec<Requirement>,  // 需要的信息
    reasoning: Option<String>,       // LLM 的推理过程（可选）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Requirement {
    #[serde(rename = "type")]
    req_type: RequirementType,       // OBJECT | SEARCH | TRAVERSE | ACT
    target: Option<String>,          // 搜索目标/对象名
    keywords: Option<Vec<String>>,   // 搜索关键词
    chunk_id: Option<String>,        // traverse 用
    edge_type: Option<String>,       // traverse 边类型
    action: Option<String>,          // act 操作
    depends_on: Option<usize>,       // 依赖的 requirement index（用于 DAG）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum RequirementType {
    OBJECT,    // 对象发现
    SEARCH,    // 知识检索
    TRAVERSE,  // 图谱漫游
    ACT,       // 动作执行
}
```

### Task（编译后）

```rust
#[derive(Debug, Clone)]
struct Task {
    id: usize,
    req: Requirement,
    dependencies: Vec<usize>,  // 依赖的 task ids
    status: TaskStatus,
    result: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
```

---

## 实现步骤

### Step 1: 新增 Planner Prompt

在 `AGENT_SYSTEM_PROMPT` 之后新增 `PLANNER_PROMPT`：

```
你是一个认知规划器。你的任务不是调用工具，而是分析用户需求，输出信息获取计划。

## 输出格式（JSON）

{
    "goal": "用户意图的一句话描述",
    "requirements": [
        {
            "type": "OBJECT",
            "target": "对象名称或关键词",
            "reason": "为什么需要这个信息"
        },
        {
            "type": "SEARCH",
            "keywords": ["关键词1", "关键词2"],
            "reason": "为什么需要搜索"
        },
        {
            "type": "TRAVERSE",
            "chunk_id": "来自之前结果的 ChunkID",
            "edge_type": "Next|Prev|Related",
            "reason": "为什么需要漫游"
        }
    ],
    "reasoning": "你的推理过程"
}

## 规则

1. 只输出 JSON，不要输出其他内容
2. requirements 中的元素可以并行执行（系统会自动处理依赖）
3. 如果需要先找到对象再搜索，在 requirements 中同时列出两个——系统会自动处理顺序
4. 对于简单问题（如"你好"），requirements 可以为空数组
5. 每个 requirement 必须有 reason 字段说明为什么需要
```

### Step 2: 新增 `compile_plan_to_tasks()` 函数

```rust
fn compile_plan_to_tasks(plan: &TaskPlan, session: &SessionMemory) -> Vec<Task> {
    let mut tasks = Vec::new();
    for (i, req) in plan.requirements.iter().enumerate() {
        let mut dependencies = Vec::new();
        
        // 自动推断依赖：
        // TRAVERSE 依赖于前一个 OBJECT/SEARCH 的结果（需要 chunk_id）
        if req.req_type == RequirementType::TRAVERSE {
            if let Some(chunk_id) = &req.chunk_id {
                // chunk_id 来自之前的结果，找到对应的 task
                for (j, prev_req) in plan.requirements.iter().enumerate() {
                    if j < i && prev_req.req_type == RequirementType::OBJECT {
                        dependencies.push(j);
                    }
                }
            }
        }
        
        tasks.push(Task {
            id: i,
            req: req.clone(),
            dependencies,
            status: TaskStatus::Pending,
            result: None,
        });
    }
    tasks
}
```

### Step 3: 新增 `execute_plan()` 函数

```rust
async fn execute_plan(
    tasks: &mut Vec<Task>,
    session: &SessionMemory,
    visited_sources: &std::collections::HashSet<String>,
    visited_chunks: &std::collections::HashSet<String>,
    traverse_source_counter: &mut std::collections::HashMap<i64, usize>,
) -> Vec<String> {
    // 拓扑排序，按层级并行执行
    let mut results = Vec::new();
    let mut executed = std::collections::HashSet::new();
    
    loop {
        // 找出所有 dependencies 已满足的 pending tasks
        let ready: Vec<usize> = tasks.iter()
            .enumerate()
            .filter(|(i, t)| t.status == TaskStatus::Pending 
                && t.dependencies.iter().all(|d| executed.contains(d)))
            .map(|(i, _)| i)
            .collect();
        
        if ready.is_empty() { break; }
        
        // 并行执行这一层
        let mut handles = Vec::new();
        for idx in ready {
            let task = &tasks[idx];
            let result = match task.req.req_type {
                RequirementType::OBJECT => {
                    let q = task.req.target.as_deref().unwrap_or("");
                    execute_object_get(q, "", None, 20)
                }
                RequirementType::SEARCH => {
                    let kw = task.req.keywords.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
                    let (res, _) = execute_search_knowledge(kw, None, visited_sources, visited_chunks, None).await;
                    res
                }
                RequirementType::TRAVERSE => {
                    let cid = task.req.chunk_id.as_deref().unwrap_or("");
                    let edge = task.req.edge_type.as_deref().unwrap_or("Next");
                    execute_traverse(cid, edge, 10, visited_chunks, "", traverse_source_counter)
                }
                RequirementType::ACT => {
                    let action = task.req.action.as_deref().unwrap_or("");
                    execute_act(action, "", "", "", "", "")
                }
            };
            results.push(result);
            executed.insert(idx);
        }
    }
    
    results
}
```

### Step 4: 新增 `merge_results()` 函数

```rust
fn merge_results(results: &[String], session: &SessionMemory) -> String {
    let mut merged = String::new();
    let mut seen_chunks = std::collections::HashSet::new();
    let mut seen_sources = std::collections::HashSet::new();
    
    for result in results {
        for line in result.lines() {
            let tl = line.trim();
            // 去重：按 ChunkID
            if let Some(cid) = tl.strip_prefix("ChunkID：") {
                let short: String = cid.chars().take(8).collect();
                if !seen_chunks.insert(short) { continue; }
            }
            // 去重：按 source_id
            if let Some(sid) = tl.split("source_id: ").nth(1) {
                let sid = sid.trim();
                if !seen_sources.insert(sid.to_string()) { continue; }
            }
            merged.push_str(line);
            merged.push('\n');
        }
        merged.push('\n');
    }
    
    merged
}
```

### Step 5: 修改 `agent_chat_workflow()` 主函数

在现有 ReAct 循环前插入 Planner 阶段：

```rust
// ═══════════════════════════════════════════════════════════════════
// Phase 1: Planner — 认知意图提取
// ═══════════════════════════════════════════════════════════════════

let planner_messages = vec![
    ChatMessage { role: MessageRole::System, content: Some(PLANNER_PROMPT.to_string()), .. },
    ChatMessage { role: MessageRole::User, content: Some(query.clone()), .. },
];

// 调用 LLM（不带 tools，强制输出 JSON）
let plan_response = call_llm_without_tools(&planner_messages, &app_handle).await?;

// 解析 TaskPlan
let plan: TaskPlan = serde_json::from_str(&plan_response).unwrap_or(TaskPlan {
    goal: query.clone(),
    requirements: vec![],
    reasoning: None,
});

// 如果 requirements 为空，走原来的 ReAct 循环
if plan.requirements.is_empty() {
    // ... 原有 ReAct 逻辑 ...
}

// ═══════════════════════════════════════════════════════════════════
// Phase 2: Compile — TaskPlan → Task DAG
// ═══════════════════════════════════════════════════════════════════

let mut tasks = compile_plan_to_tasks(&plan, &session);

// ═══════════════════════════════════════════════════════════════════
// Phase 3: Execute — 并行执行 Tasks
// ═══════════════════════════════════════════════════════════════════

let results = execute_plan(&mut tasks, &session, ...).await;

// ═══════════════════════════════════════════════════════════════════
// Phase 4: Merge — 去重 + 排序 + 融合
// ═══════════════════════════════════════════════════════════════════

let merged = merge_results(&results, &session);

// ═══════════════════════════════════════════════════════════════════
// Phase 5: Answer — LLM 理解与生成
// ═══════════════════════════════════════════════════════════════════

messages.push(ChatMessage {
    role: MessageRole::User,
    content: Some(format!("已获取以下信息：\n\n{}\n\n请基于以上信息回答：{}", merged, query)),
    ..,
});

let answer = call_llm_without_tools(&messages, &app_handle).await?;
```

---

## 关键文件改动

| 文件 | 改动 | 行数 |
|------|------|------|
| `agent_workflow.rs` | 新增 TaskPlan/Requirement 结构体 | ~30 行 |
| `agent_workflow.rs` | 新增 PLANNER_PROMPT 常量 | ~25 行 |
| `agent_workflow.rs` | 新增 `compile_plan_to_tasks()` | ~40 行 |
| `agent_workflow.rs` | 新增 `execute_plan()` | ~60 行 |
| `agent_workflow.rs` | 新增 `merge_results()` | ~30 行 |
| `agent_workflow.rs` | 修改 `agent_chat_workflow()` | ~50 行 |
| **总计** | | **~235 行** |

---

## 验证方案

1. **单元测试**：`compile_plan_to_tasks()` — 验证 DAG 依赖推断
2. **集成测试**：用固定 query 测试 Planner 输出 → TaskPlan 解析
3. **端到端测试**：
   - 简单问题："你好" → requirements 为空 → 走 ReAct
   - 中等问题："18500是什么" → 1-2 个 requirements → 并行执行
   - 复杂问题："介绍18500" → 3-4 个 requirements → 并行执行 → 合并结果
4. **性能对比**：对比 Phase 1 前后的 LLM 调用轮次和 token 消耗

---

## 风险缓解

1. **Planner 输出格式不稳定** → 用 `serde_json::from_str` + fallback
2. **并行执行失败** → 每个 Worker 有 timeout + error handling
3. **DAG 循环依赖** → `compile_plan_to_tasks()` 检测并跳过
4. **回退机制** → 如果 Planner 失败，自动降级到原有 ReAct 循环
