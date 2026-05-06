<template>
  <Teleport to="body">
    <div v-if="visible" class="soma-modal-overlay" @click.self="close">
      <div class="soma-modal">
        <header class="soma-modal-header">
          <h2>{{ node.title }}</h2>
          <button class="soma-modal-close" @click="close">&times;</button>
        </header>
        <div class="soma-modal-body">
          <SomaRenderer
            v-for="(child, i) in node.children"
            :key="i"
            :schema="child"
            :context="context"
            @action="$emit('action', $event)"
            @submit="handleSubmit"
            @navigate="$emit('navigate', $event)"
          />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { ref } from 'vue';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['action', 'submit', 'navigate']);

const visible = ref(false);

function open() {
  visible.value = true;
}

function close() {
  visible.value = false;
}

function handleSubmit(event) {
  emit('submit', event);
  close();
}

defineExpose({ open, close });
</script>
