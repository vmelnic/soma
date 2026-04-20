// OCR port. extract_text → { text, confidence }.
// Uses Tesseract.js in browser (lazy-loaded to avoid large bundle impact).

let tesseractPromise = null;

function getTesseract() {
  if (!tesseractPromise) {
    // Dynamic import with variable to prevent bundler from resolving at build time.
    const mod = 'tesseract.js';
    tesseractPromise = import(/* @vite-ignore */ mod).catch(() => null);
  }
  return tesseractPromise;
}

export function ocrPort({ adapter } = {}) {
  const eff = adapter || browserOcrAdapter();
  return {
    manifest: {
      port_id: 'ocr',
      capabilities: [
        {
          capability_id: 'extract_text',
          input_schema: {
            properties: {
              image_base64: { type: 'string' },
              language: { type: 'string' },
            },
          },
        },
      ],
    },
    handler: async (capability, input) => {
      if (capability !== 'extract_text') {
        throw new Error(`ocr: unknown capability '${capability}'`);
      }
      return eff.extractText(input || {});
    },
  };
}

export function browserOcrAdapter() {
  return {
    async extractText({ image_base64, language = 'eng' } = {}) {
      if (!image_base64) throw new Error('ocr: image_base64 required');
      const Tesseract = await getTesseract();
      if (!Tesseract) {
        throw new Error('ocr: tesseract.js not installed — run npm install tesseract.js');
      }
      const dataUrl = image_base64.startsWith('data:')
        ? image_base64
        : `data:image/png;base64,${image_base64}`;
      const result = await Tesseract.recognize(dataUrl, language);
      return {
        text: result.data.text,
        confidence: result.data.confidence,
      };
    },
  };
}
