import { defineConfig } from "vite";

// Tauri 期望一个固定端口的前端开发服务器；构建产物在 dist/ 下为多页面。
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome120",
    minify: "esbuild",
    sourcemap: false,
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        island: "island.html",
        settings: "settings.html",
      },
    },
  },
});
