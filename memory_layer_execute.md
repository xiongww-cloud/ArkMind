# ArkMind Memory Layer — 执行方案

## 核心原则

1. **source_data 是唯一的记忆容器** — 不新增任何存储体系
2. **Object 是从记忆中识别的世界实体** — 不是预存的业务表
3. **Object Link 是实体间演化的关系** — 有语义、有来源、可追溯
4. **知识由用户教** — 系统是学生，用户是老师
5. **理解不自动记忆** — 系统推断的结论不入库

---

## 一、概念定义

### 什么是"知识"

用户明确陈述的事实、规则、偏好。例如：

- "5310主要用于PACS"
- "报价不要低于8折"
- "老王是天门项目的负责人"

特征：**用户主动教、信息确定、值得长期保存**

### 什么是"理解"

系统从对话中推断出的结论。例如：

- "看起来5310在PACS领域有优势"
- "天门项目可能有风险"

特征：**系统推断、不确定、可能随上下文变化**

### 规则

| 类型 | 来源 | 是否记忆 | 示例 |
|------|------|---------|------|
| 知识 | 用户教的 | ✅ 直接入库 | "5310主要用于PACS" |
| 理解 | 系统推断 | ❌ 不入库 | "看起来5310在PACS有优势" |
| 总结 | 系统归纳 | ❌ 不入库 | "本次对话讨论了18500的规格" |

---

## 二、数据模型

### 不变的部分

```sql
-- source_data 仍然是唯一的记忆容器
-- 只扩展 source_type，不新增表
source_data: id, source_type, source_id, title, content, created_at, updated_at

-- knowledge_chunks 仍然统一挂载
knowledge_chunks: chunk_id, source_data_id, content, ...

-- global_search_fts 仍然统一检索
global_search_fts: source_type, source_id, title, content
```

### 新增的 source_type

```sql
-- 在 source_data 中新增以下类型（通过 Rust 代码创建，不需要 trigger）
-- MEMORY:    用户教的知识
-- PERSON:    人物（从对话中识别）
-- POLICY:    规则/偏好
```

**注意**：SUMMARY 和 EXPERIENCE 暂不实现。先做最核心的 MEMORY + PERSON + POLICY。

### Object Link 语义升级

```sql
-- object_links 新增 edge_type 列
ALTER TABLE object_links ADD COLUMN edge_type TEXT DEFAULT 'CORRELATE';

-- edge_type 枚举：
-- CORRELATE:  普通关联（默认，兼容旧数据）
-- LEARN_FROM: MEMORY 来自某个 CHAT
-- MENTIONS:   MEMORY 提到了某个 Object
-- CORRECTS:   MEMORY 修正了另一个 MEMORY
-- TEACHES:    USER 教了某个 MEMORY
```

---

## 三、MEMORY 创建流程

### 触发时机

用户消息中包含明确知识时，在 LLM 回复中识别并记忆。

### 判断标准

LLM 在回复时判断用户消息是否为"明确知识"：

```json
{
    "is_knowledge": true,
    "type": "MEMORY",
    "title": "5310主要用于PACS",
    "content": "5310主要用于PACS系统",
    "mentions": ["PRODUCT-5310"],
    "confidence": 0.9
}
```

### 创建逻辑

```
用户: "5310主要用于PACS"
        ↓
LLM 判断: is_knowledge = true
        ↓
创建 source_data:
  type = MEMORY
  title = "5310主要用于PACS"
  content = "5310主要用于PACS"
        ↓
建立 Object Links:
  MEMORY(5310用于PACS) --[TEACHES]--> USER
  MEMORY(5310用于PACS) --[MENTIONS]--> PRODUCT(5310)
  MEMORY(5310用于PACS) --[LEARN_FROM]--> CHAT(当前对话)
        ↓
LLM 回复: "记住了：5310主要用于PACS。以后有人问起我会告诉他们。"
```

### 回复风格

拟人化、口语化：

- "记住了：5310主要用于PACS。"
- "好的，以后报价不低于8折，我记下了。"
- "明白，老王是天门项目的负责人，记住了。"

---

## 四、PERSON 识别流程

### 触发时机

对话中首次提到一个不在已有 Person 列表中的人名。

### 创建逻辑

```
用户: "老王说下周来看样机"
        ↓
LLM 判断: "老王"不在已有 Person 中
        ↓
创建 source_data:
  type = PERSON
  title = "老王"
  content = "从对话中识别的人物"
        ↓
建立 Object Link:
  PERSON(老王) --[MENTIONS]--> PROJECT(天门项目)  (如果上下文有)
        ↓
LLM 回复: "好的，老王下周来看样机，我记住了。"
```

---

## 五、POLICY 创建流程

### 触发时机

用户明确表达规则或偏好时。

### 创建逻辑

```
用户: "以后报价不要低于8折"
        ↓
LLM 判断: is_knowledge = true, type = POLICY
        ↓
创建 source_data:
  type = POLICY
  title = "报价不低于8折"
  content = "以后报价不要低于8折"
        ↓
建立 Object Link:
  POLICY(报价不低于8折) --[TEACHES]--> USER
        ↓
LLM 回复: "记住了：以后报价不低于8折。"
```

---

## 六、检索增强

### MEMORY 参与检索

MEMORY 对象和 PRODUCT/PROJECT 走完全相同的检索路径：

```rust
// search 时，MEMORY 自然参与 FTS5 + Vector 检索
// 不需要任何特殊处理
// 因为 source_data 已经统一
```

### MEMORY 参与 Object Link 扩散

```rust
// synapse.rs: expand_object_links 已支持
// 5310 → MEMORY(5310用于PACS) → PRODUCT(5310)
// 注意力衰减同样适用于 MEMORY
```

### 溯源查询

```
用户: "为什么说5310用于PACS？"
        ↓
LLM 查询 Object Link:
  5310 → [MENTIONS] → MEMORY(5310用于PACS) → [LEARN_FROM] → CHAT(对话)
        ↓
LLM 回复: "这是你之前告诉我的：5310主要用于PACS系统。"
```

---

## 七、实现步骤

### Step 1: 数据库迁移

**文件**: `database.rs`

```sql
-- 1. object_links 新增 edge_type 列
ALTER TABLE object_links ADD COLUMN edge_type TEXT DEFAULT 'CORRELATE';

-- 2. 更新唯一约束（兼容旧数据）
-- object_links 唯一约束变为 (source_id, target_id, edge_type)
```

### Step 2: MEMORY/PERSON/POLICY 创建函数

**文件**: `agent_workflow.rs` (新增约 80 行)

```rust
/// 创建 MEMORY 对象
fn create_memory(
    title: &str, 
    content: &str, 
    source_chat_id: &str,
    mentions: &[String],
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    
    // 1. 写入 source_data
    database::run(
        "INSERT INTO source_data(source_type,source_id,title,content,created_at,updated_at) 
         VALUES('MEMORY',?1,?2,?3,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(&id), json!(title), json!(content)],
    ).map_err(|e| format!("insert memory: {}", e))?;
    
    // 2. 建立 TEACHES Link (→ USER)
    database::run(
        "INSERT INTO object_links(source_id,target_id,edge_type,weight,last_seen) 
         VALUES(?1,'USER-1','TEACHES',5.0,?2)",
        &[json!(format!("MEMORY-{}", id)), json!(now_secs() as i64)],
    ).ok();
    
    // 3. 建立 LEARN_FROM Link (→ CHAT)
    if !source_chat_id.is_empty() {
        database::run(
            "INSERT INTO object_links(source_id,target_id,edge_type,weight,last_seen) 
             VALUES(?1,?2,'LEARN_FROM',5.0,?3)",
            &[json!(format!("MEMORY-{}", id)), json!(format!("CHAT-{}", source_chat_id)), json!(now_secs() as i64)],
        ).ok();
    }
    
    // 4. 建立 MENTIONS Link (→ 提到的对象)
    for mention in mentions {
        database::run(
            "INSERT INTO object_links(source_id,target_id,edge_type,weight,last_seen) 
             VALUES(?1,?2,'MENTIONS',5.0,?3)",
            &[json!(format!("MEMORY-{}", id)), json!(mention), json!(now_secs() as i64)],
        ).ok();
    }
    
    Ok(id)
}

/// 创建 PERSON 对象
fn create_person(name: &str) -> Result<String, String> {
    // 检查是否已存在
    if let Ok(Some(_)) = database::get(
        "SELECT id FROM source_data WHERE source_type='PERSON' AND title=?1",
        &[json!(name)],
    ) {
        return Ok("exists".to_string());
    }
    
    let id = uuid::Uuid::new_v4().to_string();
    database::run(
        "INSERT INTO source_data(source_type,source_id,title,content,created_at,updated_at) 
         VALUES('PERSON',?1,?2,?3,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(&id), json!(name), json!(format!("从对话中识别的人物: {}", name))],
    ).map_err(|e| format!("insert person: {}", e))?;
    
    Ok(id)
}

/// 创建 POLICY 对象
fn create_policy(title: &str, content: &str) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    database::run(
        "INSERT INTO source_data(source_type,source_id,title,content,created_at,updated_at) 
         VALUES('POLICY',?1,?2,?3,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(&id), json!(title), json!(content)],
    ).map_err(|e| format!("insert policy: {}", e))?;
    
    // TEACHES Link
    database::run(
        "INSERT INTO object_links(source_id,target_id,edge_type,weight,last_seen) 
         VALUES(?1,'USER-1','TEACHES',5.0,?2)",
        &[json!(format!("POLICY-{}", id)), json!(now_secs() as i64)],
    ).ok();
    
    Ok(id)
}
```

### Step 3: LLM 知识识别

**文件**: `agent_workflow.rs` (修改 System Prompt + 回复逻辑)

在 `AGENT_SYSTEM_PROMPT` 中新增：

```
## 知识记忆

当用户明确陈述事实、规则或偏好时，你有权直接记忆。

判断标准：
- 用户主动告诉你的信息（"5310主要用于PACS"）
- 明确的规则或偏好（"报价不要低于8折"）
- 人物关系（"老王是天门项目的负责人"）

记忆时：
1. 在回答末尾添加 <memory> 标签
2. 回复拟人化，告诉用户你记住了
3. 不要问"是否确认"——用户说的就是知识

<memory>
{"type":"MEMORY","title":"5310主要用于PACS","content":"5310主要用于PACS系统","mentions":["PRODUCT-5310"]}
</memory>

不要记忆：
- 你自己的推断（"看起来..."）
- 临时性信息（"今天天气不错"）
- 对话中的闲聊（"你好""谢谢"）
```

### Step 4: Memory 解析与创建

**文件**: `agent_workflow.rs` (在最终回复后解析)

```rust
/// 从 LLM 回复中提取 <memory> 标签并创建对象
fn extract_and_create_memories(response: &str, chat_id: &str) -> Vec<String> {
    let mut created = Vec::new();
    let mut remaining = response;
    
    while let Some(start) = remaining.find("<memory>") {
        let end = remaining[start..].find("</memory>").map(|e| start + e + 8);
        let Some(end) = end else { break };
        
        let json_str = &remaining[start+8..end];
        remaining = &remaining[end..];
        
        if let Ok(mem) = serde_json::from_str::<Value>(json_str) {
            let mem_type = mem["type"].as_str().unwrap_or("MEMORY");
            let title = mem["title"].as_str().unwrap_or("");
            let content = mem["content"].as_str().unwrap_or("");
            let mentions: Vec<String> = mem["mentions"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            
            let result = match mem_type {
                "MEMORY" => create_memory(title, content, chat_id, &mentions),
                "PERSON" => create_person(title),
                "POLICY" => create_policy(title, content),
                _ => continue,
            };
            
            if let Ok(id) = result {
                created.push(id);
            }
        }
    }
    
    created
}
```

### Step 5: 修改最终回复逻辑

在 `agent_chat_workflow` 的最终回答处：

```rust
// 原有逻辑：直接返回 answer
// 新增逻辑：解析 answer 中的 <memory> 标签，创建对象，然后从 answer 中移除标签

let memories = extract_and_create_memories(&answer, &chat_id);
let clean_answer = remove_memory_tags(&answer);

// 如果创建了 MEMORY，追加拟人化提示
let final_answer = if !memories.is_empty() {
    format!("{}\n\n（已记住 {} 条新知识）", clean_answer, memories.len())
} else {
    clean_answer
};

return Ok(json!({"success":true,"answer":final_answer,...}));
```

---

## 八、文件改动清单

| 文件 | 改动类型 | 预估行数 |
|------|---------|---------|
| `database.rs` | 迁移：object_links 新增 edge_type | ~10 行 |
| `agent_workflow.rs` | 新增 create_memory/create_person/create_policy | ~80 行 |
| `agent_workflow.rs` | 新增 extract_and_create_memories | ~40 行 |
| `agent_workflow.rs` | 修改 System Prompt（知识记忆指令） | ~20 行 |
| `agent_workflow.rs` | 修改最终回复逻辑（解析 memory 标签） | ~15 行 |
| `synapse.rs` | expand_object_links 支持 edge_type 过滤 | ~15 行 |
| **总计** | | **~180 行** |

---

## 九、验证方案

### 单元测试

1. `create_memory()` — 创建 MEMORY + 建立 Links
2. `create_person()` — 创建 PERSON（去重）
3. `create_policy()` — 创建 POLICY + 建立 Links
4. `extract_and_create_memories()` — 解析 <memory> 标签

### 集成测试

1. 用户说"5310主要用于PACS" → 生成 MEMORY → 回复"记住了"
2. 用户说"老王下周来看样机" → 生成 PERSON → 回复"记住了"
3. 用户说"报价不低于8折" → 生成 POLICY → 回复"记住了"
4. 用户说"你好" → 不生成任何 Memory
5. 用户说"看起来5310不错" → 不生成（系统推断）

### 端到端测试

1. 记忆后检索："5310用于什么" → 命中 MEMORY
2. 溯源："为什么说5310用于PACS" → 回答"你之前告诉我的"
3. 修正："5310其实也用于HIS" → 新增 MEMORY + CORRECTS Link

---

## 十、与 Phase 1 (Planner) 的关系

Memory Layer 和 Planner 是两个独立但互补的能力：

```
Phase 1 (Planner): 认知流水线
  User → Planner → TaskPlan → Workers → Merger → LLM

Memory Layer: 知识记忆
  User → LLM → 识别知识 → 创建 MEMORY → 回复"记住了"
```

可以独立实施，也可以组合：
- Planner 从 MEMORY 中检索已有知识
- Memory Layer 在 Planner 执行过程中学习新知识

建议：**先实现 Memory Layer（本方案），再实现 Planner（Phase 1）。** 因为 Memory Layer 更小、更独立、价值更直接。
