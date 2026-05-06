<template>
  <div class="soma-entity-list">
    <div v-if="loading" class="soma-entity-list-loading">Loading...</div>
    <div v-else-if="error" class="soma-entity-list-error">{{ error }}</div>
    <template v-else-if="items.length > 0">
      <SomaRenderer
        v-for="(item, i) in items"
        :key="i"
        :schema="node.item"
        :context="item"
        @action="$emit('action', $event)"
        @submit="$emit('submit', $event)"
        @navigate="$emit('navigate', $event)"
      />
    </template>
    <SomaEmptyState
      v-else-if="node.empty"
      :node="node.empty"
      :context="context"
      @action="$emit('action', $event)"
    />
    <div v-else class="soma-entity-list-empty">No items</div>
  </div>
</template>

<script setup>
import { ref, onMounted, watch } from 'vue';
import { resolveBindMap } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';
import SomaRenderer from '../SomaRenderer.vue';
import SomaEmptyState from './SomaEmptyState.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

defineEmits(['action', 'submit', 'navigate']);

const soma = useSoma();
const items = ref([]);
const loading = ref(false);
const error = ref(null);

async function fetchData() {
  if (!props.node.source) return;
  loading.value = true;
  error.value = null;
  try {
    const input = resolveBindMap(props.node.bind, props.context);
    const result = await soma.execute(props.node.source, input);
    // The routine result may contain rows directly, or nested in a data/rows/items field
    items.value = extractItems(result);
  } catch (err) {
    error.value = err.message;
  } finally {
    loading.value = false;
  }
}

function extractItems(result) {
  if (Array.isArray(result)) return result;
  if (result.rows && Array.isArray(result.rows)) return result.rows;
  if (result.data && Array.isArray(result.data)) return result.data;
  if (result.items && Array.isArray(result.items)) return result.items;
  // Walk one level into port call results
  if (result.result) return extractItems(result.result);
  return [];
}

onMounted(fetchData);
watch(() => props.context, fetchData, { deep: true });
</script>
