// Haptics port. vibrate(ms) / impact(light|medium|heavy).

export function hapticsPort({ adapter } = {}) {
  const eff = adapter || browserHapticsAdapter();
  return {
    manifest: { port_id: 'haptics', capabilities: ['vibrate', 'impact'] },
    handler: async (capability, input) => {
      if (capability === 'vibrate') {
        const ms = Math.max(1, Math.min(2000, Number(input?.ms) || 50));
        await eff.vibrate(ms);
        return { ok: true, ms };
      }
      if (capability === 'impact') {
        const style = (input?.style === 'light' || input?.style === 'heavy') ? input.style : 'medium';
        await eff.impact(style);
        return { ok: true, style };
      }
      throw new Error(`haptics: unknown capability '${capability}'`);
    },
  };
}

export function browserHapticsAdapter() {
  return {
    async vibrate(ms) {
      if (typeof globalThis.navigator?.vibrate === 'function') navigator.vibrate(ms);
    },
    async impact(style) {
      const map = { light: 20, medium: 40, heavy: 60 };
      if (typeof globalThis.navigator?.vibrate === 'function') navigator.vibrate(map[style]);
    },
  };
}

export function capacitorHapticsAdapter() {
  return {
    async vibrate(ms) {
      const { Haptics } = await import('@capacitor/haptics');
      await Haptics.vibrate({ duration: ms });
    },
    async impact(style) {
      const { Haptics, ImpactStyle } = await import('@capacitor/haptics');
      const pick = { light: ImpactStyle.Light, medium: ImpactStyle.Medium, heavy: ImpactStyle.Heavy };
      await Haptics.impact({ style: pick[style] });
    },
  };
}
