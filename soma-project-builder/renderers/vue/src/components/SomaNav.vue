<template>
  <nav class="soma-nav">
    <button
      v-for="item in node.items"
      :key="item.id"
      class="soma-nav-item"
      :class="{ 'soma-nav-item-active': activeScreen === item.screen }"
      @click="$emit('navigate', item.screen)"
    >
      <span v-if="item.icon" class="soma-nav-icon">{{ item.icon }}</span>
      <span class="soma-nav-label">{{ item.label }}</span>
    </button>
  </nav>
</template>

<script setup>
defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
  activeScreen: { type: String, default: '' },
});

defineEmits(['navigate']);
</script>
