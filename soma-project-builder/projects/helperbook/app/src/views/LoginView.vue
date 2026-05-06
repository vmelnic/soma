<template>
  <div class="max-w-sm mx-auto p-6 mt-20">
    <h1 class="text-2xl font-bold mb-6 text-center">HelperBook</h1>

    <form v-if="!codeSent" @submit.prevent="sendCode" class="space-y-4">
      <div>
        <label class="block text-sm font-medium mb-1">Phone</label>
        <input v-model="phone" type="tel" required placeholder="+373 XX XXX XXX"
          class="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none" />
      </div>
      <div>
        <label class="block text-sm font-medium mb-1">Email</label>
        <input v-model="email" type="email" required placeholder="you@example.com"
          class="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none" />
      </div>
      <p v-if="error" class="text-red-600 text-sm">{{ error }}</p>
      <button type="submit" :disabled="loading"
        class="w-full py-2 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50">
        {{ loading ? 'Sending...' : 'Send Code' }}
      </button>
    </form>

    <form v-else @submit.prevent="verify" class="space-y-4">
      <p class="text-sm text-gray-600">Code sent to {{ email }}</p>
      <div>
        <label class="block text-sm font-medium mb-1">Verification Code</label>
        <input v-model="code" type="text" required placeholder="6-digit code"
          class="w-full px-3 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none text-center text-2xl tracking-widest" />
      </div>
      <p v-if="error" class="text-red-600 text-sm">{{ error }}</p>
      <button type="submit" :disabled="loading"
        class="w-full py-2 bg-blue-600 text-white rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50">
        {{ loading ? 'Verifying...' : 'Verify' }}
      </button>
      <button type="button" @click="codeSent = false" class="w-full text-sm text-gray-500 hover:text-gray-700">
        Back
      </button>
    </form>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import { useRouter } from 'vue-router';
import { routine } from '../api.js';
import { useAuth } from '../composables/useAuth.js';

const router = useRouter();
const { login } = useAuth();

const phone = ref('');
const email = ref('');
const code = ref('');
const codeSent = ref(false);
const loading = ref(false);
const error = ref('');

async function sendCode() {
  loading.value = true;
  error.value = '';
  try {
    await routine('login_otp', { phone: phone.value, email: email.value });
    codeSent.value = true;
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}

async function verify() {
  loading.value = true;
  error.value = '';
  try {
    const data = await routine('verify_otp', { phone: phone.value, code: code.value });
    login(data.token, data.user_id);
    router.push('/');
  } catch (e) {
    error.value = e.message;
  } finally {
    loading.value = false;
  }
}
</script>
