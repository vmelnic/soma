// Platform detection helper. Returns 'capacitor' on native mobile (iOS/Android
// built app), 'browser' on a plain web page / PWA. Tests can override.

let forcedPlatform = null;

export function setPlatformForTesting(p) {
  forcedPlatform = p;
}

export async function detectPlatform() {
  if (forcedPlatform) return forcedPlatform;
  try {
    const mod = await import('@capacitor/core');
    if (mod?.Capacitor?.isNativePlatform?.()) return 'capacitor';
  } catch { /* not bundled → browser */ }
  return 'browser';
}
