import { createRouter, createWebHashHistory } from 'vue-router'

const routes = [
  { path: '/', redirect: '/dashboard' },
  { path: '/dashboard', name: 'Dashboard', component: () => import('@/views/Dashboard.vue'), meta: { title: '总览' } },
  { path: '/ai-insights', name: 'AIInsights', component: () => import('@/views/AIInsights.vue'), meta: { title: 'AI洞察' } },
  { path: '/projects', name: 'Projects', component: () => import('@/views/ProjectHub.vue'), meta: { title: '项目库' } },
  { path: '/projects/:id', name: 'ProjectDetail', component: () => import('@/views/ProjectDetail.vue'), props: true, meta: { title: '项目详情' } },
  { path: '/products', name: 'Products', component: () => import('@/views/ProductCenter.vue'), meta: { title: '产品库' } },
  { path: '/assets', name: 'Assets', component: () => import('@/views/AssetLibrary.vue'), meta: { title: '资料库' } },
  { path: '/notes', name: 'Notes', component: () => import('@/views/NotesPage.vue'), meta: { title: '笔记库' } },
  { path: '/todos', name: 'Todos', component: () => import('@/views/TodoPage.vue'), meta: { title: '待办' } },
  { path: '/contacts', name: 'Contacts', component: () => import('@/views/ContactsPage.vue'), meta: { title: '通讯录' } },
  { path: '/email', name: 'Email', component: () => import('@/views/EmailPage.vue'), meta: { title: '邮件' } },
  { path: '/knowledge-graph', name: 'KnowledgeGraph', component: () => import('@/views/KnowledgeGraph.vue'), meta: { title: '知识观测台' } },
  { path: '/settings', name: 'Settings', component: () => import('@/views/Settings.vue'), meta: { title: '系统设置' } },
  { path: '/knowledge', component: () => import('@/views/KnowledgeLayout.vue'), redirect: '/knowledge/brain', children: [
    { path: 'brain', name: 'AgentChat', component: () => import('@/views/AgentChat.vue'), meta: { title: 'AI 助手' } }
  ]}
]

const router = createRouter({ history: createWebHashHistory(), routes })
export default router
