import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

// The server replaces this placeholder in index.html with the mount path.
const BASE_PLACEHOLDER = "/__MLA_BASE__/";
// Loco app used by `npm run dev` (the dogfood app by default).
const backend = process.env.ADMIN_BACKEND ?? "http://localhost:5150";
// The licenses of the bundled npm packages, shipped in the published crate.
const LICENSES_FILE = "THIRD-PARTY-LICENSES.md";
// Packages that put code in the CSS bundle, which Vite's license list leaves out.
const CSS_PACKAGES = ["tailwindcss"];

// Adds the CSS packages to the license file Vite writes for the JS bundle, in its format.
function cssLicenses(): Plugin {
  const require = createRequire(import.meta.url);
  return {
    name: "muy-loco-admin:css-licenses",
    apply: "build",
    generateBundle: {
      order: "post",
      handler(_, bundle) {
        const file = bundle[LICENSES_FILE];
        if (file?.type !== "asset") {
          this.error(`Vite did not write ${LICENSES_FILE}`);
        }
        let text = String(file.source);
        for (const name of CSS_PACKAGES) {
          const manifest = require.resolve(`${name}/package.json`);
          const pkg = JSON.parse(readFileSync(manifest, "utf-8")) as {
            version: string;
            license: string;
          };
          const license = readFileSync(join(dirname(manifest), "LICENSE"), "utf-8").trim();
          text += `\n## ${name} - ${pkg.version} (${pkg.license})\n\n${license}\n`;
        }
        file.source = text;
      },
    },
  };
}

export default defineConfig(({ command }) => ({
  base: command === "build" ? BASE_PLACEHOLDER : "/admin/",
  plugins: [react(), tailwindcss(), cssLicenses()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    modulePreload: false,
    assetsInlineLimit: 0,
    sourcemap: false,
    license: { fileName: LICENSES_FILE },
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
