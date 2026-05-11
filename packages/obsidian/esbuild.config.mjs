
import esbuild from "esbuild";
import builtins from "builtin-modules";

const prod = process.argv.includes("production");
const watch = process.argv.includes("--watch");

const ctx = await esbuild.context({
  entryPoints: ["src/main.ts"],
  bundle: true,
  outfile: "main.js",
  platform: "node",
  format: "cjs",
  target: "es2020",
  sourcemap: prod ? false : "inline",
  minify: prod,
  treeShaking: true,
  external: ["obsidian", "electron", ...builtins, ...builtins.map((m) => `node:${m}`)],
  define: {
    "process.env.NODE_ENV": JSON.stringify(prod ? "production" : "development")
  }
});

if (watch) {
  await ctx.watch();
  console.log("watching orderk plugin");
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
