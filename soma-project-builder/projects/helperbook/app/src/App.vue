<template>
  <div class="min-h-screen bg-gray-50 flex flex-col">
    <router-view class="flex-1" />
    <nav v-if="isLoggedIn" class="flex justify-around border-t bg-white py-2 sticky bottom-0">
      <router-link to="/" class="nav-item" active-class="text-blue-600">
        <span>🏠</span><span class="text-xs">Home</span>
      </router-link>
      <router-link to="/appointments" class="nav-item" active-class="text-blue-600">
        <span>📅</span><span class="text-xs">Appointments</span>
      </router-link>
      <router-link to="/contacts" class="nav-item" active-class="text-blue-600">
        <span>👤</span><span class="text-xs">Contacts</span>
      </router-link>
      <router-link :to="{ name: 'profile' }" class="nav-item" active-class="text-blue-600">
        <span>⚙️</span><span class="text-xs">Profile</span>
      </router-link>
    </nav>
  </div>
</template>

<script setup>
import { useAuth } from './composables/useAuth.js';
const { isLoggedIn } = useAuth();
</script>

<style>
.nav-item {
  @apply flex flex-col items-center gap-1 text-gray-500 hover:text-blue-600 transition-colors;
}
</style>
