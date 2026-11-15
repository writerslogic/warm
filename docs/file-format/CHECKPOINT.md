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
empty, and every chunk is lexical-only (no vector, embedding or pending encoding) and holds no
word positions (`IngestOptions::positions`). Explicit schema 7 native vector records are
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
   mapping with the live store's scoring code, so the results are the live store's.
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

## Layout

All integers little endian; varints are LEB128.

```
0    8  magic "HMSCKP\0\x03" (version 3; versions 2, "HMSCKP\0\x02", and 1, "HMSCKP\0\x01", are still read)
8    8  u64: CRC32 of bytes 16.. (low 32 bits)
16   8  u64: CRC32 of the generation directory's name (low 32 bits)
24   8  u64: vector dimensions of the store
32   8  u64: feature bits (bit 0: chunk text is in compressed blocks; any other bit is refused as newer than supported)
40   32 u64: reserved, zero
72   32 u64: slots, chunks, words, documents
104  16 u64: terms per posting form (unstemmed, stemmed)
120  208 (offset, length) of each section, in the order below (the last three only with feature bit 0; unused words are not read), to byte 328
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
   two varints, and its text (`0` none, `2` inline bytes, `3` the text's offset, a varint, in the
   compressed text stream; its length is the span).
6. with feature bit 0 set, three more sections, in this order: the dictionary (at most 64 KiB), the
    block table and the compressed blocks (below). Without the bit the header words of these
    sections are not read.

Compressed text (feature bit 0). The text of every chunk that has any is laid end to end in
document order and cut at chunk boundaries into blocks. The writer makes every chunk a block of its own (a
chunk is never split); a reader accepts any cut at chunk boundaries. Each block is an LZ4
block compressed against the dictionary (LZ4 block format with an external dictionary, as
`lz4_flex::block::compress_with_dict`), or stored as it is when that is not shorter. The
dictionary is 512-byte pieces taken at even steps from the stream, an eighth of the stream and at
most 64 KiB, and empty below 16 KiB of text. The table holds `u32` pairs (the block's offset in the stream, its offset in the block bytes),
one per block and a last pair (stream length, block bytes length); a block whose byte length
equals its stream length is stored. A reader checks that the table starts at (0, 0), that blocks
are nonempty, at most 16 MiB and no longer than their stream bytes, that they follow each other and end
at the section's end, and that a chunk's bytes lie in one block and are UTF-8. A hit decodes one block; the engine keeps decoded blocks in a
lock-free cache bounded by `document_checkpoint.text_cache_bytes` (default 64 MiB, `0` decodes on every hit), so a block is decoded once while it fits. The dictionary is sampled from the live documents' text when the checkpoint is
built, so a scrubbing compaction writes none of an erased document's text, in a block or in the
dictionary. Without text (every chunk stored without it) the feature bit is clear and the file
is byte for byte what version 2 wrote.

Every compaction writes version 3, so an older checkpoint is replaced at the store's next
compaction; until then it opens, serves and rebuilds the live store exactly as version 3 does,
ranking it exhaustively where a version 3 checkpoint is pruned (the ranking is the same to the
bit, and a version 1 or 2 checkpoint serves from the checkpoint only up to 2^19 slots). A checkpoint with
inline text opens and serves as one with compressed blocks and is replaced by one at the next
compaction. A release that predates feature bit 0 refuses a checkpoint that sets it (its section
table does not end at the end of the file).

## Durability and crash safety

The checkpoint is written with the compaction's staging generation: temporary file, sync,
rename, directory sync, before the generation's segments are synced and before `CURRENT` is
replaced (CRASH-CONSISTENCY.md). A crash before the switch leaves the previous generation; after
it, the new generation, checkpoint included. A published checkpoint is never written again; the
next one goes to the next generation.

## Text segment at close

`documents.text` ([CACHES.md](CACHES.md)) is a cache of chunk text that every open which needs it
recreates. A close that ends with a checkpoint and no frames after it cuts the file to its 8-byte
header, so the text is on disk once, in the checkpoint. The cut is a truncation, not an
overwrite, as at the next open.

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
