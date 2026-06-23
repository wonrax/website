import { defineConfig } from "vitest/config";

// These are plain unit tests for the remark/rehype plugins, so they don't need
// Astro's Vite setup. Astro v7's `getViteConfig` return type no longer carries
// Vitest's `test` field (Vite 8 / Vitest 4), so use Vitest's own config.
export default defineConfig({
  test: {
    include: ["plugins/**/*.test.ts"],
  },
});
