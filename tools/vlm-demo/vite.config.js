import { defineConfig } from 'vite'

// ── 通用反向代理：/p/<host>/<path> → https://<host>/<path> ──
// 让 demo 可以直连任意 OpenAI 兼容服务（SiliconFlow / 阿里百炼 / 本地 vLLM…），绕过浏览器 CORS
const HOST_RE = /^[a-z0-9.-]+(:\d+)?$/i
const DROP_REQ_HEADERS = ['host', 'connection', 'content-length', 'accept-encoding', 'origin', 'referer']
const DROP_RES_HEADERS = ['content-encoding', 'content-length', 'transfer-encoding', 'connection']

function genericProxyMiddleware() {
  return (req, res, next) => {
    const url = req.url || ''
    if (!url.startsWith('/p/')) return next()
    const rest = url.slice(3) // '/<host>/<path>'
    const slash = rest.indexOf('/')
    const host = slash === -1 ? rest : rest.slice(0, slash)
    const path = slash === -1 ? '/' : rest.slice(slash)
    if (!HOST_RE.test(host)) {
      res.statusCode = 400
      return res.end('bad proxy host')
    }
    const chunks = []
    req.on('data', (c) => chunks.push(c))
    req.on('end', async () => {
      try {
        const method = req.method || 'GET'
        const hasBody = method !== 'GET' && method !== 'HEAD'
        const headers = { ...req.headers }
        for (const h of DROP_REQ_HEADERS) delete headers[h]
        const upstream = await fetch('https://' + host + path, {
          method,
          headers,
          body: hasBody ? Buffer.concat(chunks) : undefined,
          signal: AbortSignal.timeout(300000)
        })
        const out = {}
        upstream.headers.forEach((v, k) => {
          if (!DROP_RES_HEADERS.includes(k.toLowerCase())) out[k] = v
        })
        const buf = Buffer.from(await upstream.arrayBuffer())
        res.writeHead(upstream.status, out)
        res.end(buf)
      } catch (e) {
        if (!res.headersSent) {
          res.writeHead(502, { 'content-type': 'text/plain; charset=utf-8' })
        }
        res.end('proxy error: ' + (e && e.message))
      }
    })
  }
}

export default defineConfig({
  plugins: [
    {
      name: 'generic-cors-proxy',
      configureServer(server) {
        // 注册在 vite 内部中间件之前
        server.middlewares.use(genericProxyMiddleware())
      }
    }
  ],
  server: {
    proxy: {
      // 旧的 SiliconFlow 专用代理（兼容已有 localStorage 配置）
      '/sf': {
        target: 'https://api.siliconflow.cn',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/sf/, '')
      }
    }
  }
})
