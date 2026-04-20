import { createRouter, createWebHashHistory } from 'vue-router';

import Talk from './views/Talk.vue';
import Work from './views/Work.vue';
import Apps from './views/Apps.vue';
import Devices from './views/Devices.vue';

export default createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/talk' },
    { path: '/talk', name: 'talk', component: Talk },
    { path: '/work', name: 'work', component: Work },
    { path: '/apps', name: 'apps', component: Apps },
    { path: '/devices', name: 'devices', component: Devices },
  ],
});
