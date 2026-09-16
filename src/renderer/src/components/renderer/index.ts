import { defineAsyncComponent } from 'vue'

// 动态导入组件
export const ProjectCard = defineAsyncComponent(() => import('./ProjectCard.vue'))
export const TodoCard = defineAsyncComponent(() => import('./TodoCard.vue'))
export const ObjectList = defineAsyncComponent(() => import('./ObjectList.vue'))

// Renderer Registry
export const rendererRegistry: Record<string, any> = {
  project: ProjectCard,
  todo: TodoCard,
  object_list: ObjectList,
}

// 获取组件
export function getRenderer(type: string) {
  return rendererRegistry[type] || null
}

// HTML 转义：refs 数据源自 LLM 输出（可被外部内容注入），拼入 v-html 前必须净化
function esc(s: unknown): string {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

// 渲染对象引用
export function renderObjectReference(data: any): string {
  if (!data) return ''

  const { objects, items } = data

  // 如果是对象引用数组
  if (Array.isArray(objects)) {
    return objects.map(obj => {
      const renderer = getRenderer(String(obj.type || '').toLowerCase())
      if (renderer) {
        // 组件会通过 Vue 渲染
        return `<div data-component="ObjectCard" data-type="${esc(obj.type)}" data-id="${esc(obj.id)}" data-name="${esc(obj.name)}"></div>`
      }
      return `<div class="object-ref">📌 ${esc(obj.name || obj.id)}</div>`
    }).join('')
  }

  // 如果是对象列表
  if (Array.isArray(items)) {
    return `<div data-component="ObjectList" data-items="${esc(JSON.stringify(items))}"></div>`
  }

  return ''
}
