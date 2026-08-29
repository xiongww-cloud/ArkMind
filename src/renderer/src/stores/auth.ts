import { defineStore } from 'pinia'
import { ref } from 'vue'
import { fetchUserProfile } from '@/api/db-client'

export interface User {
  id: string
  name: string
  username: string
  role: string
  phone: string
  email: string
  avatar: string
  aiConfig: string
}

const DEFAULT_USER: User = {
  id: '',
  name: '本地管理员',
  username: 'local-admin',
  role: 'OWNER',
  phone: '',
  email: '',
  avatar: '',
  aiConfig: '{}'
}

export const useAuthStore = defineStore('auth', () => {
  const currentUser = ref<User>({ ...DEFAULT_USER })

  let loaded = false

  async function loadUser() {
    if (loaded) return
    try {
      const user = await fetchUserProfile()
      currentUser.value = user
      loaded = true
    } catch (e) {
      console.warn('Failed to load user profile, using defaults:', e)
      currentUser.value = { ...DEFAULT_USER, id: 'local-user' }
    }
  }

  async function refreshUser() {
    loaded = false
    await loadUser()
  }

  function getAvatarSrc() {
    if (currentUser.value.avatar) {
      if (
        currentUser.value.avatar.startsWith('http') ||
        currentUser.value.avatar.startsWith('data:')
      ) {
        return currentUser.value.avatar
      }
      return `/api/user/avatar?t=${Date.now()}`
    }
    return ''
  }

  return { currentUser, loadUser, refreshUser, getAvatarSrc }
})
