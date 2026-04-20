// Microphone port. record_audio → { audio_base64, mime, duration_ms }.

export function micPort({ adapter } = {}) {
  const eff = adapter || browserMicAdapter();
  return {
    manifest: { port_id: 'mic', capabilities: ['record_audio'] },
    handler: async (capability, input) => {
      if (capability !== 'record_audio') {
        throw new Error(`mic: unknown capability '${capability}'`);
      }
      return eff.record(input || {});
    },
  };
}

export function browserMicAdapter() {
  return {
    async record({ duration_ms = 3000, mime_type } = {}) {
      if (!globalThis.navigator?.mediaDevices) {
        throw new Error('mic: mediaDevices not available');
      }
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const preferred = mime_type || (MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : 'audio/webm');
      const recorder = new MediaRecorder(stream, { mimeType: preferred });
      const chunks = [];
      recorder.ondataavailable = (e) => { if (e.data.size) chunks.push(e.data); };

      const done = new Promise((resolve, reject) => {
        recorder.onstop = () => resolve();
        recorder.onerror = (e) => reject(e.error || new Error('mic: recording error'));
      });

      recorder.start();
      await new Promise((r) => setTimeout(r, duration_ms));
      recorder.stop();
      stream.getTracks().forEach((t) => t.stop());
      await done;

      const blob = new Blob(chunks, { type: recorder.mimeType });
      const base64 = await blobToBase64(blob);
      return { audio_base64: base64, mime: recorder.mimeType, duration_ms };
    },
  };
}

function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      const s = r.result;
      resolve(typeof s === 'string' ? s.split(',')[1] || s : '');
    };
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });
}
