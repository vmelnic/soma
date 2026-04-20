// Geolocation port. current_position → { lat, lon, accuracy_m, ts }.

export function geoPort({ adapter } = {}) {
  const eff = adapter || browserGeoAdapter();
  return {
    manifest: { port_id: 'geo', capabilities: ['current_position'] },
    handler: async (capability, input) => {
      if (capability !== 'current_position') {
        throw new Error(`geo: unknown capability '${capability}'`);
      }
      return eff.currentPosition(input || {});
    },
  };
}

export function browserGeoAdapter() {
  return {
    currentPosition({ timeout_ms = 10000, high_accuracy = true } = {}) {
      if (!globalThis.navigator?.geolocation) {
        return Promise.reject(new Error('geo: navigator.geolocation unavailable'));
      }
      return new Promise((resolve, reject) => {
        navigator.geolocation.getCurrentPosition(
          (pos) => resolve({
            lat: pos.coords.latitude,
            lon: pos.coords.longitude,
            accuracy_m: pos.coords.accuracy,
            ts: pos.timestamp,
          }),
          (err) => reject(new Error(`geo: ${err.message}`)),
          { enableHighAccuracy: high_accuracy, timeout: timeout_ms },
        );
      });
    },
  };
}

export function capacitorGeoAdapter() {
  return {
    async currentPosition({ high_accuracy = true } = {}) {
      const { Geolocation } = await import('@capacitor/geolocation');
      const pos = await Geolocation.getCurrentPosition({ enableHighAccuracy: high_accuracy });
      return {
        lat: pos.coords.latitude,
        lon: pos.coords.longitude,
        accuracy_m: pos.coords.accuracy,
        ts: pos.timestamp,
      };
    },
  };
}
