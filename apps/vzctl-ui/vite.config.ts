import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: "graph-utils", test: /node_modules\/@antv\/x6\/es\/(common|geometry)\//, priority: 10 },
            { name: "graph", test: /node_modules\/@antv\// },
            { name: "react", test: /node_modules\/(react|react-dom|scheduler)\// },
            { name: "routing", test: /node_modules\/@tanstack\// },
            { name: "config", test: /node_modules\/(zod|yaml)\// },
          ],
        },
      },
    },
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
