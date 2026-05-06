<template>
  <div class="max-w-lg mx-auto p-4">
    <h1 class="text-2xl font-bold mb-4">Book Appointment</h1>

    <form @submit.prevent="book" class="space-y-4">
      <div>
        <label class="block text-sm font-medium mb-1">Provider</label>
        <select v-model="form.provider_id" required
          class="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none">
          <option value="" disabled>Select a provider...</option>
          <option v-for="p in providers" :key="p.id" :value="p.id">{{ p.name }} — {{ p.bio }}</option>
        </select>
      </div>
      <div>
        <label class="block text-sm font-medium mb-1">Service</label>
        <input v-model="form.service" required placeholder="e.g. Hair Styling, Plumbing..."
          class="w-full px-3 py-2 border rounded-lg" />
      </div>
      <div class="grid grid-cols-2 gap-4">
        <div>
          <label class="block text-sm font-medium mb-1">Start</label>
          <input v-model="form.start_time" type="datetime-local" required class="w-full px-3 py-2 border rounded-lg" />
        </div>
        <div>
          <label class="block text-sm font-medium mb-1">End</label>
          <input v-model="form.end_time" type="datetime-local" required class="w-full px-3 py-2 border rounded-lg" />
        </div>
      </div>
      <div>
        <label class="block text-sm font-medium mb-1">Location</label>
        <input v-model="form.location" placeholder="Address or meeting point" class="w-full px-3 py-2 border rounded-lg" />
      </div>
      <div class="grid grid-cols-2 gap-4">
        <div>
          <label class="block text-sm font-medium mb-1">Rate (EUR)</label>
          <input v-model="form.rate_amount" type="number" step="0.01" placeholder="0.00" class="w-full px-3 py-2 border rounded-lg" />
        </div>
        <div>
          <label class="block text-sm font-medium mb-1">Rate Type</label>
          <select v-model="form.rate_type" class="w-full px-3 py-2 border rounded-lg">
            <option value="hourly">Hourly</option>
            <option value="fixed">Fixed</option>
            <option value="negotiable">Negotiable</option>
          </select>
        </div>
      </div>
      <div>
        <label class="block text-sm font-medium mb-1">Notes</label>
        <input v-model="form.notes" placeholder="Any special requests..." class="w-full px-3 py-2 border rounded-lg" />
      </div>
      <p v-if="error" class="text-red-600 text-sm">{{ error }}</p>
      <button type="submit" :disabled="loading"
        class="w-full py-2 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50">
        {{ loading ? 'Booking...' : 'Book Now' }}
      </button>
    </form>
  </div>
</template>

<script setup>
import { ref, reactive, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { routine } from '../api.js';
import { useAuth } from '../composables/useAuth.js';

const router = useRouter();
const { token } = useAuth();
const providers = ref([]);
const loading = ref(false);
const error = ref('');
const form = reactive({
  provider_id: '',
  service: '',
  start_time: '',
  end_time: '',
  location: '',
  rate_amount: '',
  rate_type: 'hourly',
  notes: '',
});

onMounted(async () => {
  try {
    const r = await routine('list_providers', {});
    providers.value = r.rows || r || [];
  } catch { /* empty */ }
});

async function book() {
  loading.value = true;
  error.value = '';
  try {
    await routine('book_appointment', { token: token.value, ...form });
    router.push('/appointments');
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}
</script>
