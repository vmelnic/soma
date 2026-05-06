<template>
  <div class="soma-screen">
    <header class="soma-screen-header" v-if="node.title">
      <h1>{{ resolvedTitle }}</h1>
    </header>
    <div class="soma-screen-body">
      <SomaRenderer
        v-for="(child, i) in node.children"
        :key="i"
        :schema="child"
        :context="context"
        @action="$emit('action', $event)"
        @submit="$emit('submit', $event)"
        @navigate="$emit('navigate', $event)"
      />
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { resolveBinding } from '../utils/bindings.js';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

defineEmits(['action', 'submit', 'navigate']);

const resolvedTitle = computed(() => resolveBinding(props.node.title, props.context));
</script>
