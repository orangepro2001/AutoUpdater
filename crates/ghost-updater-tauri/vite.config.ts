import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// 输出目录用 webdist(src-tauri 的 frontendDist 指向这里)
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { outDir: "webdist", emptyOutDir: true, target: "chrome105" },
});
