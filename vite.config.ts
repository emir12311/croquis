import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// Frontend-only for now. Tauri wraps this later (a `src-tauri/` sibling),
// so the dev server stays a plain browser target while we build the UI.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // Tauri's devUrl must match exactly, so the port is not allowed to drift.
  server: { port: 5173, strictPort: true },
})
