import { reactive, ref } from 'vue'

interface Task {
  id: number
  name: string
  status: 'uploading' | 'done' | 'error'
  error?: string
}

const tasks: Task[] = reactive([])
let nextId = 0
const visible = ref(false)
let hideTimer: ReturnType<typeof setTimeout> | null = null

export function useUploadProgress() {
  function start(name: string) {
    // 清理掉所有已完成/失败的任务，避免面板堆积旧记录
    for (let i = tasks.length - 1; i >= 0; i--) {
      if (tasks[i].status === 'done' || tasks[i].status === 'error') tasks.splice(i, 1)
    }
    const id = ++nextId
    tasks.push({ id, name, status: 'uploading' })
    visible.value = true
    if (hideTimer) { clearTimeout(hideTimer); hideTimer = null }
    return tasks[tasks.length - 1]
  }

  function done(task: Task) {
    task.status = 'done'
    scheduleHide()
  }

  function fail(task: Task, err: string) {
    task.status = 'error'
    task.error = err
    scheduleHide()
  }

  function scheduleHide() {
    if (tasks.every(t => t.status === 'done' || t.status === 'error')) {
      hideTimer = setTimeout(() => { visible.value = false; tasks.splice(0, tasks.length) }, 3000)
    }
  }

  return { tasks, visible, start, done, fail }
}
