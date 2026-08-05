<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { fetchTodos, createTodo, toggleTodo, deleteTodo, updateTodo } from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import TodoDetail from '@/views/TodoDetail.vue'
import ProjectFilePanel from '@/components/project/ProjectFilePanel.vue'

const props = defineProps<{ projectId: string; projectName: string; projectBackendId?: string }>()
const emit = defineEmits<{ update: [] }>()

const projectStore = useProjectStore()

const projectTodos = ref<any[]>([])
const selectedTodoId = ref<string | null>(null)
const selectedTodo = computed(() => projectTodos.value.find(t => t.id === selectedTodoId.value))
const todoDetailRef = ref<any>(null)
const projectTodoDoneOpen = ref(false)
const projectUndoneTodos = computed(() => projectTodos.value.filter(t => !t.done))
const projectDoneTodos = computed(() => projectTodos.value.filter(t => t.done))

async function loadProjectTodos() {
  try {
    const r = await fetchTodos()
    const all = Array.isArray(r) ? r : (r?.todos || [])
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    projectTodos.value = all.filter((t: any) => t.project_id === backendId).map((t: any) => ({
      id: t.id, title: t.title || '', content: t.content || '',
      done: t.done === 1 || t.done === true,
      createdAt: t.created_at || new Date().toISOString(),
      dueDate: t.due_date || undefined,
      priority: t.priority || undefined,
      projectId: t.project_id || undefined,
      projectName: t.project_name || '',
    }))
  } catch { projectTodos.value = [] }
}

async function addProjectTodo() {
  try {
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    const r = await createTodo({ title: '新待办', projectId: backendId })
    if (r?.todo) {
      const t = r.todo
      projectTodos.value.unshift({
        id: t.id, title: t.title || '新待办', content: t.content || '',
        done: false, createdAt: t.created_at || new Date().toISOString(),
        dueDate: t.due_date || undefined, priority: t.priority || undefined,
        projectId: t.project_id || undefined, projectName: t.project_name || '',
      })
      selectedTodoId.value = t.id
    } else {
      toast({ message: '创建待办失败', type: 'error' })
    }
  } catch (e) {
    console.warn('创建待办失败', e)
    toast({ message: '创建待办失败', type: 'error' })
  }
}

async function toggleProjectTodo(todoId: string) {
  try {
    await toggleTodo(todoId)
    await loadProjectTodos()
  } catch (e) { console.warn('切换待办状态失败', e) }
}

async function removeProjectTodo(todoId: string) {
  const todo = projectTodos.value.find((t: any) => t.id === todoId)
  const todoTitle = todo?.title || '此待办'
  const ok = await dangerConfirm({ title: '删除待办', message: `确定要删除待办「${todoTitle}」吗？此操作不可撤销。`, confirmText: '删除' })
  if (!ok) return
  try {
    await deleteTodo(todoId)
    selectedTodoId.value = null
    await loadProjectTodos()
  } catch (e) { console.warn('删除待办失败', e) }
}

async function handleTodoUpdate(todoId: string, data: Record<string, any>) {
  try {
    await updateTodo(todoId, data)
    const idx = projectTodos.value.findIndex(t => t.id === todoId)
    if (idx !== -1) {
      Object.assign(projectTodos.value[idx], data)
    }
  } catch (e) { console.warn('更新待办失败', e) }
}

const filePanelRef = ref<InstanceType<typeof ProjectFilePanel> | null>(null)
watch(selectedTodo, () => {
  if (selectedTodo.value) filePanelRef.value?.loadFiles()
})

watch(() => props.projectId, () => { loadProjectTodos() }, { immediate: true })

defineExpose({ loadProjectTodos })
</script>

<template>
  <div class="h-full flex overflow-hidden bg-white">
    <div class="w-[280px] flex-shrink-0 border-r border-surface-200 flex flex-col bg-white/50">
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">待办</span>
        <div class="flex items-center gap-2">
          <span class="text-[10px] text-surface-400 font-medium">{{ projectUndoneTodos.length }}</span>
          <button @click="addProjectTodo" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建待办"><AppIcon name="plus" size="12" stroke="2.5"/></button>
        </div>
      </div>
      <div style="display:flex;flex-direction:column;flex:1;min-height:0">
        <div class="flex-1 overflow-y-auto px-2 py-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
          <div v-for="todo in projectUndoneTodos" :key="todo.id" @click="selectedTodoId = todo.id"
            class="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden select-none mb-1"
            :class="['border-surface-100 hover:border-surface-300 hover:shadow-md', selectedTodoId === todo.id ? 'bg-blue-50/70 border-brand-200 shadow-none scale-100' : 'hover:scale-[1.02] hover:z-10']">
            <div v-if="selectedTodoId === todo.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
            <button @click.stop="toggleProjectTodo(todo.id)" class="w-5 h-5 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-200 hover:shadow-md"
              :style="{ borderColor: todo.priority ? ({P1:'#ef4444',P2:'#f59e0b',P3:'#3b82f6'} as any)[todo.priority] : '#d1d5db' }">
              <AppIcon v-if="todo.done" name="check" class="w-3 h-3 text-white" />
            </button>
            <div class="flex-1 min-w-0">
              <p class="text-[12px] font-medium truncate text-surface-800">{{ todo.title }}</p>
              <p class="text-[9px] mt-1 text-surface-400" v-if="todo.content">{{ (todo.content || '').replace(/<[^>]*>/g,'').slice(0,40) }}</p>
              <p class="text-[9px] mt-0.5 text-surface-400" v-if="todo.dueDate">{{ todo.dueDate.slice(0,10) }}</p>
            </div>
            <button @click.stop="removeProjectTodo(todo.id)" class="w-4 h-4 flex items-center justify-center rounded-full text-surface-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除"><AppIcon name="trash" size="10"/></button>
          </div>
          <div v-if="projectUndoneTodos.length === 0 && projectDoneTodos.length === 0" class="text-center py-10 text-surface-400 text-xs">暂无待办</div>
        </div>
        <div v-if="projectDoneTodos.length > 0" class="border-t border-surface-100 bg-white">
          <button @click="projectTodoDoneOpen = !projectTodoDoneOpen" class="w-full flex items-center gap-1.5 px-3 py-2 text-[10px] text-surface-400 hover:text-surface-600 transition-colors">
            <svg class="w-3 h-3 transition-transform" :class="projectTodoDoneOpen ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 5l7 7-7 7"/></svg>
            <span>已完成</span>
            <span class="ml-auto text-surface-300">{{ projectDoneTodos.length }} 项</span>
          </button>
          <div v-show="projectTodoDoneOpen" class="px-2 pb-2 space-y-1 max-h-[200px] overflow-y-auto">
            <div v-for="todo in projectDoneTodos" :key="todo.id" @click="selectedTodoId = todo.id"
              class="flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer group border bg-gray-50/80 border-gray-200"
              :class="selectedTodoId === todo.id ? 'bg-gray-100/70 border-gray-300' : ''">
              <button @click.stop="toggleProjectTodo(todo.id)" class="w-4 h-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all"
                style="border-color:#9ca3af;background:#9ca3af">
                <AppIcon name="check" class="w-2.5 h-2.5 text-white" />
              </button>
              <div class="flex-1 min-w-0">
                <p class="text-[11px] font-medium truncate text-gray-400 line-through">{{ todo.title }}</p>
                <div class="flex items-center gap-1 mt-0.5">
                  <span class="text-[8px] text-gray-300" v-if="todo.dueDate">{{ todo.dueDate.slice(0,10) }}</span>
                </div>
              </div>
              <button @click.stop="removeProjectTodo(todo.id)" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除"><AppIcon name="trash" size="8"/></button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="flex-1 flex min-w-0 overflow-hidden">
      <div class="flex-1 flex flex-col bg-white min-w-0 overflow-hidden">
        <TodoDetail
          ref="todoDetailRef"
          v-if="selectedTodo"
          :todo="selectedTodo"
          :projects="[{id: projectBackendId || projectId, name: projectName || ''}]"
          :hide-project-selector="true"
          @update="(id, data) => handleTodoUpdate(id, data)"
          @close="selectedTodoId = null"
        />
        <div v-else class="h-full flex items-center justify-center text-surface-300 text-xs">从左侧选择一个待办</div>
      </div>
      <ProjectFilePanel ref="filePanelRef" :project-id="projectId" v-if="selectedTodo" @insert-file="(f) => { const fname = f.custom_name || f.displayName || f.name || f.lib_cn || f.lib_gn || '未命名'; todoDetailRef?.insertFileAtCursor?.(f.id, fname) }" />
    </div>
  </div>
</template>
