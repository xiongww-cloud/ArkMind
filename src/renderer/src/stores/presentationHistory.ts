import { ref } from 'vue'
import { defineStore } from 'pinia'
import { usePresentationsStore } from '@/stores/presentations'

/**
 * 演示材料 Command/Undo 层。
 * 所有对 Deck 的修改都以「整篇快照」为单位提交（before/after），
 * 因此 AI 操作、手工编辑、主题切换共用同一套可撤销历史。
 *
 * 合并规则：同一 mergeKey 在 MERGE_MS 内的连续提交合并为一步（打字去抖），
 * 撤销时一次性回到本次输入的起点。
 */
interface HistoryEntry {
  label: string
  key?: string
  pid: string
  before: string
  after: string
}

const CAP = 60
const MERGE_MS = 1200

export const usePresentationHistoryStore = defineStore('presentationHistory', () => {
  const undoStack = ref<HistoryEntry[]>([])
  const redoStack = ref<HistoryEntry[]>([])
  /** 正在进行的编辑会话：key → 起始快照 */
  const sessionBefore = new Map<string, string>()
  let lastAt = 0

  function snapshot(pid: string): string | null {
    const p = usePresentationsStore().getById(pid)
    if (!p) return null
    return JSON.stringify({ theme: p.theme, slides: p.slides })
  }

  function restore(pid: string, json: string) {
    const p = usePresentationsStore().getById(pid)
    if (!p) return
    const data = JSON.parse(json)
    if (data.theme !== undefined) p.theme = data.theme
    if (data.slides !== undefined) p.slides = data.slides
    usePresentationsStore().touch(p)
  }

  /** 记录一次会话起点（key 通常为 `blockId:field` 或一次性唯一键） */
  function beginSession(key: string, pid: string) {
    const before = snapshot(pid)
    if (before) sessionBefore.set(key, before)
  }

  /** 提交会话终点；与上次同 key 且间隔 < MERGE_MS 则合并 */
  function commit(key: string, label: string, pid: string) {
    const after = snapshot(pid)
    if (!after) {
      sessionBefore.delete(key)
      return
    }
    const before = sessionBefore.get(key) ?? after
    sessionBefore.delete(key)
    if (before === after) return
    const now = Date.now()
    const last = undoStack.value[undoStack.value.length - 1]
    if (last && last.pid === pid && last.key === key && now - lastAt < MERGE_MS) {
      last.after = after
    } else {
      undoStack.value.push({ label, key, before, after, pid })
      if (undoStack.value.length > CAP) undoStack.value.shift()
    }
    lastAt = now
    redoStack.value = []
  }

  function undo() {
    const entry = undoStack.value.pop()
    if (!entry) return
    restore(entry.pid, entry.before)
    redoStack.value.push(entry)
    sessionBefore.clear()
    lastAt = 0
  }

  function redo() {
    const entry = redoStack.value.pop()
    if (!entry) return
    restore(entry.pid, entry.after)
    undoStack.value.push(entry)
    sessionBefore.clear()
    lastAt = 0
  }

  function clear() {
    undoStack.value = []
    redoStack.value = []
    sessionBefore.clear()
    lastAt = 0
  }

  return {
    undoStack,
    redoStack,
    beginSession,
    commit,
    undo,
    redo,
    clear
  }
})
