import assert from "node:assert/strict";
import process from "node:process";
import { openStore } from "@writerslogic/warm-wasm";
import { FileStorage } from "@writerslogic/warm-wasm/storage/file";

const [mode, directory, encryptionMode] = process.argv.slice(2);
assert.ok(["writer", "reader"].includes(mode));
assert.ok(["plain", "pbkdf2", "argon2id"].includes(encryptionMode));
const options = {
  dimensions: 1024,
  storage: new FileStorage(directory),
  autosave: false,
  ...(encryptionMode === "plain" ? {} : { encryption: {
    passphrase: "public-recovery-fixture", kdf: encryptionMode, iterations: 1000,
  } }),
};
let store = await openStore(options);
const id = (index) => `r${String(index).padStart(2, "0")}`;
const text = (index, revision) => `recovery record ${String(index).padStart(2, "0")} ${revision === 1 ? "original" : "replacement"}`;
if (mode === "writer") {
  for (const [count, revision, phase] of [[32, 1, "inserted"], [16, 2, "replaced"]]) {
    const counts = await store.memorizeDocuments(Array.from({ length: count }, (_, index) => ({
      id: id(index), text: text(index, revision), metadata: { revision },
    })));
    assert.deepEqual(counts, Array(count).fill(1));
    await store.save();
    console.log(JSON.stringify({ phase, documents: count }));
  }
  for (let index = 16; index < 24; index++) assert.equal(await store.deleteDocument(id(index)), true);
  await store.save();
  console.log(JSON.stringify({ phase: "ready", deleted: 8 }));
  setInterval(() => {}, 60_000);
  await new Promise(() => {});
} else {
  assert.equal((await store.stats()).documents, 24);
  for (let index = 0; index < 32; index++) {
    const hits = await store.searchDocuments("recovery", { k: 1, documentIds: [id(index)] });
    if (index >= 16 && index < 24) {
      assert.deepEqual(hits, []);
      assert.equal(await store.hasDocument(id(index)), false);
      continue;
    }
    assert.equal(hits.length, 1);
    assert.equal(hits[0].documentId, id(index));
    const revision = index < 16 ? 2 : 1;
    assert.equal(hits[0].text, text(index, revision));
    assert.deepEqual(hits[0].metadata, { revision });
  }
  await store.memorizeDocument({ id: "after", text: "recovery resumed write" });
  await store.save();
  await store.close();
  store = await openStore(options);
  const hits = await store.searchDocuments("recovery", { documentIds: ["after"] });
  assert.equal(hits.length, 1);
  assert.equal(hits[0].text, "recovery resumed write");
  await store.close();
  console.log(JSON.stringify({ phase: "verified", records: 24, deleted: 8, resumedWrite: true }));
}
