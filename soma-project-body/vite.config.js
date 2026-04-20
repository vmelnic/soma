import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  server: { host: true, port: 5173, hmr: { path: '/__hmr' } },
  build: { target: 'es2022', sourcemap: true },
});
