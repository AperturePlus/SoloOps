import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig, loadEnv } from "vite";
import { mockPlugin } from "./src/mock/index.ts";

export default defineConfig(({ mode }) => {
  // Vite loads .env.[mode] for `import.meta.env` but does NOT populate
  // process.env, so load explicitly and mirror into process.env so the mock
  // plugin's gate (process.env.VITE_MOCK) sees the mode file's value.
  const env = loadEnv(mode, process.cwd(), "VITE_");
  if (env.VITE_MOCK !== undefined) process.env.VITE_MOCK = env.VITE_MOCK;

  return {
    plugins: [tailwindcss(), sveltekit(), mockPlugin()],
    server: {
      host: "127.0.0.1",
      proxy: {
        "/api": {
          target: "http://127.0.0.1:3001",
          ws: true
        }
      }
    }
  };
});
