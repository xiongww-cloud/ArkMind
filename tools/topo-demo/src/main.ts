import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import { registerTopoShapes } from './graph/node'

registerTopoShapes()
createApp(App).mount('#app')
