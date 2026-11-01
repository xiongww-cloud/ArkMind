import router from '@/router'
import { previewFile } from '@/api/db-client'
import { invoke } from '@tauri-apps/api/core'
import { toast } from '@/composables/useToast'

/**
 * 软件内链接点击入口：
 * - markdown 内链 `#/...`（hash 路由，能活过 DOMPurify）→ 主窗直接 router 跳转；
 *   浮窗无 router → 调 chat_navigate_main 唤起主窗并由主窗执行跳转
 * 引用展示不另做 chips——回答正文里的链接本身即可点击核实。
 */

/** 当前是否浮窗（独立 chat webview，未安装 vue-router） */
function isFloatWindow(): boolean {
  try {
    return window.location.pathname.includes('chat.html')
  } catch {
    return false
  }
}

/** EntityRef 形态归一：`PRODUCT-<uuid>` / `EQUIPMENT_LIST-<uuid>` → 裸 uuid。
 *  AI 生成的深链常带类型前缀，而 focus/详情路由只认裸 id；仅当前段为大写类型且后段是 uuid 时剥前缀。 */
export function stripEntityRef(v: string): string {
  if (/^[A-Z][A-Z_0-9]*-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-/.test(v)) {
    return v.slice(v.indexOf('-') + 1)
  }
  return v
}

/** 执行主窗路由跳转（hash 内链 + 浮窗桥共用） */
export async function pushInAppPath(path: string): Promise<void> {
  let to = path.startsWith('/') ? path : `/${path}`
  // focus 参数与深链 id 兼容 EntityRef 形态（PRODUCT-uuid → uuid）
  to = to.replace(/([?&]focus=)([^&#]+)/g, (_m, k: string, v: string) => k + stripEntityRef(v))
  to = to.replace(/^(\/(?:projects|files)\/)([^/?#]+)/, (_m, p: string, id: string) => p + stripEntityRef(id))
  // 文件没有独立路由：#/files/<id> 约定直接走系统打开
  const m = to.match(/^\/files\/([^/?#]+)/)
  if (m) {
    try {
      const r = await previewFile(decodeURIComponent(m[1]))
      if (!r?.success) toast({ message: r?.error || '打开文件失败', type: 'error' })
    } catch (e: any) {
      toast({ message: '打开文件失败：' + (e?.message || e), type: 'error' })
    }
    return
  }
  // 同 URL 重复点击：vue-router 对相同 location 去重（NAVIGATION_DUPLICATED），
  // route.query 引用不变 → 各页 watch(route.query) 不触发、深链"没反应"。
  // 先 replace 清掉 focus 再 push 重入，制造 query 变化让消费逻辑重跑。
  if (to.includes('focus=') && router.currentRoute.value.fullPath === to) {
    let bare = to.replace(/([?&])focus=[^&#]*/g, '').replace(/\?$/, '')
    if (!bare.includes('?') && bare.includes('&')) bare = bare.replace('&', '?')
    await router.replace(bare).catch(() => {})
  }
  await router.push(to)
}

/** markdown 内链点击（a[href^="#/"]）：主窗直接跳，浮窗桥接到主窗 */
export async function dispatchInAppLink(href: string): Promise<void> {
  const path = href.replace(/^#/, '')
  if (isFloatWindow()) {
    try {
      await invoke('chat_navigate_main', { path })
    } catch (e: any) {
      toast({ message: '跳转失败：' + (e?.message || e), type: 'error' })
    }
    return
  }
  await pushInAppPath(path)
}
