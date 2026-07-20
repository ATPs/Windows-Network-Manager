import { defineConfig } from 'vite';

export default defineConfig({
  root: 'frontend',
  clearScreen: false,
  build: { outDir: '../dist', emptyOutDir: true },
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
});
