<template>
  <div class="h-full overflow-hidden">
    <router-view v-slot="{ Component }">
      <KeepAlive include="AgentChat">
        <component :is="Component" />
      </KeepAlive>
    </router-view>
  </div>
</template>
