<template>
  <div class="soma-field" :class="`soma-field-${fieldType}`">
    <label v-if="node.label" :for="fieldId" class="soma-field-label">
      {{ node.label }}
      <span v-if="node.required" class="soma-field-required">*</span>
    </label>

    <input
      v-if="fieldType === 'text'"
      :id="fieldId"
      type="text"
      class="soma-field-input"
      :name="node.name"
      :placeholder="node.placeholder"
      :required="node.required"
      :value="modelValue"
      @input="update($event.target.value)"
    />

    <input
      v-else-if="fieldType === 'phone'"
      :id="fieldId"
      type="tel"
      class="soma-field-input"
      :name="node.name"
      :placeholder="node.placeholder || '+1234567890'"
      :required="node.required"
      :value="modelValue"
      @input="update($event.target.value)"
    />

    <input
      v-else-if="fieldType === 'number'"
      :id="fieldId"
      type="number"
      class="soma-field-input"
      :name="node.name"
      :placeholder="node.placeholder"
      :required="node.required"
      :value="modelValue"
      @input="update($event.target.value)"
    />

    <input
      v-else-if="fieldType === 'datetime'"
      :id="fieldId"
      type="datetime-local"
      class="soma-field-input"
      :name="node.name"
      :required="node.required"
      :value="modelValue"
      @input="update($event.target.value)"
    />

    <select
      v-else-if="fieldType === 'select'"
      :id="fieldId"
      class="soma-field-input"
      :name="node.name"
      :required="node.required"
      :value="modelValue"
      @change="update($event.target.value)"
    >
      <option value="" disabled>{{ node.placeholder || 'Select...' }}</option>
      <option
        v-for="opt in resolvedOptions"
        :key="opt.value"
        :value="opt.value"
      >
        {{ opt.label }}
      </option>
    </select>

    <input
      v-else-if="fieldType === 'image'"
      :id="fieldId"
      type="file"
      accept="image/*"
      class="soma-field-input"
      :name="node.name"
      :required="node.required"
      @change="handleFile($event)"
    />

    <!-- Fallback to text for unknown types -->
    <input
      v-else
      :id="fieldId"
      type="text"
      class="soma-field-input"
      :name="node.name"
      :placeholder="node.placeholder"
      :required="node.required"
      :value="modelValue"
      @input="update($event.target.value)"
    />
  </div>
</template>

<script setup>
import { computed, ref, onMounted } from 'vue';
import { resolveBinding } from '../utils/bindings.js';
import { useSoma } from '../composables/useSoma.js';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
  formData: { type: Object, default: () => ({}) },
});

const soma = useSoma();
const dynamicOptions = ref(null);

const fieldType = computed(() => {
  const t = props.node.type;
  return t.startsWith('field:') ? t.slice(6) : t;
});

const fieldId = computed(() => `soma-field-${props.node.name}`);

const modelValue = computed(() => {
  if (props.formData && props.node.name in props.formData) {
    return props.formData[props.node.name];
  }
  if (props.node.default) {
    return resolveBinding(props.node.default, props.context);
  }
  return '';
});

const resolvedOptions = computed(() => {
  if (dynamicOptions.value) return dynamicOptions.value;
  return props.node.options || [];
});

function update(value) {
  if (props.formData) {
    props.formData[props.node.name] = value;
  }
}

function handleFile(event) {
  const file = event.target.files?.[0];
  if (file && props.formData) {
    props.formData[props.node.name] = file;
  }
}

onMounted(async () => {
  if (props.node.options_source) {
    try {
      const result = await soma.execute(props.node.options_source);
      const items = Array.isArray(result) ? result
        : result.rows || result.data || result.items || [];
      dynamicOptions.value = items.map(item => ({
        value: item.value || item.id || String(item),
        label: item.label || item.name || String(item),
      }));
    } catch {
      dynamicOptions.value = [];
    }
  }
});
</script>
