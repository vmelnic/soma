// Filesystem port. On native: @capacitor/filesystem; on browser: an
// IndexedDB-backed key→bytes store scoped to the PWA origin. Uniform wire
// shape so routines work identically on phone and desktop.

export function filesystemPort({ adapter } = {}) {
  const eff = adapter || browserFilesystemAdapter();
  return {
    manifest: { port_id: 'filesystem', capabilities: ['read', 'write', 'list', 'delete'] },
    handler: async (capability, input) => {
      const path = String(input?.path || '').trim();
      if (!path && capability !== 'list') throw new Error('filesystem: path required');
      switch (capability) {
        case 'read':   return { data: await eff.read(path), encoding: 'base64' };
        case 'write':  return { ok: true, bytes: await eff.write(path, input?.data ?? '') };
        case 'delete': return { ok: await eff.remove(path) };
        case 'list':   return { entries: await eff.list(path || '/') };
        default:       throw new Error(`filesystem: unknown capability '${capability}'`);
      }
    },
  };
}

export function browserFilesystemAdapter({ dbName = 'soma-fs' } = {}) {
  const open = () =>
    new Promise((resolve, reject) => {
      const req = indexedDB.open(dbName, 1);
      req.onupgradeneeded = () => {
        req.result.createObjectStore('files');
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });

  const tx = (mode, fn) =>
    open().then((db) =>
      new Promise((resolve, reject) => {
        const t = db.transaction('files', mode);
        const store = t.objectStore('files');
        const result = fn(store);
        t.oncomplete = () => resolve(result);
        t.onerror = () => reject(t.error);
      }),
    );

  return {
    async read(path) {
      return new Promise((resolve, reject) => {
        open().then((db) => {
          const r = db.transaction('files').objectStore('files').get(path);
          r.onsuccess = () => resolve(r.result || '');
          r.onerror = () => reject(r.error);
        });
      });
    },
    async write(path, data) {
      await tx('readwrite', (s) => s.put(String(data ?? ''), path));
      return String(data ?? '').length;
    },
    async remove(path) {
      await tx('readwrite', (s) => s.delete(path));
      return true;
    },
    async list(prefix) {
      return new Promise((resolve, reject) => {
        open().then((db) => {
          const out = [];
          const cur = db.transaction('files').objectStore('files').openKeyCursor();
          cur.onsuccess = () => {
            const c = cur.result;
            if (!c) return resolve(out);
            const k = String(c.key);
            if (k.startsWith(prefix)) out.push(k);
            c.continue();
          };
          cur.onerror = () => reject(cur.error);
        });
      });
    },
  };
}

export function capacitorFilesystemAdapter() {
  return {
    async read(path) {
      const { Filesystem, Directory } = await import('@capacitor/filesystem');
      const r = await Filesystem.readFile({ path, directory: Directory.Data });
      return r.data; // base64
    },
    async write(path, data) {
      const { Filesystem, Directory } = await import('@capacitor/filesystem');
      await Filesystem.writeFile({ path, data: String(data ?? ''), directory: Directory.Data });
      return String(data ?? '').length;
    },
    async remove(path) {
      const { Filesystem, Directory } = await import('@capacitor/filesystem');
      await Filesystem.deleteFile({ path, directory: Directory.Data });
      return true;
    },
    async list(path) {
      const { Filesystem, Directory } = await import('@capacitor/filesystem');
      const r = await Filesystem.readdir({ path, directory: Directory.Data });
      return r.files.map((f) => (typeof f === 'string' ? f : f.name));
    },
  };
}
