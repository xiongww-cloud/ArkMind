<template>
  <div class="object-list">
    <div class="list-header">
      <span class="list-title">{{ title }}</span>
      <span class="list-count">共 {{ items.length }} 项</span>
    </div>
    <div class="list-content">
      <div v-for="item in items" :key="item.id" class="list-item" @click="handleItemClick(item)">
        <div class="item-icon">{{ getIcon(item.type) }}</div>
        <div class="item-info">
          <div class="item-name">{{ item.name }}</div>
          <div class="item-meta">
            <span class="item-type">{{ item.type }}</span>
            <span v-if="item.extra" class="item-extra">{{ item.extra }}</span>
          </div>
        </div>
        <div class="item-action">查看 →</div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
interface ObjectItem {
  id: string
  type: string
  name: string
  extra?: string
}

const props = defineProps<{
  title?: string
  items: ObjectItem[]
}>()

const emit = defineEmits<{
  (e: 'item-click', item: ObjectItem): void
}>()

function getIcon(type: string): string {
  switch (type) {
    case 'PROJECT': return '📋'
    case 'TODO': return '☑️'
    case 'FILE': return '📄'
    case 'NOTE': return '📝'
    case 'CONTACT': return '👤'
    case 'CHANNEL': return '🏢'
    case 'VENDOR': return '🏭'
    case 'CUSTOMER': return '🛒'
    case 'PRODUCT': return '📦'
    default: return '📌'
  }
}

function handleItemClick(item: ObjectItem) {
  emit('item-click', item)
}
</script>

<style scoped>
.object-list {
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  overflow: hidden;
  background: white;
}

.list-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  background: #f9fafb;
  border-bottom: 1px solid #e5e7eb;
}

.list-title {
  font-weight: 600;
  color: #1f2937;
}

.list-count {
  font-size: 13px;
  color: #6b7280;
}

.list-content {
  max-height: 400px;
  overflow-y: auto;
}

.list-item {
  display: flex;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid #f3f4f6;
  cursor: pointer;
  transition: background 0.2s;
}

.list-item:hover {
  background: #f9fafb;
}

.list-item:last-child {
  border-bottom: none;
}

.item-icon {
  font-size: 20px;
  margin-right: 12px;
}

.item-info {
  flex: 1;
  min-width: 0;
}

.item-name {
  font-weight: 500;
  color: #1f2937;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-meta {
  display: flex;
  gap: 8px;
  margin-top: 4px;
  font-size: 12px;
  color: #6b7280;
}

.item-type {
  padding: 1px 6px;
  background: #f3f4f6;
  border-radius: 4px;
}

.item-extra {
  color: #9ca3af;
}

.item-action {
  color: #3b82f6;
  font-size: 13px;
  font-weight: 500;
}
</style>
