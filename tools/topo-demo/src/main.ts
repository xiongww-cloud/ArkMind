import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import { registerTopoShapes } from './graph/node'
import { initIcons } from './graph/iconArt'

registerTopoShapes()
initIcons().finally(() => {
  createApp(App).mount('#app')
})
