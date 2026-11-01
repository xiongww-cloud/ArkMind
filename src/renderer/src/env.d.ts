/// <reference types="vite/client" />
declare const __APP_VERSION__: string

declare module '@/router' {
  import type { Router } from 'vue-router'
  const router: Router
  export default router
}
