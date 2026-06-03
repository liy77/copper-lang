// Bundles the shared Copper lexicon into out/ at build time.
//
// At runtime the extension first looks for ../copper-lexicon.json (works in the
// monorepo / dev host), then for out/copper-lexicon.json (the only copy present
// inside a packaged .vsix). This script produces that second copy so a packaged
// extension shows the exact same Copper keyword descriptions as copper-lsp —
// the single source of truth stays editors/copper-lexicon.json.

const fs = require("fs");
const path = require("path");

const src = path.join(__dirname, "..", "..", "copper-lexicon.json");
const outDir = path.join(__dirname, "..", "out");
const dest = path.join(outDir, "copper-lexicon.json");

try {
  fs.mkdirSync(outDir, { recursive: true });
  fs.copyFileSync(src, dest);
  console.log("bundled copper-lexicon.json -> out/copper-lexicon.json");
} catch (err) {
  console.error("failed to bundle copper-lexicon.json:", err.message);
  process.exit(1);
}
