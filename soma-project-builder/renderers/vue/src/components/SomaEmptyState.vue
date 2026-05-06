<template>
  <div class="soma-empty-state">
    <h3 class="soma-empty-state-title">{{ node.title }}</h3>
    <p v-if="node.message" class="soma-empty-state-message">{{ node.message }}</p>
    <SomaActionButton
      v-if="node.action"
      :node="{ type: 'action_button', label: node.action.label || 'Go', action: node.action }"
      :context="context"
      @action="$emit('action', $event)"
    />
  </div>
</template>

<script setup>
import SomaActionButton from './SomaActionButton.vue';

defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

defineEmits(['action']);
</script>
