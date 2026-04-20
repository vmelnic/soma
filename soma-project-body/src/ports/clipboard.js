// Clipboard port. read → { text }; write({text}) → { ok }.

export function clipboardPort({ adapter } = {}) {
  const eff = adapter || browserClipboardAdapter();
  return {
    manifest: { port_id: 'clipboard', capabilities: ['read', 'write'] },
    handler: async (capability, input) => {
      if (capability === 'read') return { text: await eff.read() };
      if (capability === 'write') {
        const text = String(input?.text ?? '');
        await eff.write(text);
        return { ok: true, bytes: text.length };
      }
      throw new Error(`clipboard: unknown capability '${capability}'`);
    },
  };
}

export function browserClipboardAdapter() {
  return {
    async read() {
      if (!globalThis.navigator?.clipboard?.readText) throw new Error('clipboard read unavailable');
      return navigator.clipboard.readText();
    },
    async write(text) {
      if (!globalThis.navigator?.clipboard?.writeText) throw new Error('clipboard write unavailable');
      return navigator.clipboard.writeText(text);
    },
  };
}
