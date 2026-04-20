// Notifications port. send_local → { shown }, request_permission → { granted }.

export function notificationsPort({ adapter } = {}) {
  const eff = adapter || browserNotificationsAdapter();
  return {
    manifest: {
      port_id: 'notifications',
      capabilities: [
        { capability_id: 'send_local', input_schema: { properties: { title: { type: 'string' }, body: { type: 'string' } } } },
        'request_permission',
      ],
    },
    handler: async (capability, input) => {
      if (capability === 'send_local') return eff.sendLocal(input || {});
      if (capability === 'request_permission') return eff.requestPermission();
      throw new Error(`notifications: unknown capability '${capability}'`);
    },
  };
}

export function browserNotificationsAdapter() {
  return {
    async requestPermission() {
      if (!('Notification' in globalThis)) {
        return { granted: false, reason: 'Notification API unavailable' };
      }
      const result = await Notification.requestPermission();
      return { granted: result === 'granted' };
    },
    async sendLocal({ title = 'SOMA', body = '' } = {}) {
      if (!('Notification' in globalThis)) {
        throw new Error('notifications: Notification API unavailable');
      }
      if (Notification.permission !== 'granted') {
        const result = await Notification.requestPermission();
        if (result !== 'granted') {
          return { shown: false, reason: 'permission denied' };
        }
      }
      new Notification(title, { body });
      return { shown: true };
    },
  };
}

export function capacitorNotificationsAdapter() {
  const mod = '@capacitor/local-notifications';
  return {
    async requestPermission() {
      const { LocalNotifications } = await import(/* @vite-ignore */ mod);
      const result = await LocalNotifications.requestPermissions();
      return { granted: result.display === 'granted' };
    },
    async sendLocal({ title = 'SOMA', body = '' } = {}) {
      const { LocalNotifications } = await import(/* @vite-ignore */ mod);
      await LocalNotifications.schedule({
        notifications: [{
          title,
          body,
          id: Date.now() % 2147483647,
          schedule: { at: new Date() },
        }],
      });
      return { shown: true };
    },
  };
}
