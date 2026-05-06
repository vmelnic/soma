<template>
  <div class="max-w-lg mx-auto p-4">
    <h1 class="text-2xl font-bold mb-4">{{ isOwnProfile ? 'My Profile' : 'Profile' }}</h1>

    <p v-if="loading" class="text-center py-8 text-gray-500">Loading...</p>
    <div v-else-if="user" class="space-y-4">
      <div class="border rounded-lg p-4 space-y-2">
        <p class="text-lg font-semibold">{{ user.name }}</p>
        <p v-if="user.bio" class="text-gray-600">{{ user.bio }}</p>
      </div>

      <div class="grid grid-cols-2 gap-x-4 gap-y-2 border rounded-lg p-4">
        <span class="text-sm font-medium text-gray-500">Phone</span>
        <span class="text-sm">{{ user.phone }}</span>
        <span class="text-sm font-medium text-gray-500">Email</span>
        <span class="text-sm">{{ user.email || '—' }}</span>
        <span class="text-sm font-medium text-gray-500">Role</span>
        <span class="text-sm">{{ user.role }}</span>
        <span class="text-sm font-medium text-gray-500">Verified</span>
        <span class="text-sm">{{ user.is_verified ? 'Yes' : 'No' }}</span>
        <span class="text-sm font-medium text-gray-500">Plan</span>
        <span class="text-sm">{{ user.subscription_plan }}</span>
        <span class="text-sm font-medium text-gray-500">Member since</span>
        <span class="text-sm">{{ user.created_at ? new Date(user.created_at).toLocaleDateString() : '—' }}</span>
      </div>

      <button v-if="isOwnProfile" @click="doLogout"
        class="w-full py-2 bg-red-600 text-white rounded-lg font-medium hover:bg-red-700">
        Sign Out
      </button>

      <router-link v-if="!isOwnProfile" to="/book"
        class="block w-full py-2 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 text-center">
        Book Appointment
      </router-link>
    </div>
    <p v-else class="text-center py-8 text-gray-500">User not found.</p>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { routine } from '../api.js';
import { useAuth } from '../composables/useAuth.js';

const route = useRoute();
const router = useRouter();
const { token, userId, logout } = useAuth();

const user = ref(null);
const loading = ref(true);
const profileId = computed(() => route.params.id || userId.value);
const isOwnProfile = computed(() => profileId.value === userId.value);

async function load() {
  loading.value = true;
  try {
    const r = await routine('get_user_profile', { token: token.value, id: profileId.value });
    user.value = r.row || r || null;
  } catch { user.value = null; }
  loading.value = false;
}

function doLogout() {
  logout();
  router.push('/login');
}

onMounted(load);
watch(() => route.params.id, load);
</script>
