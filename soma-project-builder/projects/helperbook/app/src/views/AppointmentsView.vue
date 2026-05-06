<template>
  <div class="max-w-2xl mx-auto p-4">
    <div class="flex justify-between items-center mb-4">
      <h1 class="text-2xl font-bold">My Appointments</h1>
      <router-link to="/book" class="px-4 py-2 bg-blue-600 text-white rounded-lg text-sm font-medium hover:bg-blue-700">
        Book New
      </router-link>
    </div>

    <p v-if="loading" class="text-center py-8 text-gray-500">Loading...</p>
    <div v-else-if="items.length" class="space-y-3">
      <div v-for="a in items" :key="a.id" class="border rounded-lg p-4">
        <div class="flex justify-between items-start">
          <div>
            <p class="font-semibold">{{ a.service }}</p>
            <p class="text-sm text-gray-600">{{ fmt(a.start_time) }} — {{ fmt(a.end_time) }}</p>
          </div>
          <span class="text-xs font-medium px-2 py-0.5 rounded-full" :class="badgeClass(a.status)">
            {{ a.status }}
          </span>
        </div>
        <div v-if="canCancel(a.status)" class="mt-3">
          <button @click="cancel(a.id)" :disabled="cancelling === a.id"
            class="text-sm text-red-600 hover:text-red-800 font-medium">
            {{ cancelling === a.id ? 'Cancelling...' : 'Cancel' }}
          </button>
        </div>
      </div>
    </div>
    <div v-else class="text-center py-12">
      <p class="text-gray-700 font-medium">No appointments yet</p>
      <router-link to="/book" class="text-blue-600 font-medium">Browse providers</router-link>
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
const cancelling = ref(null);

function fmt(ts) { return ts ? new Date(ts).toLocaleString() : ''; }
function canCancel(s) { return ['proposed', 'confirmed'].includes(s); }
function badgeClass(status) {
  const m = { proposed: 'bg-blue-100 text-blue-800', confirmed: 'bg-green-100 text-green-800', in_progress: 'bg-yellow-100 text-yellow-800', completed: 'bg-green-100 text-green-800', cancelled: 'bg-red-100 text-red-800', dismissed: 'bg-gray-100 text-gray-800', no_show: 'bg-red-100 text-red-800' };
  return m[status] || 'bg-gray-100 text-gray-800';
}

async function load() {
  loading.value = true;
  try {
    const r = await routine('list_appointments', { token: token.value });
    items.value = r.rows || r || [];
  } catch { /* empty */ }
  loading.value = false;
}

async function cancel(id) {
  if (!confirm('Cancel this appointment?')) return;
  cancelling.value = id;
  try {
    await routine('cancel_appointment', { token: token.value, appointment_id: id });
    await load();
  } catch { /* empty */ }
  cancelling.value = null;
}

onMounted(load);
</script>
