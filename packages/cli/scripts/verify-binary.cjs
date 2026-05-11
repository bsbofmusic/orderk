
const { resolveOrderkBinary } = require("../dist/resolveBinary.js");
try {
  const bin = resolveOrderkBinary(process.env.ORDERK_BIN);
  console.log(bin);
} catch (error) {
  console.log("skip: " + error.message);
}
