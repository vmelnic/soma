import { ref, computed } from 'vue';

const token = ref(localStorage.getItem('hb_token') || '');
const userId = ref(localStorage.getItem('hb_user_id') || '');

export function useAuth() {
  const isLoggedIn = computed(() => !!token.value);

  function login(t, uid) {
    token.value = t;
    userId.value = uid;
    localStorage.setItem('hb_token', t);
    localStorage.setItem('hb_user_id', uid);
  }

  function logout() {
    token.value = '';
    userId.value = '';
    localStorage.removeItem('hb_token');
    localStorage.removeItem('hb_user_id');
  }

  return { token, userId, isLoggedIn, login, logout };
}
