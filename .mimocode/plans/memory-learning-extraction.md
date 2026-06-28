# Memory Learning Extraction — 从经历中提取知识

## 背景

当前 `record_experience` 只记录认知过程（查阅了什么、用了什么工具），但不记录"学到了什么"。最有价值的部分——LLM 从资料中提取的关键事实——没有被捕获。

## 核心问题

LLM 回答问题时的思考过程：
```
用户问 → 搜索 → 阅读资料 → 提取关键信息 → 组织回答
```

关键信息的提取发生在 LLM 的推理过程中，而不是工具调用结果里。工具返回的是完整内容，但 LLM 只"学到"了其中一部分。

**问题**：如何知道 LLM 从资料中"学到"了什么？

## 方案对比

| 方案 | 原理 | 优点 | 缺点 |
|------|------|------|------|
| A: LLM 输出 `<learnings>` | LLM 回答时额外输出学到的事实 | 精确捕获 LLM 实际学到的内容 | 依赖 LLM 输出格式，增加 token |
| B: 从工具结果提取 | 遍历工具结果，提取关键信息片段 | 不需要 LLM 额外输出 | 提取"访问了什么"而非"学到了什么" |
| C: 混合模式 | 后端提取基础信息 + LLM 补充精确信息 | 两者结合 | 实现复杂度较高 |

## 推荐方案：条件触发 + LLM 输出

### 触发条件

**不是每次都要求 LLM 输出 learnings**，而是当 LLM 访问了 MEMORY 或 FILE 类型的来源时，才要求它总结学到的事实。

简单对话（问候、闲聊）→ 无额外开销
阅读资料的对话 → 产生学习记录

### 实现方式

#### 1. Prompt 修改

在 `AGENT_SYSTEM_PROMPT` 中新增：

```
## 知识提取

当你通过工具访问了 MEMORY 或 FILE 类型的来源并基于这些资料回答了问题时，
在回答末尾用 <learnings> 标签列出你从这些来源中学到的关键事实。

格式：
<learnings>
- 你学到的事实1
- 你学到的事实2
</learnings>

规则：
- 只记录你从资料中确认的事实，不要记录你的推断
- 每条 learning 应该是一个完整的陈述
- 如果没有从资料中学到新东西，不要输出 <learnings>
- 例如：用户问5310用途，你查阅了产品手册后回答。
  learning 应该是"5310支持SAN/NAS双协议"，而不是"回答了用户的问题"
```

#### 2. 解析与存储

在 `record_experience` 之后，新增 `extract_learnings` 逻辑：

```rust
fn extract_learnings(answer: &str, sources: &[String]) -> Vec<String> {
    // 从 <learnings> 标签中提取
    if let Some(start) = answer.find("<learnings>") {
        if let Some(end) = answer[start..].find("</learnings>") {
            let body = &answer[start+11..start+end];
            return body.lines()
                .filter_map(|line| {
                    let line = line.trim().trim_start_matches('-').trim();
                    if line.is_empty() { return None; }
                    Some(line.to_string())
                })
                .collect();
        }
    }
    vec![]
}
```

#### 3. 创建 Fact MEMORY

每条 learning 创建一条 MEMORY：

```
source_type = 'MEMORY'
source_id = uuid
title = learning 的前 50 字
content = "[学到] {learning}\n[来源] {sources}\n[时间] {created_at}"
created_at = 自动
```

#### 4. 集成位置

在 `record_experience` 之后：

```rust
// 记录认知经历（已实现）
record_experience(&query, &session, &tool_log);

// 提取学到的知识
let learnings = extract_learnings(&answer, &sources);
for learning in learnings {
    create_learning_memory(&learning, &sources);
}
```

## 数据模型

### MEMORY (type=experience) — 认知过程
```
[用户问] 5310适用于什么场景？
[查阅] MEMORY-xxx、FILE-xxx
[工具] search(["5310"])
[结论] 完成回答
```

### MEMORY (type=fact) — 学到的知识
```
[学到] 5310 支持 SAN 和 NAS 双协议
[来源] FILE-xxx (5310产品手册)
[时间] 2026-06-29
```

## 检索增强

两种 MEMORY 都参与 FTS5 + 向量检索：
- 用户问"5310 用于什么" → 命中 fact MEMORY（学到的知识）
- 用户问"我之前讨论过什么" → 命中 experience MEMORY（认知过程）

## 改动量

| 文件 | 改动 | 行数 |
|------|------|------|
| `agent_workflow.rs` | Prompt 新增 `<learnings>` 指令 | ~10 行 |
| `agent_workflow.rs` | `extract_learnings` 函数 | ~15 行 |
| `agent_workflow.rs` | `create_learning_memory` 函数 | ~15 行 |
| `agent_workflow.rs` | 在 return 路径中调用 | ~5 行 |
| **总计** | | **~45 行** |

## 依赖

- 需要先实现 `record_experience`（认知经历记录）
- 需要 LLM 支持 `<learnings>` 标签输出（部分模型可能不遵循）
- 需要验证不同模型（GPT-4、Qwen、DeepSeek）的输出稳定性

## 风险

1. LLM 可能不输出 `<learnings>` → 降级为只记录 experience，不影响功能
2. LLM 可能输出错误的 learning → 但 MEMORY 是"用户说"格式，不会被当作事实
3. Token 消耗增加 → 仅在访问 MEMORY/FILE 来源时触发，控制开销
