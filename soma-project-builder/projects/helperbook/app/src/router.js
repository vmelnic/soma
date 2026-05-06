import { createRouter, createWebHistory } from 'vue-router';
import { useAuth } from './composables/useAuth.js';

import LoginView from './views/LoginView.vue';
import HomeView from './views/HomeView.vue';
import AppointmentsView from './views/AppointmentsView.vue';
import BookView from './views/BookView.vue';
import ContactsView from './views/ContactsView.vue';
import ProfileView from './views/ProfileView.vue';

const routes = [
  { path: '/login', name: 'login', component: LoginView, meta: { guest: true } },
  { path: '/', name: 'home', component: HomeView },
  { path: '/appointments', name: 'appointments', component: AppointmentsView },
  { path: '/book', name: 'book', component: BookView },
  { path: '/contacts', name: 'contacts', component: ContactsView },
  { path: '/profile/:id?', name: 'profile', component: ProfileView },
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

router.beforeEach((to) => {
  const { isLoggedIn } = useAuth();
  if (!to.meta.guest && !isLoggedIn.value) return { name: 'login' };
  if (to.meta.guest && isLoggedIn.value) return { name: 'home' };
});

export default router;
