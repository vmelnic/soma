// Register all local ports (browser-adapters by default) with a SomaClient
// and announce them to the runtime via reverse/register_ports.

import { cameraPort, browserCameraAdapter, capacitorCameraAdapter } from './camera.js';
import { geoPort, browserGeoAdapter, capacitorGeoAdapter } from './geo.js';
import { hapticsPort, browserHapticsAdapter, capacitorHapticsAdapter } from './haptics.js';
import { clipboardPort, browserClipboardAdapter } from './clipboard.js';
import {
  filesystemPort, browserFilesystemAdapter, capacitorFilesystemAdapter,
} from './filesystem.js';
import { micPort, browserMicAdapter } from './mic.js';
import { notificationsPort, browserNotificationsAdapter, capacitorNotificationsAdapter } from './notifications.js';
import { nfcPort, browserNfcAdapter } from './nfc.js';
import { ocrPort, browserOcrAdapter } from './ocr.js';
import { detectPlatform } from './platform.js';

export { cameraPort, geoPort, hapticsPort, clipboardPort, filesystemPort };
export { micPort, notificationsPort, nfcPort, ocrPort };

export async function buildAllPorts(platform) {
  const p = platform || (await detectPlatform());
  const native = p === 'capacitor';
  return [
    cameraPort({        adapter: native ? capacitorCameraAdapter()        : browserCameraAdapter()        }),
    geoPort({           adapter: native ? capacitorGeoAdapter()           : browserGeoAdapter()           }),
    hapticsPort({       adapter: native ? capacitorHapticsAdapter()       : browserHapticsAdapter()       }),
    clipboardPort({     adapter: browserClipboardAdapter()                                                }),
    filesystemPort({    adapter: native ? capacitorFilesystemAdapter()    : browserFilesystemAdapter()    }),
    micPort({           adapter: browserMicAdapter()                                                      }),
    notificationsPort({ adapter: native ? capacitorNotificationsAdapter() : browserNotificationsAdapter() }),
    nfcPort({           adapter: browserNfcAdapter()                                                      }),
    ocrPort({           adapter: browserOcrAdapter()                                                      }),
  ];
}

export async function registerAllPorts(client, deviceId, opts = {}) {
  const ports = opts.ports || (await buildAllPorts(opts.platform));
  for (const p of ports) {
    client.registerLocalPort(p.manifest.port_id, p.manifest.capabilities, p.handler);
  }
  const manifests = ports.map((p) => p.manifest);
  await client.announceLocalPorts(deviceId, manifests);
  return { count: ports.length, port_ids: manifests.map((m) => m.port_id) };
}
