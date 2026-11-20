import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

const projectDirectory = import.meta.dirname;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome120",
    cssCodeSplit: true,
    rollupOptions: {
      input: {
        control: resolve(projectDirectory, "control.html"),
        widget: resolve(projectDirectory, "widget.html"),
        overlay: resolve(projectDirectory, "overlay.html"),
      },
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
  },
});
