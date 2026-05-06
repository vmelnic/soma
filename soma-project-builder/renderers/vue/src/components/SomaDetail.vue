<template>
  <div class="soma-detail">
    <div v-if="loading" class="soma-detail-loading">Loading...</div>
    <div v-else-if="error" class="soma-detail-error">{{ error }}</div>
    <template v-else>
      <dl class="soma-detail-fields">
        <template v-for="(field, i) in node.fields" :key="i">
          <dt class="soma-detail-key">{{ field.key }}</dt>
          <dd class="soma-detail-value">{{ resolveBinding(field.value, data) }}</dd>
        </template>
      </dl>
      <SomaRenderer
        v-for="(child, i) in (node.children || [])"
        :key="i"
        :schema="child"
        :context="data"
        @action="$emit('action', $event)"
        @submit="$emit('submit', $event)"
        @navigate="$emit('navigate', $event)"
      />
    </template>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { resolveBinding, resolveBindMap } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

defineEmits(['action', 'submit', 'navigate']);

const soma = useSoma();
const data = ref(props.context);
const loading = ref(false);
const error = ref(null);

onMounted(async () => {
  if (!props.node.source) return;
  loading.value = true;
  try {
    const input = resolveBindMap(props.node.bind, props.context);
    const result = await soma.execute(props.node.source, input);
    // Unwrap: result could be { rows: [...] } or the object directly
    if (result.rows && result.rows.length > 0) {
      data.value = result.rows[0];
    } else if (result.data) {
      data.value = result.data;
    } else {
      data.value = result;
    }
  } catch (err) {
    error.value = err.message;
  } finally {
    loading.value = false;
  }
});
</script>
