import { createApp } from 'vue'
import SpriteWindow from './components/sprite/SpriteWindow.vue'
import './assets/main.css'

const app = createApp(SpriteWindow)
app.mount('#sprite-root')
