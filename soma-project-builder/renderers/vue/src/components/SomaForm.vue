<template>
  <form class="soma-form" @submit.prevent="handleSubmit">
    <SomaRenderer
      v-for="(child, i) in node.children"
      :key="i"
      :schema="child"
      :context="context"
      :form-data="formData"
      @action="$emit('action', $event)"
      @navigate="$emit('navigate', $event)"
    />
    <div v-if="error" class="soma-form-error">{{ error }}</div>
    <button type="submit" class="soma-form-submit" :disabled="submitting">
      {{ submitting ? 'Submitting...' : (node.submit_label || 'Submit') }}
    </button>
  </form>
</template>

<script setup>
import { ref, reactive } from 'vue';
import { resolveBindMap } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['action', 'submit', 'navigate']);
const soma = useSoma();

const formData = reactive({});
const submitting = ref(false);
const error = ref(null);

async function handleSubmit() {
  const action = props.node.submit;
  if (!action) return;

  submitting.value = true;
  error.value = null;

  try {
    // Merge bind mappings (resolved from context) with form field values
    const bindInput = resolveBindMap(action.bind, props.context);
    const input = { ...bindInput, ...formData };
    const result = await soma.execute(action.routine, input);
    emit('submit', { routine: action.routine, result, input, on_success: action.on_success });
  } catch (err) {
    error.value = err.message;
    emit('submit', { routine: action.routine, error: err, input: { ...bindInput, ...formData }, on_failure: action.on_failure });
  } finally {
    submitting.value = false;
  }
}
</script>
