<template>
  <button
    class="soma-action-button"
    :class="`soma-action-button-${node.variant || 'primary'}`"
    :disabled="executing"
    @click="handleClick"
  >
    {{ executing ? '...' : node.label }}
  </button>
</template>

<script setup>
import { ref } from 'vue';
import { resolveBindMap } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['action', 'navigate']);
const soma = useSoma();
const executing = ref(false);

async function handleClick() {
  const action = props.node.action;
  if (!action) return;

  if (props.node.confirm) {
    if (!window.confirm(props.node.confirm)) return;
  }

  executing.value = true;
  try {
    const input = resolveBindMap(action.bind, props.context);
    const result = await soma.execute(action.routine, input);
    emit('action', { routine: action.routine, result, on_success: action.on_success });
    if (action.on_success) {
      emit('navigate', action.on_success);
    }
  } catch (err) {
    emit('action', { routine: action.routine, error: err, on_failure: action.on_failure });
  } finally {
    executing.value = false;
  }
}
</script>
