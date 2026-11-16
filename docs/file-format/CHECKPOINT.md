<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Document checkpoint (`documents.ckpt`)

Schema 4 and later (`store.json` `version`) may hold a document checkpoint in a journal generation: `vectors_data.bin/gen-*/documents.ckpt`. It is the
store's data, not a cache. The generation is the checkpoint followed by the frames in its
segments, and the store's state is the checkpoint's documents with those frames applied in
order. The frames that came before the checkpoint are not kept: the term data is stored once.

## When one is written

A compaction (`WarmCore::compact`, `compact_scrubbing`, purge and forget) of a store that takes
checkpoints writes one in place of the document frames it would otherwise write. A store takes
one when checkpoints are enabled (`WarmConfig::document_checkpoint.enabled`, on by default), every
mutation applied since the open was a document write, encoding or delete, the vector shards are
empty, every chunk is lexical-only (no vector, embedding or pending encoding), and either every
chunk or none holds word positions (`IngestOptions::positions`; version 4 stores them beside the
postings, earlier versions could not hold them). Explicit schema 7 native vector records are
not representable, even though their HDC state is lexical: they retain a dense embedding
and a distinct record kind. They remain in the journal/live overlay; any store containing
them compacts to frames and its next generation has no checkpoint. Any other ineligible
store also compacts to frames, as before.

`flush` and closing the engine run such a compaction when the live generation holds at least
`document_checkpoint.min_journal_bytes` (256 KiB by default) of frames. Ingest closes into a single
checkpoint; later writes append frames after it until the next one folds them in.

## Opening

1. The live generation is the one `CURRENT` names (LAYOUT.md). No `documents.ckpt` there: the
   frames are replayed as in any store.
2. A checkpoint in a store whose schema is below 4 fails the open.
3. The checkpoint envelope and indexes are checked before use: magic, generation binding (CRC32 of the
   generation directory's name), vector dimensions, CRC32 of every byte after the CRC word (or
   the AEAD tag, ENCRYPTION.md), and every section, offset, dictionary entry and slot owner. A
   checkpoint that fails any check fails the open: it is never skipped, since the frames it
   replaced are gone.
4. The file is mapped and unfiltered lexical queries without a query embedding rank from the
   mapping with the live store's scoring code, so the results are the live store's. A version 4
   checkpoint with no documents written since and no dead document also answers explicit
   Boolean and phrase queries and counts (`WarmCore::search_lexical`) from the mapping: required
   clauses are intersected by leapfrogging posting cursors that skip blocks through the skip
   tables, a phrase checks the words' positions on the documents that hold all of them, and the
   result, scores to the bit, is the exhaustive scorer's. Any other store builds the live store
   first, as before.
5. Document writes in every encode mode (eager, deferred, lexical), deferred encodings and
   document deletes go to a store over the checkpoint that holds the documents written since;
   a checkpoint document they replace or delete is marked dead. Queries rank the checkpoint's
   live slots and that store's together, with the document count, average length and document
   frequencies counted over live chunks only (dead slots are skipped while a posting list
   decodes). The checkpoint holds no vector state, so chunk vectors, embeddings and pending
   encodings belong to the documents written since: chunk-vector queries and deferred encoding
   read that store and the vector shards without building the live store. Embedding counts,
   vector/dense index status and index maintenance also use only this live overlay, without
   decoding unrelated checkpoint records. Frames after the checkpoint are replayed the same
   way at open, which rebuilds the dead set: the frames are
   its persistent record.
6. Document records, metadata summaries and counts read the live overlay and checkpoint
   directly. Sorted pages merge their IDs and skip tombstones; only selected records are
   decoded. Metadata summaries avoid copying chunk text; retained-text reads check the selected
   text. Record decoding still checks borrowed text and allocates chunk descriptors.
7. Operations requiring the complete lexical index, compaction and writes other than those
   listed above build the live store from the checkpoint (no frame is read), remove the dead
   documents and install the ones written
   since. Deep offline verification explicitly performs this rebuild to validate all record
   contents and their agreement with the slot table, including checksum-valid malformed data.

Record decoding enforces the writer's input limits before allocating metadata or
chunk arrays: version 1..=1024 bytes, source URI at most 8192 bytes, metadata JSON
at most 64 KiB and an object, and 1..=4096 chunks. Document and explicit chunk IDs
must be nonempty, at most 65535 UTF-8 bytes, and contain no NUL. Chunk spans are
nonempty and end within the 8 MiB document limit; retained text must match the
span's byte length. These checks do not replace whole-checkpoint verification of
record/slot/posting agreement.

Building the live store re-encodes each posting list in the live encoding, restores every document and slot (free slots
included) as recorded, reads each chunk's terms back from the postings, and moves chunk text to
`documents.text`. The result answers every query exactly as the replay of the frames the
checkpoint replaced would: `tests/document_checkpoint.rs` compares hits, scores (as bits) and
records with a store that never checkpointed, through writes and reopens.

## Streaming folds

A checkpoint is also written while documents are still arriving. After each committed
document batch, an engine whose live index has reached `document_checkpoint.index_memory_budget`
(64 MiB by default; 0 turns folding off) compacts, and the compaction writes the checkpoint by
merging instead of encoding the whole store in memory:

1. the checkpoint under the store (if there is one) and the live store are read side by side;
2. each term dictionary merges in term order; a term only the checkpoint has keeps its posting
   bytes as they are, a term only the live store has is packed after the checkpoint's slots, and
   a term in both is unpacked and packed again as one list;
3. the checkpoint's documents are copied as bytes and interleaved by id with the live
   documents; owners in the slot table are renumbered;
4. every section goes to a block-addressed run (`durable_file::BlockWriter`, 64 KiB blocks), and
   the runs are assembled into one file, synced, and published by the compaction's staging
   generation exactly as an in-memory checkpoint is.

The file is byte for byte the checkpoint the whole store would encode to (the format is
unchanged; the generation CRC differs by name), so a store ingested in folds and one ingested in
a pass are the same store. When the compaction published and no frame was appended since its
snapshot, the engine then replaces the live index with a store over the new checkpoint (the state
a reopen reaches) and the resident index is released. The merge reads the mapped checkpoint
sequentially and drops its resident pages as it goes.

What a fold needs: a plaintext store that takes checkpoints, only lexical-only chunks without
positions, a version 2 checkpoint, and no dead checkpoint document. A sealed (encrypted)
store's checkpoint is one AEAD message and stays on the in-memory path until the block
container carries it; a document written under an id the checkpoint already holds kills the
checkpoint's document, and the next compaction then rebuilds the full store as before.

A crash at any point of a fold leaves the previous generation current: its checkpoint plus the
frames of every committed batch, which is the state the next open replays. The scratch directory
beside the staging generation (`gen-*.fold`) is removed by the next compaction.

Each fold rewrites the whole checkpoint, so a fixed budget does quadratic work. The trigger is
therefore `max(index_memory_budget, index_fold_fraction * checkpoint bytes)` (fraction 20.0 by
default; 0 keeps the fixed budget): the live index may grow to a multiple of the checkpoint
before the next rewrite and the number of rewrites is logarithmic in the corpus. A larger
fraction folds less and holds more memory; the MS MARCO sweep is in the pull request.

The estimate that `index_memory_budget` is compared with is the sum, over the chunks installed
since the store was created or last folded, of 512 bytes plus 40 bytes per distinct term plus the
document id length. It runs at about a third of the process's resident growth (measured on
BEIR FiQA and NFCorpus: the budget `B` gave a peak of about the base footprint plus `2.5 B`
to `3 B`), so the parameter bounds the index, not the process.

## Layout

All integers little endian; varints are LEB128.

```
0    8  magic "HMSCKP\0\x04" (version 4; versions 3, "HMSCKP\0\x03", 2, "HMSCKP\0\x02", and 1, "HMSCKP\0\x01", are still read)
8    8  u64: CRC32 of bytes 16.. (low 32 bits)
16   8  u64: CRC32 of the generation directory's name (low 32 bits)
24   8  u64: vector dimensions of the store
32   8  u64: flags (version 4; bit 0: every chunk holds word positions; bit 1: no posting has
         frequency zero, which the Boolean executor over the mapping requires), zero before version 4
40   32 u64: reserved, zero
72   32 u64: slots, chunks, words, documents
104  16 u64: terms per posting form (unstemmed, stemmed)
120  160 (offset, length) of each section, in the order below (twelve from version 4, ten
         before); then reserved words to byte 328
328  sections, back to back, ending at the end of the file
```

Sections:

1. per posting form, a dictionary of the sorted terms in blocks of 32 front-coded entries
   (prefix length, suffix bytes, posting count, posting byte length) and a block table of `u32`
   pairs (the block's dictionary offset, the posting offset of its first term);
2. the posting lists of every term, form 0 then form 1 in dictionary order. Version 3: version 2
   plus impacts. The list starts with two varints, its largest frequency and the smallest word
   count of any chunk it holds. Each block carries three more varints after its two width
   bytes: its last slot minus its first, its largest frequency and the smallest word count among
   its chunks (a list is at least two bytes plus six per block). A reader bounds the BM25 weight
   of every posting in a block, or in the list, by that frequency at that word count, and skips
   the blocks whose bound cannot reach the current `k`-th best score without decoding them.
   Opening for a rebuild checks every recorded value against the postings and the slot word
   counts. Version 2: blocks
   of 128 postings, the last one shorter; per block a varint of its first slot minus the
   previous block's last slot (the first block's is the slot itself), a byte of slot-gap bits,
   a byte of frequency bits, then the block's slot gaps (the first one zero) and its
   frequencies, each a little-endian bit string at its width (a list is at least three bytes per
   block). Version 1: per posting one varint `slot delta << 1 | (frequency != 1)`, followed by
   the frequency as a varint only when it is not 1 (the first delta is the slot itself; a list
   is at least one byte per posting);
3. per slot, `u32` word count, `u32` document ordinal (`u32::MAX` for a free slot) and `u32`
   chunk index;
4. document ids in id order: `documents + 1` `u32` offsets, then the UTF-8 bytes;
5. per document, `documents + 1` `u32` offsets, then a record: version, source URI (flag and
   bytes), metadata JSON, chunk count, and per chunk a flags byte (bit 0: the chunk id follows,
   otherwise it is the default id for its index; bit 1: its terms are stemmed; bit 2: its terms
   come from a non-legacy store analyzer), the byte span as
   two varints, and its text (`0` none, `2` inline bytes).

Version 4 adds, after the version 3 sections, two more:

6. skip tables: for each list of two or more blocks, in dictionary order across both forms, five
    little-endian `u32` arrays one after the other, each as long as the list has blocks: the last
    slot of each block, the offset of the block's header in the list's bytes, the offset of the
    block's positions in the list's positions bytes, the block's largest frequency and the block's
    least word count. Then the frontier directory, `blocks + runs + 1` `u32` byte offsets into the
    frontier bytes that follow it (a run is 32 blocks, the last one shorter): the offset of each
    block's frontier, of each run's, and the end. A frontier is a varint point count and that many
    `(frequency, words)` varint pairs, by frequency descending: the points of the block's (run's)
    postings that no other point dominates with a larger frequency at no greater word count. A
    reader bounds the BM25 weight of every posting in the block (run) by the largest weight of any
    point, which is tighter than the block's two maxima; a block whose count is zero has the single
    point of its two maxima. A list of one block has no table;
7. positions: for each list, in dictionary order, one region per block when the flags say
    positional: a width byte `w` (the bits needed for the longest posting's byte length), the
    byte length of each posting's positions packed at `w` bits (the packing of the frequencies),
    then each posting's positions end to end. A posting's positions are the token positions of
    the term in the chunk as unsigned LEB128 gaps (the first absolute), the very bytes of the
    chunk's positional record for the term (LEXICAL.md). There are no more of them than the
    posting's frequency.

A dictionary entry gains two final varints, the byte length of its list's positions (zero without
positions) and the byte length of its skip table (zero for a list of one block), and a block table entry is four `u64`s: the offsets of the block's first entry in the
dictionary and of its first list in the postings, skip tables and positions. The `WHOLE` chunk
flag (bit 3) marks a chunk whose positions are a whole article's (positional record version 2).
Opening checks the dictionary, every offset it implies and the section totals, not the
posting, skip or position bytes beyond the CRC; the rebuild of the live store checks each list
as before and validates every chunk's positions.

Every compaction writes version 4, so an older checkpoint is replaced at the store's next
compaction; until then it opens, serves and rebuilds the live store exactly as version 3 does,
ranking it exhaustively where a version 3 checkpoint is pruned (the ranking is the same to the
bit, and a version 1 or 2 checkpoint serves from the checkpoint only up to 2^19 slots).

## Durability and crash safety

The checkpoint is written with the compaction's staging generation: temporary file, sync,
rename, directory sync, before the generation's segments are synced and before `CURRENT` is
replaced (CRASH-CONSISTENCY.md). A crash before the switch leaves the previous generation; after
it, the new generation, checkpoint included. A published checkpoint is never written again; the
next one goes to the next generation.

## Purge and forget

A delete is a frame after the checkpoint; the document's terms stay in the checkpoint until the
next compaction, as they stayed in the journal before checkpoints. A purge or forget runs a
scrubbing compaction before it returns: the store's slots and text segment are first laid out
as a store that never held the document lays them out, the new checkpoint is built from that
store, and the previous generation, checkpoint included, is overwritten with zeros, synced
and removed (`tests/document_checkpoint.rs::purge_leaves_no_trace_in_any_checkpoint`).

The process-local text-cache location also carries a CRC32 of the original
plaintext. It is computed while replaying or inserting authoritative records and
checked on reads before text can enter a checkpoint or compacted journal. It is
not serialized: neither the cache bytes nor the durable formats change. This
detects accidental cache corruption, not maliciously chosen CRC collisions;
encrypted records retain AES-GCM authentication. A failed optional close-time
checkpoint reports its error and leaves the original journal recoverable.
