<template>
  <component
    :is="componentFor(schema.type)"
    :node="schema"
    :context="context"
    :form-data="formData"
    :active-screen="activeScreen"
    @action="$emit('action', $event)"
    @submit="$emit('submit', $event)"
    @navigate="$emit('navigate', $event)"
  />
</template>

<script setup>
import SomaScreen from './components/SomaScreen.vue';
import SomaCard from './components/SomaCard.vue';
import SomaEntityList from './components/SomaEntityList.vue';
import SomaForm from './components/SomaForm.vue';
import SomaField from './components/SomaField.vue';
import SomaDetail from './components/SomaDetail.vue';
import SomaModal from './components/SomaModal.vue';
import SomaActionButton from './components/SomaActionButton.vue';
import SomaTabs from './components/SomaTabs.vue';
import SomaEmptyState from './components/SomaEmptyState.vue';
import SomaStatusBadge from './components/SomaStatusBadge.vue';
import SomaNav from './components/SomaNav.vue';

defineProps({
  schema: { type: Object, required: true },
  context: { type: Object, default: () => ({}) },
  formData: { type: Object, default: undefined },
  activeScreen: { type: String, default: '' },
});

defineEmits(['action', 'submit', 'navigate']);

const typeMap = {
  screen: SomaScreen,
  card: SomaCard,
  entity_list: SomaEntityList,
  form: SomaForm,
  'field:text': SomaField,
  'field:datetime': SomaField,
  'field:select': SomaField,
  'field:image': SomaField,
  'field:number': SomaField,
  'field:phone': SomaField,
  detail: SomaDetail,
  modal: SomaModal,
  action_button: SomaActionButton,
  tabs: SomaTabs,
  empty_state: SomaEmptyState,
  status_badge: SomaStatusBadge,
  nav: SomaNav,
};

function componentFor(type) {
  return typeMap[type] || SomaEmptyState;
}
</script>
