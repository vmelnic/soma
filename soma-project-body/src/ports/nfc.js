// NFC port. read_tag → { records[] }, write_tag → { written }.
// Web NFC is Chrome-only (Android) — gracefully returns unsupported elsewhere.

export function nfcPort({ adapter } = {}) {
  const eff = adapter || browserNfcAdapter();
  return {
    manifest: {
      port_id: 'nfc',
      capabilities: [
        { capability_id: 'read_tag', input_schema: { properties: { timeout_ms: { type: 'number' } } } },
        { capability_id: 'write_tag', input_schema: { properties: { records: { type: 'array' } } } },
      ],
    },
    handler: async (capability, input) => {
      if (capability === 'read_tag') return eff.readTag(input || {});
      if (capability === 'write_tag') return eff.writeTag(input || {});
      throw new Error(`nfc: unknown capability '${capability}'`);
    },
  };
}

export function browserNfcAdapter() {
  return {
    async readTag({ timeout_ms = 5000 } = {}) {
      if (!('NDEFReader' in globalThis)) {
        return { error: 'unsupported', records: [] };
      }
      const reader = new NDEFReader();
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), timeout_ms);

      try {
        await reader.scan({ signal: ctrl.signal });
        const event = await new Promise((resolve, reject) => {
          reader.onreading = (e) => resolve(e);
          reader.onreadingerror = () => reject(new Error('nfc: reading error'));
          ctrl.signal.addEventListener('abort', () => reject(new Error('nfc: timeout')));
        });
        clearTimeout(timer);
        const records = Array.from(event.message.records).map((r) => ({
          record_type: r.recordType,
          media_type: r.mediaType,
          data: r.data ? new TextDecoder().decode(r.data) : null,
        }));
        return { records };
      } catch (e) {
        clearTimeout(timer);
        if (e.message === 'nfc: timeout') return { records: [], timeout: true };
        throw e;
      }
    },

    async writeTag({ records = [] } = {}) {
      if (!('NDEFReader' in globalThis)) {
        return { error: 'unsupported', written: false };
      }
      const writer = new NDEFReader();
      const ndefRecords = records.map((r) => ({
        recordType: r.record_type || 'text',
        data: r.data || '',
      }));
      await writer.write({ records: ndefRecords });
      return { written: true };
    },
  };
}
