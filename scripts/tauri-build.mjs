/**
 * 跨平台 Tauri 构建脚本
 *
 * 为什么需要这个脚本？
 * ─────────────────────────────────────────────────────────────
 * pdfium-render 是一个 Rust 依赖库，它的 build.rs 需要
 * PDFIUM_STATIC_LIB_PATH 环境变量来定位 pdfium 静态库。
 *
 * 问题1：路径跨平台
 *   - Windows: D:\Projects\ArkMind\src-tauri\bin
 *   - macOS:   /Users/x/Projects/ArkMind/src-tauri/bin
 *   .cargo/config.toml 无法同时适配两个平台（[env] 不支持条件判断）
 *
 * 问题2：构建缓存（导致"每次更新后都报同样的错"）
 *   cargo 会缓存 build.rs 的输出。如果 pdfium-render 曾在
 *   没有设置 PDFIUM_STATIC_LIB_PATH 的情况下编译过
 *   （例如直接运行 cargo build），缓存的 build.rs 输出
 *   不包含链接指令，后续即使设置了环境变量也不会重新执行。
 *   解决方案：编译前先 cargo clean -p pdfium-render 清除缓存。
 *
 * 用法：
 *   node scripts/tauri-build.mjs build    # 打包编译
 *   node scripts/tauri-build.mjs dev      # 开发模式
 * ─────────────────────────────────────────────────────────────
 */

import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const srcTauri = path.join(repoRoot, 'src-tauri')
const pdfiumBin = path.join(srcTauri, 'bin')

// ── 1. 清除 pdfium-render 的 build.rs 缓存 ──────────────────
// 防止旧的（缺少链接指令的）缓存导致 "could not find native static library pdfium" 错误
const cleanResult = spawnSync('cargo', ['clean', '-p', 'pdfium-render'], {
  cwd: srcTauri,
  stdio: 'inherit',
  shell: true,
})
if (cleanResult.status !== 0) {
  console.warn('[tauri-build] cargo clean -p pdfium-render failed, continuing anyway...')
}

// ── 2. 设置 PDFIUM_STATIC_LIB_PATH 并运行 tauri ─────────────
const tauriCli = path.join(repoRoot, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const args = process.argv.slice(2) // e.g. ['build'] or ['dev']

const result = spawnSync(process.execPath, [tauriCli, ...args], {
  stdio: 'inherit',
  env: {
    ...process.env,
    PDFIUM_STATIC_LIB_PATH: pdfiumBin,
  },
})

process.exit(result.status ?? 1)
