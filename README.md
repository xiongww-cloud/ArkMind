# ArkMind 方舟智库

> AI-powered local-first desktop knowledge management system
> AI 驱动的本地优先桌面知识管理系统

ArkMind 是一款面向项目型业务团队（系统集成商、IT 服务商）的桌面应用，提供从项目管理、产品目录、方案报价到知识沉淀的全流程数字化工具，内置 RAG 知识库和 Agent 智能助手。

## Badges

![Tauri v2](https://img.shields.io/badge/Tauri-v2-blue)
![Vue 3](https://img.shields.io/badge/Vue-3-green)
![Rust](https://img.shields.io/badge/Rust-1.77%2B-orange)
![TypeScript](https://img.shields.io/badge/TypeScript-5.x-blue)

## Screenshots

<!-- screenshots -->

## Features

- **Dashboard 仪表盘** — 项目概览、数据统计、快捷入口
- **Projects 项目管理** — 8 阶段状态管道（Lead → Won/Lost），动态时间线，批量导入导出
- **Products 产品目录** — 层级系列树，物料文件关联，拖拽排序
- **Notes 笔记系统** — TipTap 富文本编辑，@提及项目/文件，笔记本层级，回收站
- **Todos 待办管理** — 优先级、截止日期、重复计划、项目关联
- **Contacts 通讯录** — 渠道商/供应商/客户统一管理，联系人多对多
- **Assets 资料库** — 统一文件上传，文件夹树，FTS5 全文搜索
- **Knowledge Base 知识库** — AI RAG 问答，双层知识块，神经能量衰减模型，混合检索（FTS5 + 向量）
- **AI Agent 智能助手** — ReAct 循环，9 个原生工具，流式输出，用户策略/"宪法"
- **Solution 报价** — BOM 物料清单配置器，Excel 导入导出，服务费分摊，版本管理
- **Settings 设置** — 系统配置、AI 模型管理、许可证管理

## Tech Stack

| Layer 层 | Technology 技术 |
|---|---|
| Desktop Framework 桌面框架 | Tauri v2 (Rust backend) |
| Frontend 前端 | Vue 3 + TypeScript + Tailwind CSS + TipTap |
| Database 数据库 | SQLite (WAL + FTS5) + LanceDB (vector) |
| AI | OpenAI-compatible API, multi-provider support |
| Build 构建 | Vite 7 + Cargo |

## Prerequisites

- **Rust** >= 1.77 ([rustup.rs](https://rustup.rs/))
- **Node.js** >= 18
- **pnpm** or npm
- **WebView2 Runtime** (usually pre-installed on Windows 10/11)

## Installation

```bash
# Frontend dependencies
npm install

# Rust dependencies (auto-downloaded on first build)
cargo check --manifest-path src-tauri/Cargo.toml
```

## Development

```bash
# Start frontend hot-reload + Rust backend simultaneously
npm run tauri dev
```

## Build

```bash
# Build NSIS installer (Windows)
npx tauri build --bundles nsis

# Build for all platforms
npx tauri build
```

Build artifacts are located in `src-tauri/target/release/bundle/`.

## Project Structure

```
ArkMind/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── lib.rs          # App entry, command registration
│   │   ├── database.rs     # SQLite schema + migrations + FTS5
│   │   ├── crypto.rs       # AES-256-GCM encryption
│   │   ├── license.rs      # Ed25519 license verification
│   │   ├── commands/       # 14 Tauri command modules (248 commands)
│   │   └── services/       # 24 service modules (AI pipeline, file parsing, etc.)
│   └── Cargo.toml
├── src/renderer/src/       # Vue.js frontend
│   ├── views/              # 15 pages
│   ├── components/         # 34 components
│   ├── stores/             # 6 Pinia stores
│   ├── composables/        # 9 composables
│   └── api/db-client.ts    # Tauri IPC wrapper
├── docs/                   # Project documentation
└── package.json
```

## Architecture Overview

ArkMind 采用 5 层 AI 流水线架构：

```
Business CRUD → Push Pipeline → Data Ingestion → Knowledge Pipeline → Retrieval
                                  (Sync/Embed/Tag)
                                  ↓
                          FTS5 BM25 + Vector Similarity → RRF Fusion → Agent Answer
```

1. **数据采集层** — 业务 CRUD 操作触发数据推送
2. **同步层** — 增量同步至知识库
3. **嵌入层** — 文本向量化，存储至 LanceDB
4. **检索层** — FTS5 全文检索 + 向量相似度混合检索
5. **生成层** — RRF 融合排序，Agent 回答生成

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

Please follow existing code conventions and run `npm run lint` before submitting.

## License

Commercial software. Requires license key activation. Advanced features (AI knowledge base, vector retrieval) require a premium license.

商业软件，需要许可证密钥激活。高级功能（AI 知识库、向量检索）需要高级许可证。

## Recommended IDE

- [VSCode](https://code.visualstudio.com/) + [Volar](https://marketplace.visualstudio.com/items?itemName=Vue.volar) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
