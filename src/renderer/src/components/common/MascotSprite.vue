<script setup lang="ts">
defineProps<{ state: 'idle' | 'thinking' | 'speaking' | 'notifying' }>()
</script>

<template>
  <div class="mascot-wrapper">
    <svg viewBox="0 0 64 64" class="mascot-svg" :class="state" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="robotBlueGrad" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stop-color="#64B5F6" />
          <stop offset="100%" stop-color="#1976D2" />
        </linearGradient>
        <linearGradient id="visorGrad" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stop-color="#283593" />
          <stop offset="100%" stop-color="#1A237E" />
        </linearGradient>
      </defs>

      <!-- 左手 -->
      <ellipse class="bot-arm-l" cx="16" cy="42" rx="4.5" ry="3.5" fill="url(#robotBlueGrad)" transform="rotate(30, 16, 42)" />
      
      <!-- 右手 -->
      <ellipse class="bot-arm-r" cx="48" cy="42" rx="4.5" ry="3.5" fill="url(#robotBlueGrad)" transform="rotate(-30, 48, 42)" />

      <!-- 身体 -->
      <g class="bot-body">
        <path d="M 22 40 Q 32 36 42 40 Q 45 54 32 56 Q 19 54 22 40 Z" fill="url(#robotBlueGrad)" />
        <ellipse cx="26" cy="45" rx="2.5" ry="2" fill="#FFFFFF" opacity="0.5" transform="rotate(-15, 26, 45)" />
      </g>

      <!-- 头部群组 -->
      <g class="bot-head-group">
        <!-- 天线 -->
        <path d="M 22 22 Q 12 12 16 6" stroke="#4FC3F7" stroke-width="2" fill="none" stroke-linecap="round" />
        <circle class="bot-antenna-ball" cx="16" cy="6" r="3" fill="#1A237E" />

        <!-- 头部主结构 -->
        <rect x="18" y="14" width="28" height="24" rx="11" fill="url(#robotBlueGrad)" />
        <ellipse cx="23" cy="18" rx="3.5" ry="1.5" fill="#FFFFFF" opacity="0.5" transform="rotate(-20, 23, 18)" />

        <!-- 面罩 -->
        <rect x="20" y="20" width="24" height="11" rx="4" fill="url(#visorGrad)" />

        <!-- 眼睛 -->
        <g class="bot-eyes">
          <circle cx="26" cy="25" r="2.5" fill="#FFFFFF" />
          <circle cx="38" cy="25" r="2.5" fill="#FFFFFF" />
        </g>

        <!-- 嘴巴 -->
        <g class="bot-mouth-group">
          <path class="mouth-idle" d="M 29 34 Q 32 36 35 34" fill="none" stroke="#1A237E" stroke-width="1.5" stroke-linecap="round" />
          <ellipse class="mouth-open" cx="32" cy="35" rx="2.5" ry="3.5" fill="#1A237E" opacity="0" />
        </g>
      </g>
    </svg>
    <!-- 地面投影 -->
    <span class="mascot-ground-shadow" />
  </div>
</template>

<style scoped>
.mascot-wrapper {
  position: relative;
  width: 100%; height: 100%;
  display: flex; align-items: center; justify-content: center;
  padding: 2px;
  box-sizing: border-box;
}
.mascot-svg {
  width: 64px; height: 64px; overflow: visible;
}
.mascot-ground-shadow {
  position: absolute; bottom: -3px; left: 50%; transform: translateX(-50%);
  width: 30px; height: 6px; border-radius: 50%;
  background: radial-gradient(ellipse, rgba(0,0,0,0.18) 0%, transparent 70%);
  pointer-events: none;
  transition: all 0.3s ease;
}
.thinking .mascot-ground-shadow { width: 24px; height: 5px; background: radial-gradient(ellipse, rgba(99,102,241,0.2) 0%, transparent 70%); }
.speaking .mascot-ground-shadow { width: 34px; height: 6px; background: radial-gradient(ellipse, rgba(37,99,235,0.2) 0%, transparent 70%); }
.notifying .mascot-ground-shadow { width: 26px; height: 5px; background: radial-gradient(ellipse, rgba(255,82,82,0.2) 0%, transparent 70%); }

/* ── 分层磁悬浮动画 ── */
.bot-head-group { 
  transform-origin: 32px 26px; 
  animation: float-head 3s ease-in-out infinite; 
  transition: transform 0.4s ease;
}
.bot-body { 
  transform-origin: 32px 48px;
  animation: float-body 3s ease-in-out infinite 0.2s; 
  transition: transform 0.4s ease;
}
.bot-arm-l { 
  transform-origin: 16px 42px; 
  animation: float-arm-l 3.5s ease-in-out infinite; 
  transition: transform 0.4s ease; 
}
.bot-arm-r { 
  transform-origin: 48px 42px; 
  animation: float-arm-r 3.2s ease-in-out infinite; 
  transition: transform 0.4s ease; 
}
.bot-antenna-ball { 
  transition: fill 0.3s ease, filter 0.3s ease; 
}
.bot-eyes { 
  transform-origin: 32px 26px; 
  transition: transform 0.2s ease; 
}
.mouth-idle { 
  transition: opacity 0.2s; 
  opacity: 1; 
}
.mouth-open { 
  transition: opacity 0.2s; 
  opacity: 0; 
  transform-origin: 32px 35px; 
}

/* ── Idle 动画 ── */
.idle .bot-eyes {
  animation: blink 4s ease-in-out infinite;
}
@keyframes blink {
  0%, 90%, 100% { transform: scaleY(1); }
  95% { transform: scaleY(0.1); }
}
@keyframes float-head { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-3px); } }
@keyframes float-body { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-2px); } }
@keyframes float-arm-l { 0%, 100% { transform: translateY(0) rotate(0deg); } 50% { transform: translateY(-4px) rotate(5deg); } }
@keyframes float-arm-r { 0%, 100% { transform: translateY(0) rotate(0deg); } 50% { transform: translateY(-5px) rotate(-5deg); } }

/* ── Thinking ── */
.thinking .bot-head-group { animation: think-head-scan 2.5s ease-in-out infinite; }
.thinking .bot-body { transform: scale(0.95, 1.05); animation: none; }
.thinking .bot-eyes { transform: scaleY(0.8) translateX(-2px); }
.thinking .bot-arm-r { transform: translate(-4px, -12px) rotate(-20deg); animation: none; }
.thinking .bot-arm-l { transform: translate(2px, 4px) rotate(10deg); animation: none; }
.thinking .bot-antenna-ball { 
  fill: #FFCA28; filter: drop-shadow(0 0 4px #FFCA28);
  animation: pulse-antenna 1s infinite alternate; 
}
.thinking .mouth-idle { d: path("M 29 35 Q 32 34 35 35"); }

@keyframes think-head-scan {
  0%, 100% { transform: rotate(5deg) translateX(2px); }
  50% { transform: rotate(-8deg) translateX(-2px); }
}
@keyframes pulse-antenna { 0% { opacity: 0.6; transform: scale(1); } 100% { opacity: 1; transform: scale(1.3); } }

/* ── Speaking ── */
.speaking .bot-head-group { animation: speak-head-nod 2s infinite; }
.speaking .bot-body { animation: speak-body-bounce 2s infinite; }
.speaking .bot-arm-r { animation: speak-arm-gesture 2s ease-in-out infinite; }
.speaking .bot-antenna-ball { fill: #4FC3F7; filter: drop-shadow(0 0 3px #4FC3F7); }
.speaking .mouth-idle { opacity: 0; }
.speaking .mouth-open { opacity: 1; animation: speak-lip-sync 2s infinite; }

@keyframes speak-head-nod {
  0%, 100% { transform: translateY(0) rotate(0deg); }
  20% { transform: translateY(2px) rotate(3deg); } 
  45% { transform: translateY(-1px) rotate(-2deg); }
  70% { transform: translateY(1px) rotate(1deg); }
}
@keyframes speak-body-bounce {
  0%, 100% { transform: scale(1, 1) translateY(0); }
  20% { transform: scale(1.02, 0.96) translateY(2px); }
  45% { transform: scale(0.98, 1.02) translateY(-1px); }
}
@keyframes speak-arm-gesture {
  0%, 100% { transform: translate(0, 0) rotate(0deg); }
  20% { transform: translate(-2px, -8px) rotate(-30deg); } 
  45% { transform: translate(0px, -4px) rotate(-10deg); }
  70% { transform: translate(-1px, -6px) rotate(-20deg); }
}
@keyframes speak-lip-sync {
  0%   { transform: scale(0.8, 0.2); }
  15%  { transform: scale(0.6, 1.5); } 
  30%  { transform: scale(1.3, 1.3); } 
  45%  { transform: scale(1.6, 0.4); } 
  60%  { transform: scale(0.8, 0.2); } 
  75%  { transform: scale(1.1, 1.0); }
  90%  { transform: scale(0.7, 0.8); }
  100% { transform: scale(0.8, 0.2); }
}

/* ── Notifying ── */
.notifying .bot-head-group { animation: notify-jump-head 1.2s cubic-bezier(0.28, 0.84, 0.42, 1) infinite; }
.notifying .bot-body { animation: notify-jump-body 1.2s cubic-bezier(0.28, 0.84, 0.42, 1) infinite; }
.notifying .bot-arm-l { transform: translate(4px, -16px) rotate(45deg); animation: none; }
.notifying .bot-arm-r { transform: translate(-4px, -16px) rotate(-45deg); animation: none; }
.notifying .bot-eyes { transform: scaleY(0.15); }
.notifying .bot-antenna-ball { 
  fill: #FF5252; filter: drop-shadow(0 0 6px #FF5252);
  animation: pulse-antenna 0.3s infinite alternate; 
}
.notifying .mouth-idle { opacity: 0; }
.notifying .mouth-open { opacity: 1; transform: scale(1.2, 1.2); animation: none; }

@keyframes notify-jump-head {
  0%, 100% { transform: translateY(0); }
  20% { transform: translateY(2px); }
  50% { transform: translateY(-10px); }
  70% { transform: translateY(1px); }
}
@keyframes notify-jump-body {
  0%, 100% { transform: scale(1, 1) translateY(0); }
  20% { transform: scale(1.1, 0.9) translateY(4px); } 
  50% { transform: scale(0.9, 1.1) translateY(-8px); } 
  70% { transform: scale(1.05, 0.95) translateY(2px); } 
}
</style>
