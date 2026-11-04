import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";

const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")) as { version: string };

/**
 * TV runtimes load the app from file:// (Tizen/webOS packaged apps). Chromium refuses module
 * scripts and `crossorigin` resources from file://, and 2018 TVs (Chromium 53/56) have no ES
 * modules at all, so emit one classic deferred IIFE script and plain stylesheet link.
 */
function classicScript(): Plugin {
  return {
    name: "captureone-tv-classic-script",
    enforce: "post",
    transformIndexHtml(html) {
      return html
        .replace(/<script type="module" crossorigin src="([^"]+)"><\/script>/g, '<script defer src="$1"></script>')
        .replace(/<script type="module" src="([^"]+)"><\/script>/g, '<script defer src="$1"></script>')
        .replace(/ crossorigin(="[^"]*")?/g, "");
    },
  };
}

export default defineConfig({
  base: "./",
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  plugins: [classicScript()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2015",
    cssTarget: "chrome53",
    modulePreload: false,
    assetsInlineLimit: 0,
    sourcemap: false,
    rolldownOptions: {
      output: {
        format: "iife",
        entryFileNames: "assets/app.js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
  test: {
    environment: "jsdom",
    include: ["test/**/*.test.ts"],
  },
});
