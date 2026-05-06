<template>
  <span class="soma-status-badge" :class="`soma-status-badge-${variant}`">
    {{ resolvedValue }}
  </span>
</template>

<script setup>
import { computed } from 'vue';
import { resolveBinding } from '../utils/bindings.js';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const resolvedValue = computed(() => resolveBinding(props.node.value, props.context));

const variant = computed(() => {
  if (!props.node.variant_map) return 'neutral';
  return props.node.variant_map[resolvedValue.value] || 'neutral';
});
</script>
