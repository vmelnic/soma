import { inject, provide } from 'vue';

const SOMA_KEY = Symbol('soma-sdk');

/**
 * Provide a SomaSDK instance to the component tree.
 * Call this in the root component or app setup.
 */
export function provideSoma(sdk) {
  provide(SOMA_KEY, sdk);
}

/**
 * Inject the SomaSDK instance from the component tree.
 * Throws if no SDK was provided.
 */
export function useSoma() {
  const sdk = inject(SOMA_KEY);
  if (!sdk) {
    throw new Error('SomaSDK not provided. Call provideSoma(sdk) in a parent component.');
  }
  return sdk;
}
