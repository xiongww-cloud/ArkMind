import { ref, onUnmounted } from 'vue'
import { fetchMigrationProgress } from '@/api/db-client'

interface MigrationProgress {
  status: string         // idle | migrating | completed | failed
  direction: string      // LOCAL_TO_OSS | OSS_TO_LOCAL
  total: number
  current: number
  errorMessage: string
  startedAt: string | null
  finishedAt: string | null
}

/**
 * Composable for polling storage migration progress via IPC.
 *
 * Usage:
 *   const { progress, isMigrating, startPolling, stopPolling } = useStorageMigration()
 *   startPolling()
 */
export function useStorageMigration() {
  const progress = ref<MigrationProgress | null>(null)
  const isMigrating = ref(false)
  const isCompleted = ref(false)
  const isFailed = ref(false)

  let pollTimer: ReturnType<typeof setInterval> | null = null
  let hideTimer: ReturnType<typeof setTimeout> | null = null

  async function checkProgress() {
    try {
      const data = await fetchMigrationProgress()

      if (!data || data.status === 'idle') {
        // No active migration
        stopPolling()
        return
      }

      progress.value = data

      if (data.status === 'migrating') {
        isMigrating.value = true
        isCompleted.value = false
        isFailed.value = false
      } else if (data.status === 'completed') {
        isMigrating.value = false
        isCompleted.value = true
        isFailed.value = false
        // Auto-hide after 5 seconds on success
        hideTimer = setTimeout(() => stopPolling(), 5000)
      } else if (data.status === 'failed') {
        isMigrating.value = false
        isCompleted.value = false
        isFailed.value = true
      } else {
        isMigrating.value = false
        isCompleted.value = false
        isFailed.value = false
      }
    } catch {
      // IPC error — silently retry
    }
  }

  function startPolling() {
    if (pollTimer) return
    checkProgress() // Immediate first check
    pollTimer = setInterval(checkProgress, 2000)
  }

  function stopPolling() {
    if (pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }
    if (hideTimer) {
      clearTimeout(hideTimer)
      hideTimer = null
    }
    isMigrating.value = false
    isCompleted.value = false
    isFailed.value = false
  }

  /** Check if there's an active migration on mount */
  async function checkActive() {
    try {
      const data = await fetchMigrationProgress()
      if (data && data.status === 'migrating') {
        progress.value = data
        startPolling()
      }
    } catch { /* ignore */ }
  }

  onUnmounted(() => {
    stopPolling()
  })

  return {
    progress,
    isMigrating,
    isCompleted,
    isFailed,
    startPolling,
    stopPolling,
    checkActive,
  }
}
