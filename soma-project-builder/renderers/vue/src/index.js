export { default as SomaRenderer } from './SomaRenderer.vue';
export { default as SomaScreen } from './components/SomaScreen.vue';
export { default as SomaCard } from './components/SomaCard.vue';
export { default as SomaEntityList } from './components/SomaEntityList.vue';
export { default as SomaForm } from './components/SomaForm.vue';
export { default as SomaField } from './components/SomaField.vue';
export { default as SomaDetail } from './components/SomaDetail.vue';
export { default as SomaModal } from './components/SomaModal.vue';
export { default as SomaActionButton } from './components/SomaActionButton.vue';
export { default as SomaTabs } from './components/SomaTabs.vue';
export { default as SomaEmptyState } from './components/SomaEmptyState.vue';
export { default as SomaStatusBadge } from './components/SomaStatusBadge.vue';
export { default as SomaNav } from './components/SomaNav.vue';
export { provideSoma, useSoma } from './composables/useSoma.js';
export { resolveBinding, resolveBindMap } from './utils/bindings.js';

/**
 * Vue plugin: app.use(SomaPlugin, { sdk })
 */
export const SomaPlugin = {
  install(app, options = {}) {
    if (options.sdk) {
      app.provide(Symbol.for('soma-sdk-compat'), options.sdk);
    }
  },
};
