// Thin wrappers around Web Speech API for STT and TTS.

export function sttAvailable() {
  return !!(globalThis.SpeechRecognition || globalThis.webkitSpeechRecognition);
}

export function startListening({ onResult, onError, lang = 'en-US' } = {}) {
  const Ctor = globalThis.SpeechRecognition || globalThis.webkitSpeechRecognition;
  if (!Ctor) {
    onError?.(new Error('SpeechRecognition not available'));
    return { stop() {} };
  }
  const rec = new Ctor();
  rec.lang = lang;
  rec.interimResults = false;
  rec.maxAlternatives = 1;
  rec.onresult = (e) => {
    const text = e.results[0]?.[0]?.transcript || '';
    onResult?.(text);
  };
  rec.onerror = (e) => onError?.(new Error(e.error || 'speech error'));
  rec.onend = () => {};
  rec.start();
  return { stop() { try { rec.stop(); } catch {} } };
}

export function ttsAvailable() {
  return !!globalThis.speechSynthesis;
}

export function speak(text) {
  if (!globalThis.speechSynthesis || !text) return;
  const utt = new SpeechSynthesisUtterance(text);
  utt.rate = 1.05;
  utt.pitch = 0.95;
  speechSynthesis.speak(utt);
}

export function stopSpeaking() {
  if (globalThis.speechSynthesis) speechSynthesis.cancel();
}
