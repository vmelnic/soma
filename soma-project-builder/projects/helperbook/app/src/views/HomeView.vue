<template>
  <div class="max-w-2xl mx-auto p-4">
    <h1 class="text-2xl font-bold mb-4">HelperBook</h1>

    <div class="flex border-b mb-4">
      <button v-for="t in tabs" :key="t" @click="tab = t"
        class="px-4 py-2 text-sm font-medium border-b-2 transition-colors"
        :class="tab === t ? 'border-blue-600 text-blue-600' : 'border-transparent text-gray-500'">
        {{ t }}
      </button>
    </div>

    <!-- Upcoming appointments -->
    <div v-if="tab === 'Upcoming'">
      <p v-if="loadingAppts" class="text-center py-8 text-gray-500">Loading...</p>
      <div v-else-if="appointments.length" class="space-y-3">
        <div v-for="a in appointments" :key="a.id" class="border rounded-lg p-4 hover:shadow-md cursor-pointer"
          @click="$router.push({ name: 'profile', params: { id: a.provider_id } })">
          <div class="flex justify-between items-start">
            <div>
              <p class="font-semibold">{{ a.service }}</p>
              <p class="text-sm text-gray-600">{{ fmt(a.start_time) }}</p>
            </div>
            <span class="text-xs font-medium px-2 py-0.5 rounded-full" :class="badgeClass(a.status)">
              {{ a.status }}
            </span>
          </div>
        </div>
      </div>
      <div v-else class="text-center py-12">
        <p class="text-gray-700 font-medium">No upcoming appointments</p>
        <p class="text-sm text-gray-500 mb-4">Book your first appointment to get started.</p>
        <router-link to="/book" class="text-blue-600 font-medium">Find a provider</router-link>
      </div>
    </div>

    <!-- Providers -->
    <div v-if="tab === 'Providers'">
      <p v-if="loadingProviders" class="text-center py-8 text-gray-500">Loading...</p>
      <div v-else-if="providers.length" class="space-y-3">
        <div v-for="p in providers" :key="p.id" class="border rounded-lg p-4 hover:shadow-md cursor-pointer"
          @click="$router.push({ name: 'profile', params: { id: p.id } })">
          <p class="font-semibold">{{ p.name }}</p>
          <p class="text-sm text-gray-600">{{ p.bio }}</p>
        </div>
      </div>
      <p v-else class="text-center py-12 text-gray-500">No providers found.</p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue';
import { routine } from '../api.js';
import { useAuth } from '../composables/useAuth.js';

const { token } = useAuth();
const tabs = ['Upcoming', 'Providers'];
const tab = ref('Upcoming');

const appointments = ref([]);
const providers = ref([]);
const loadingAppts = ref(true);
const loadingProviders = ref(true);

function fmt(ts) {
  if (!ts) return '';
  return new Date(ts).toLocaleString();
}

function badgeClass(status) {
  const m = { proposed: 'bg-blue-100 text-blue-800', confirmed: 'bg-green-100 text-green-800', in_progress: 'bg-yellow-100 text-yellow-800', completed: 'bg-green-100 text-green-800', cancelled: 'bg-red-100 text-red-800', dismissed: 'bg-gray-100 text-gray-800', no_show: 'bg-red-100 text-red-800' };
  return m[status] || 'bg-gray-100 text-gray-800';
}

onMounted(async () => {
  try {
    const r = await routine('list_appointments', { token: token.value });
    appointments.value = r.rows || r || [];
  } catch { /* empty */ }
  loadingAppts.value = false;

  try {
    const r = await routine('list_providers', {});
    providers.value = r.rows || r || [];
  } catch { /* empty */ }
  loadingProviders.value = false;
});
</script>
