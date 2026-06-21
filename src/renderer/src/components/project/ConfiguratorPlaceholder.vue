<script setup lang="ts">
import { ref } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'

defineProps({
  project: { type: Object, required: true }
})

const showPlaceholder = ref(true)
</script>

<template>
  <div class="card">
    <div class="text-center py-8">
      <!-- Placeholder Illustration -->
      <div class="mx-auto h-20 w-20 rounded-2xl bg-brand-50 flex items-center justify-center mb-4">
        <AppIcon name="settings" class="h-10 w-10 text-brand-400" />
      </div>

      <h3 class="text-sm font-semibold text-surface-700 mb-1">Solution Configurator</h3>
      <p class="text-sm text-surface-400 max-w-md mx-auto mb-6">
        The parameter configuration engine will be integrated here as a pluggable component.
        Build product quotes with real-time pricing, parameter selection, and bill-of-materials generation.
      </p>

      <!-- Placeholder UI Mock -->
      <div class="max-w-md mx-auto border border-surface-200 rounded-xl p-4 bg-surface-50 text-left space-y-3">
        <div class="flex items-center gap-2 text-xs text-surface-400">
          <AppIcon name="clock" class="h-3.5 w-3.5" />
          Coming Soon
        </div>

        <!-- Mock Parameter Row -->
        <div class="flex items-center justify-between py-2 border-b border-surface-100">
          <span class="text-xs text-surface-500">Product Model</span>
          <span class="text-xs font-medium text-surface-700">OceanStor Dorado 8000 V6</span>
        </div>
        <div class="flex items-center justify-between py-2 border-b border-surface-100">
          <span class="text-xs text-surface-500">Controller</span>
          <span class="text-xs font-medium text-surface-700">4-Controller Active-Active</span>
        </div>
        <div class="flex items-center justify-between py-2 border-b border-surface-100">
          <span class="text-xs text-surface-500">SSD Type</span>
          <span class="text-xs font-medium text-surface-700">7.68TB NVMe SSD x24</span>
        </div>
        <div class="flex items-center justify-between py-2 border-b border-surface-100">
          <span class="text-xs text-surface-500">Protocol</span>
          <span class="text-xs font-medium text-surface-700">FC 32Gb + NVMe-oF</span>
        </div>
        <div class="flex items-center justify-between py-2">
          <span class="text-xs text-surface-500">Warranty</span>
          <span class="text-xs font-medium text-surface-700">3-Year 24x7</span>
        </div>

        <div class="flex items-center justify-between pt-3 border-t border-surface-200">
          <span class="text-xs font-semibold text-surface-700">Estimated Total</span>
          <span class="text-sm font-bold text-brand-600">¥4,200,000</span>
        </div>
      </div>

      <div class="mt-6 flex items-center justify-center gap-2">
        <div class="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse" />
        <span class="text-xs text-surface-400">This component is under development</span>
      </div>
    </div>
  </div>
</template>
