<template>
  <div class="max-w-2xl mx-auto p-4">
    <h1 class="text-2xl font-bold mb-4">Contacts</h1>

    <p v-if="loading" class="text-center py-8 text-gray-500">Loading...</p>
    <div v-else-if="items.length" class="space-y-3">
      <div v-for="c in items" :key="c.id" class="border rounded-lg p-4 flex justify-between items-center">
        <div class="cursor-pointer" @click="$router.push({ name: 'profile', params: { id: c.id } })">
          <p class="font-semibold">{{ c.name }}</p>
          <p class="text-sm text-gray-600">{{ c.phone }}</p>
          <span v-if="c.role" class="text-xs font-medium px-2 py-0.5 rounded-full"
            :class="c.role === 'provider' ? 'bg-green-100 text-green-800' : 'bg-blue-100 text-blue-800'">
            {{ c.role }}
          </span>
        </div>
      </div>
    </div>
    <div v-else class="text-center py-12">
      <p class="text-gray-700 font-medium">No contacts yet</p>
      <p class="text-sm text-gray-500">Connect with service providers to add them to your contacts.</p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { routine } from '../api.js';
import { useAuth } from '../composables/useAuth.js';

const { token } = useAuth();
const items = ref([]);
const loading = ref(true);

onMounted(async () => {
  try {
    const r = await routine('list_contacts', { token: token.value });
    items.value = r.rows || r || [];
  } catch { /* empty */ }
  loading.value = false;
});
</script>
