<template>
  <div class="soma-tabs">
    <div class="soma-tabs-header">
      <button
        v-for="tab in node.items"
        :key="tab.id"
        class="soma-tabs-tab"
        :class="{ 'soma-tabs-tab-active': activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </div>
    <div class="soma-tabs-body">
      <template v-for="tab in node.items" :key="tab.id">
        <div v-show="activeTab === tab.id" class="soma-tabs-panel">
          <SomaRenderer
            v-for="(child, i) in tab.children"
            :key="i"
            :schema="child"
            :context="context"
            @action="handleAction"
            @submit="handleSubmit"
            @navigate="handleNavigate"
          />
        </div>
      </template>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import SomaRenderer from '../SomaRenderer.vue';

const props = defineProps({
  node: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['action', 'submit', 'navigate']);

const activeTab = ref(props.node.items?.[0]?.id || '');
const tabIds = new Set((props.node.items || []).map(t => t.id));

function switchIfTab(target) {
  if (target && tabIds.has(target)) {
    activeTab.value = target;
    return true;
  }
  return false;
}

function handleSubmit(event) {
  if (!event.error && event.on_success) {
    switchIfTab(event.on_success);
  }
  if (event.error && event.on_failure) {
    switchIfTab(event.on_failure);
  }
  emit('submit', event);
}

function handleAction(event) {
  if (!event.error && event.on_success) {
    switchIfTab(event.on_success);
  }
  if (event.error && event.on_failure) {
    switchIfTab(event.on_failure);
  }
  emit('action', event);
}

function handleNavigate(target) {
  if (!switchIfTab(target)) {
    emit('navigate', target);
  }
}
</script>
