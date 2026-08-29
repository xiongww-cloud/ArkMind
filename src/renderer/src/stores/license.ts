import { defineStore } from 'pinia'
import { ref } from 'vue'

export type LicenseTier = 'basic' | 'advanced' | 'enhanced'

export const useLicenseStore = defineStore('license', () => {
  const tier = ref<LicenseTier>('basic')
  const product = ref('')

  function setTier(t: LicenseTier, p: string = '') {
    tier.value = t; product.value = p
  }

  return { tier, product, setTier }
})
