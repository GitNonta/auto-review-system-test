import { defineConfig } from 'vite';

// dev: `npm run dev` → http://127.0.0.1:5174 (proxy /api ไปที่ dashboard-rs :7790)
// build: `npm run build` → dist/ (ไฟล์ static ล้วน)
export default defineConfig({
  server: {
    host: '0.0.0.0',
    port: 5174,
    proxy: {
      '/api': {
        target: process.env.DASHBOARD_API || 'http://127.0.0.1:7790',
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    target: 'es2020',
  },
});
