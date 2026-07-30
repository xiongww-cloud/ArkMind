import { createApp } from 'vue'
import ChatWindow from './components/chat/ChatWindow.vue'
import './assets/main.css'
import './assets/markdown.css'

const app = createApp(ChatWindow)
app.mount('#chat-root')
