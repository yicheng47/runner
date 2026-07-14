// Standalone Vitest config — deliberately not extending vite.config.ts so
// unit tests don't pull in the Tauri/Tailwind dev-server plugins. Today's
// suite is pure-logic modules (node environment, no jsdom).
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
