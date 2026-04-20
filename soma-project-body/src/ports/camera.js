// Camera port. capture_image → { image_base64, mime, width?, height? }.
// Adapter pattern lets tests inject a mock without touching real hardware.

export function cameraPort({ adapter } = {}) {
  const eff = adapter || browserCameraAdapter();
  return {
    manifest: { port_id: 'camera', capabilities: ['capture_image'] },
    handler: async (capability, input) => {
      if (capability !== 'capture_image') {
        throw new Error(`camera: unknown capability '${capability}'`);
      }
      return eff.capture(input || {});
    },
  };
}

export function browserCameraAdapter() {
  return {
    async capture({ facingMode = 'environment' } = {}) {
      if (!globalThis.navigator?.mediaDevices) {
        throw new Error('camera: mediaDevices not available');
      }
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode },
      });
      const track = stream.getVideoTracks()[0];
      let blob;
      if (typeof globalThis.ImageCapture === 'function') {
        const ic = new ImageCapture(track);
        blob = await ic.takePhoto();
      } else {
        // Fallback: draw a frame onto a canvas.
        const video = document.createElement('video');
        video.srcObject = stream;
        await video.play();
        const canvas = document.createElement('canvas');
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        canvas.getContext('2d').drawImage(video, 0, 0);
        blob = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.9));
        video.pause();
      }
      track.stop();
      const base64 = await blobToBase64(blob);
      return { image_base64: base64, mime: blob.type || 'image/jpeg' };
    },
  };
}

export function capacitorCameraAdapter() {
  return {
    async capture({ quality = 80, source = 'camera' } = {}) {
      const { Camera } = await import('@capacitor/camera');
      const photo = await Camera.getPhoto({
        quality, resultType: 'base64', source,
      });
      return { image_base64: photo.base64String, mime: `image/${photo.format || 'jpeg'}` };
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
