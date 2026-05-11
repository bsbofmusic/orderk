#!/usr/bin/env node
import { runOrderk } from "../dist/index.js";
runOrderk(process.argv.slice(2)).catch((error) => {
  console.error(error?.stack || error?.message || String(error));
  process.exit(1);
});
