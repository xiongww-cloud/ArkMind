# ArkMind — Agent 工作守则

## 打包/发布红线（最高优先级）
- **未经用户明确说「打包」或「发布」，禁止产出任何安装包**：不 `git push` tag（`.github/workflows/release.yml` 仅由 tag push 触发），不跑 `npm run tauri build` / `node scripts/tauri-build.mjs build` 产 bundle，不外传 draft release 资产。
- `git push github master` 本身不触发打包，但也**先问再推**。
- 日常验证只用轻量门禁：`cargo test`、`npm run typecheck`、`npm run build`（vite only，不出包）。

## 环境备忘
- Rust 编译/测试前需要：`export PDFIUM_STATIC_LIB_PATH=$PWD/src-tauri/bin`（macOS 本机路径；CI 由 scripts/tauri-build.mjs 注入）。
- `npm run typecheck` 存在**历史错误基线**（数字随重构变动，以最近一次提交时的值为准）：判断"是否引入新错"要对比基线，不是看非零。
- 版本号三处必须一致：`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`（`__APP_VERSION__` 取自 Cargo.toml，漏改会导致应用内更新弹窗静默失效）；`scripts/tauri-build.mjs` 会在构建时硬校验，改动版本必须三处同改。
- 应用内有**两份**版本说明源：`WhatsNewModal.vue`（弹窗 changelog）与 `GuideModal.vue` 的「版本更新日志」章节；另有仓库根 `release-notes.md`（GitHub Release body 的 CI 取源）。发版时三处都要同步。
- GitHub remote 名为 `github`（origin 指向旧本机路径）；tag 修复性重打用 `git tag -f` + force push，CI 的 prepare job 会复用现有 Draft（edit-first）。
- CI（release.yml）自 v14.4.4 起只产 Windows NSIS（2x 计费倍率）；macOS 包由维护者本地跑 `scripts/mac-dmg.sh` 产出（app → ad-hoc 签名自验 → dmg）。
- macOS 产物为 ad-hoc 本地签名（codesign --sign -）+ 未公证：「应用已损坏」已根治，但首开仍需 右键打开(≤14)/隐私设置放行(15+)。

## 改动纪律
- 未经用户明确要求，不 commit、不 push、不开 PR。
- 每个功能批次跑门禁后再 commit：`cargo test`（全量）+ `npm run typecheck` + `npm run build`。
- `tools/topo-demo` 只作为本地 demo：其工作区改动**永不提交**（分类收编时勿 stage 该目录；历史已入库部分保留不动）。
