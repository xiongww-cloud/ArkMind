import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'
import { readFileSync } from 'fs'

const cargoToml = readFileSync(resolve(__dirname, 'src-tauri/Cargo.toml'), 'utf-8')
const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)
const appVersion = versionMatch ? versionMatch[1] : '0.0.0'

export default defineConfig({
  root: 'src/renderer',
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
  },
  plugins: [vue()],
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src/renderer/src'),
      '@renderer': resolve(__dirname, 'src/renderer/src'),
      // xlsx 的 .mjs 是混合模块(export + exports. + require)，Rollup 无法静态分析。
      // 用 alias 指向 UMD 版本，让 Vite 的 commonjs 插件能正确处理。
      'xlsx': resolve(__dirname, 'node_modules/xlsx/dist/xlsx.full.min.js'),
    },
  },
  optimizeDeps: {
    include: ['xlsx'],
  },
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: resolve(__dirname, 'dist'),
    emptyOutDir: true,
    target: 'esnext',
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'src/renderer/index.html'),
        sprite: resolve(__dirname, 'src/renderer/sprite.html'),
        chat: resolve(__dirname, 'src/renderer/chat.html'),
      },
    },
  },
})
