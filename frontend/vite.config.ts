import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The server replaces this placeholder in index.html with the mount path.
const BASE_PLACEHOLDER = "/__MLA_BASE__/";
// Loco app used by `npm run dev` (the dogfood app by default).
const backend = process.env.ADMIN_BACKEND ?? "http://localhost:5150";

export default defineConfig(({ command }) => ({
  base: command === "build" ? BASE_PLACEHOLDER : "/admin/",
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    modulePreload: false,
    assetsInlineLimit: 0,
    sourcemap: false,
  },
  server: {
    proxy: {
      "/admin/api": backend,
      "/api": backend,
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/test/**", "src/main.tsx", "src/**/*.test.{ts,tsx}"],
      reporter: ["text", "lcov"],
      thresholds: { lines: 85, functions: 85, statements: 85, branches: 75 },
    },
  },
}));
