<script setup lang="ts">
import { ref, watch } from 'vue'
import type { WithdrawnFile } from '@/api/db-client'

const props = defineProps<{ visible: boolean; anchorText: string; files: WithdrawnFile[] }>()
const emit = defineEmits<{ confirm: [deleteFiles: boolean]; cancel: [] }>()

const delFiles = ref(false)
watch(() => props.visible, (v) => { if (v) delFiles.value = false })
</script>

<template>
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="visible" class="fixed inset-0 z-[120] bg-black/30 flex items-center justify-center" @click.self="emit('cancel')">
        <div class="bg-white rounded-2xl shadow-2xl w-[400px] max-w-[90vw] p-5">
          <div class="flex items-center gap-2 mb-2">
            <span class="w-7 h-7 rounded-lg bg-red-50 text-red-500 flex items-center justify-center flex-shrink-0 text-[14px]">↺</span>
            <h3 class="text-[14px] font-semibold text-surface-800">撤回这条消息之后的全部对话？</h3>
          </div>
          <p class="text-[11px] text-surface-500 leading-relaxed mb-1">从这条起的内容（含其后的回答）将从会话与 AI 上下文中移除，工具记忆与压缩摘要同步断代：</p>
          <p class="text-[11px] text-surface-700 bg-surface-50 rounded-lg px-2.5 py-1.5 leading-snug mb-3 max-h-[88px] overflow-y-auto whitespace-pre-wrap break-all">「{{ anchorText }}」</p>
          <div v-if="files.length" class="border border-surface-100 rounded-xl px-3 py-2 mb-3">
            <label class="flex items-start gap-2 cursor-pointer">
              <input type="checkbox" v-model="delFiles" class="w-3.5 h-3.5 mt-0.5 accent-red-500" />
              <span class="text-[11px] text-surface-600 leading-snug">
                同时删除期间生成的 {{ files.length }} 个文件：
                <span class="text-surface-400">{{ files.slice(0, 3).map(f => f.name).join('、') }}{{ files.length > 3 ? ' 等' : '' }}</span>
              </span>
            </label>
            <p v-if="!delFiles" class="text-[10px] text-surface-400 mt-1 ml-[22px]">不勾选则保留在右侧「交付文件」面板，可随时删除</p>
          </div>
          <div class="flex justify-end gap-2">
            <button @click="emit('cancel')" class="px-4 py-1.5 text-xs font-medium rounded-lg border border-surface-200 text-surface-600 hover:bg-surface-50 transition-colors">取消</button>
            <button @click="emit('confirm', delFiles)" class="px-4 py-1.5 text-xs font-medium rounded-lg bg-red-500 text-white hover:bg-red-600 transition-colors">撤回</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-fade-enter-active, .modal-fade-leave-active { transition: opacity .18s ease; }
.modal-fade-enter-from, .modal-fade-leave-to { opacity: 0; }
</style>
