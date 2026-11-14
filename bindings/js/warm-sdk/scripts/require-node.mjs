// The built client runs on Node 18 or newer; its build tooling (tsup 9) needs Node 20.19 or newer.
const [major, minor] = process.versions.node.split('.').map(Number)
if (major < 20 || (major === 20 && minor < 19)) {
  console.error(`Building warm-sdk needs Node 20.19 or newer (found ${process.versions.node}). ` +
    'The published warm-sdk package runs on Node 18 and needs no build: npm install warm-sdk')
  process.exit(1)
}
