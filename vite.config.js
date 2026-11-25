import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

// Vite configuration for the Servora frontend.
// The dev server is bound to a fixed port so Tauri can point at it reliably.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2022",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
