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

// 渲染对象引用
export function renderObjectReference(data: any): string {
  if (!data) return ''

  const { objects, items } = data

  // 如果是对象引用数组
  if (Array.isArray(objects)) {
    return objects.map(obj => {
      const renderer = getRenderer(obj.type?.toLowerCase())
      if (renderer) {
        // 组件会通过 Vue 渲染
        return `<div data-component="ObjectCard" data-type="${obj.type}" data-id="${obj.id}" data-name="${obj.name}"></div>`
      }
      return `<div class="object-ref">📌 ${obj.name || obj.id}</div>`
    }).join('')
  }

  // 如果是对象列表
  if (Array.isArray(items)) {
    return `<div data-component="ObjectList" data-items='${JSON.stringify(items)}'></div>`
  }

  return ''
}
