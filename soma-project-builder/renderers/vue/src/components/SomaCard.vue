<template>
  <div class="soma-card" @click="handleClick">
    <img
      v-if="resolvedImage"
      class="soma-card-image"
      :src="resolvedImage"
      :alt="resolvedTitle || ''"
    />
    <div class="soma-card-body">
      <h3 v-if="resolvedTitle" class="soma-card-title">{{ resolvedTitle }}</h3>
      <p v-if="resolvedSubtitle" class="soma-card-subtitle">{{ resolvedSubtitle }}</p>
      <SomaRenderer
        v-for="(child, i) in (node.children || [])"
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
import { resolveBinding, resolveBindMap } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['action', 'submit', 'navigate']);
const soma = useSoma();

const resolvedTitle = computed(() => resolveBinding(props.node.title, props.context));
const resolvedSubtitle = computed(() => resolveBinding(props.node.subtitle, props.context));
const resolvedImage = computed(() => resolveBinding(props.node.image, props.context));

async function handleClick() {
  const action = props.node.action;
  if (!action) return;
  try {
    const input = resolveBindMap(action.bind, props.context);
    const result = await soma.execute(action.routine, input);
    emit('action', { routine: action.routine, result, on_success: action.on_success });
  } catch (err) {
    emit('action', { routine: action.routine, error: err, on_failure: action.on_failure });
  }
}
</script>
