// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//
// Apache Lucene BM25 comparator for warm-bench (version pinned in warmbench/jars.py): same phases and output shape as
// text_worker.py (build, serve, ops). EnglishAnalyzer (Porter stemming, English stop words)
// and the default BM25Similarity (k1 1.2, b 0.75) are Lucene's documented defaults.
//
// Usage: java -cp lucene-core.jar:lucene-analysis-common.jar:classes LuceneBm25
//        <build|serve|ops> <input.json> <store dir> <out.json> <t0 epoch ns> <passes> <ops> [analyzer] [stored]
// The optional last argument `stored` selects the documents contract: the index stores id, body and bucket (default
// codec, Lucene104 BEST_SPEED stored fields) and serve fetches every hit's stored document inside the timed region.
// The optional analyzer (default english) selects a Lucene analyzer for multilingual runs; any
// analyzer other than english reads the whole query text through it, since the alphanumeric
// pre-split below breaks words at combining marks.
// The input is the {documents, queries} JSON written by warm-bench; it is parsed with a small
// purpose-built reader so the harness needs only the two Lucene jars.

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.en.EnglishAnalyzer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.store.FSDirectory;

public final class LuceneBm25 {
  static final int K = 100;

  record Doc(String id, String text, int bucket) {}

  record Q(String id, String text) {}

  /** Minimal JSON reader for the fixed input shape; throws on anything unexpected. */
  static final class Json {
    final String s;
    int p;

    Json(String s) {
      this.s = s;
    }

    void ws() {
      while (p < s.length() && Character.isWhitespace(s.charAt(p))) p++;
    }

    Object value() {
      ws();
      char c = s.charAt(p);
      if (c == '{') {
        p++;
        Map<String, Object> m = new LinkedHashMap<>();
        ws();
        if (s.charAt(p) == '}') {
          p++;
          return m;
        }
        while (true) {
          ws();
          String k = string();
          ws();
          expect(':');
          m.put(k, value());
          ws();
          if (s.charAt(p) == ',') {
            p++;
            continue;
          }
          expect('}');
          return m;
        }
      }
      if (c == '[') {
        p++;
        List<Object> l = new ArrayList<>();
        ws();
        if (s.charAt(p) == ']') {
          p++;
          return l;
        }
        while (true) {
          l.add(value());
          ws();
          if (s.charAt(p) == ',') {
            p++;
            continue;
          }
          expect(']');
          return l;
        }
      }
      if (c == '"') return string();
      int start = p;
      while (p < s.length() && ",]} \n\r\t".indexOf(s.charAt(p)) < 0) p++;
      return s.substring(start, p);
    }

    void expect(char c) {
      if (s.charAt(p) != c) throw new IllegalStateException("expected " + c + " at " + p);
      p++;
    }

    String string() {
      expect('"');
      StringBuilder b = new StringBuilder();
      while (true) {
        char c = s.charAt(p++);
        if (c == '"') return b.toString();
        if (c != '\\') {
          b.append(c);
          continue;
        }
        char e = s.charAt(p++);
        switch (e) {
          case 'n' -> b.append('\n');
          case 't' -> b.append('\t');
          case 'r' -> b.append('\r');
          case 'b' -> b.append('\b');
          case 'f' -> b.append('\f');
          case 'u' -> {
            try {
              b.append((char) Integer.parseInt(s.substring(p, p + 4), 16));
            } catch (NumberFormatException bad) {
              throw new IllegalStateException("bad unicode escape at " + p, bad);
            }
            p += 4;
          }
          default -> b.append(e);
        }
      }
    }
  }

  @SuppressWarnings("unchecked")
  static void load(Path input, List<Doc> docs, List<Q> queries) throws IOException {
    if (Files.size(input) > (1L << 30)) {
      loadLines(input, docs, queries);
      return;
    }
    Map<String, Object> root = (Map<String, Object>) new Json(Files.readString(input, StandardCharsets.UTF_8)).value();
    for (Object o : (List<Object>) root.get("documents")) {
      Map<String, Object> m = (Map<String, Object>) o;
      Object md = m.get("metadata");
      int bucket = md instanceof Map<?, ?> mm && mm.get("bucket") != null ? Integer.parseInt((String) mm.get("bucket")) : 0;
      docs.add(new Doc((String) m.get("id"), (String) m.get("text"), bucket));
    }
    for (Object o : (List<Object>) root.get("queries")) {
      Map<String, Object> m = (Map<String, Object>) o;
      queries.add(new Q((String) m.get("id"), (String) m.get("text")));
    }
  }

  /** Input above 1 GiB cannot be read as one String: warm-bench writes such files with one document per line,
   *  `{"documents":[`, the documents (comma-led after the first), `],`, then `"queries":[...]}`. */
  @SuppressWarnings("unchecked")
  static void loadLines(Path input, List<Doc> docs, List<Q> queries) throws IOException {
    try (java.io.BufferedReader r = Files.newBufferedReader(input, StandardCharsets.UTF_8)) {
      String line = r.readLine();
      if (!"{\"documents\":[".equals(line)) throw new IOException("unexpected layout of " + input);
      while ((line = r.readLine()) != null && !line.equals("],")) {
        Map<String, Object> m = (Map<String, Object>) new Json(line.startsWith(",") ? line.substring(1) : line).value();
        Object md = m.get("metadata");
        int bucket = md instanceof Map<?, ?> mm && mm.get("bucket") != null ? Integer.parseInt((String) mm.get("bucket")) : 0;
        docs.add(new Doc((String) m.get("id"), (String) m.get("text"), bucket));
      }
      Map<String, Object> root = (Map<String, Object>) new Json("{" + r.readLine()).value();
      for (Object o : (List<Object>) root.get("queries")) {
        Map<String, Object> m = (Map<String, Object>) o;
        queries.add(new Q((String) m.get("id"), (String) m.get("text")));
      }
    }
  }

  static List<String> words(String text) {
    Set<String> out = new java.util.TreeSet<>();
    StringBuilder cur = new StringBuilder();
    for (int i = 0; i < text.length(); ) {
      int cp = text.codePointAt(i);
      i += Character.charCount(cp);
      if (Character.isLetterOrDigit(cp)) {
        cur.appendCodePoint(Character.toLowerCase(cp));
      } else if (cur.length() > 0) {
        out.add(cur.toString());
        cur.setLength(0);
      }
    }
    if (cur.length() > 0) out.add(cur.toString());
    return new ArrayList<>(out);
  }

  static boolean rawQueries = false;

  static Analyzer analyzer(String name) {
    return switch (name) {
      case "english" -> new EnglishAnalyzer();
      case "standard" -> new org.apache.lucene.analysis.standard.StandardAnalyzer();
      case "hindi" -> new org.apache.lucene.analysis.hi.HindiAnalyzer();
      case "thai" -> new org.apache.lucene.analysis.th.ThaiAnalyzer();
      case "persian" -> new org.apache.lucene.analysis.fa.PersianAnalyzer();
      case "arabic" -> new org.apache.lucene.analysis.ar.ArabicAnalyzer();
      case "finnish" -> new org.apache.lucene.analysis.fi.FinnishAnalyzer();
      case "indonesian" -> new org.apache.lucene.analysis.id.IndonesianAnalyzer();
      case "turkish" -> new org.apache.lucene.analysis.tr.TurkishAnalyzer();
      case "russian" -> new org.apache.lucene.analysis.ru.RussianAnalyzer();
      case "spanish" -> new org.apache.lucene.analysis.es.SpanishAnalyzer();
      case "french" -> new org.apache.lucene.analysis.fr.FrenchAnalyzer();
      case "german" -> new org.apache.lucene.analysis.de.GermanAnalyzer();
      default -> throw new IllegalArgumentException("unknown analyzer " + name);
    };
  }

  static Query query(Analyzer analyzer, String text) throws IOException {
    Set<String> terms = new LinkedHashSet<>();
    for (String w : rawQueries ? List.of(text) : words(text)) {
      try (TokenStream ts = analyzer.tokenStream("body", w)) {
        CharTermAttribute attr = ts.addAttribute(CharTermAttribute.class);
        ts.reset();
        while (ts.incrementToken()) terms.add(attr.toString());
        ts.end();
      }
    }
    BooleanQuery.Builder b = new BooleanQuery.Builder();
    for (String t : terms) b.add(new TermQuery(new Term("body", t)), BooleanClause.Occur.SHOULD);
    return b.build();
  }

  static String esc(String s) {
    StringBuilder b = new StringBuilder("\"");
    for (char c : s.toCharArray()) {
      if (c == '"' || c == '\\') b.append('\\').append(c);
      else if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
      else b.append(c);
    }
    return b.append('"').toString();
  }

  static long rssBytes() {
    try {
      Process p = new ProcessBuilder("ps", "-o", "rss=", "-p", Long.toString(ProcessHandle.current().pid())).start();
      return Long.parseLong(new String(p.getInputStream().readAllBytes()).trim()) * 1024;
    } catch (Exception e) {
      return 0;
    }
  }

  public static void main(String[] a) throws Exception {
    String phase = a[0];
    Path input = Paths.get(a[1]), store = Paths.get(a[2]), out = Paths.get(a[3]);
    long t0 = Long.parseLong(a[4]);
    int passes = Integer.parseInt(a[5]), opsCount = Integer.parseInt(a[6]);
    List<Doc> docs = new ArrayList<>();
    List<Q> queries = new ArrayList<>();
    load(input, docs, queries);
    String analyzerName = a.length > 7 ? a[7] : "english";
    rawQueries = !analyzerName.equals("english");
    Analyzer analyzer = analyzer(analyzerName);
    boolean stored = a.length > 8 && a[8].startsWith("stored");
    // `stored-dv`: documents contract with id, body and bucket held in doc values instead of stored fields (see searchDocsDv).
    DV = a.length > 8 && a[8].equals("stored-dv");
    StringBuilder json = new StringBuilder("{");
    switch (phase) {
      case "build" -> {
        long start = System.nanoTime();
        try (FSDirectory dir = FSDirectory.open(store);
            IndexWriter w = new IndexWriter(dir, config(analyzer, stored))) {
          for (Doc d : docs) w.addDocument(doc(d, stored));
          w.forceMerge(1);
          w.commit();
        }
        json.append("\"documents\":").append(docs.size()).append(",\"ingest_s\":").append((System.nanoTime() - start) / 1e9);
      }
      case "serve" -> {
        long start = System.nanoTime();
        FSDirectory dir = FSDirectory.open(store);
        DirectoryReader reader = DirectoryReader.open(dir);
        IndexSearcher searcher = new IndexSearcher(reader);
        searcher.setSimilarity(new BM25Similarity());
        // Documented for benchmarking: the query cache would answer repeated queries from memory,
        // which no other system's timed region may do.
        searcher.setQueryCache(null);
        // Document id table built at open, as WARM keeps its own: a hit's id is an array read, not a
        // decompressing stored-fields lookup.
        String[] ids = new String[stored ? 0 : reader.maxDoc()];
        StoredFields storedFields = reader.storedFields();
        for (int d = 0; d < ids.length; d++) ids[d] = storedFields.document(d, Set.of("id")).get("id");
        // Documents contract: the hits' stored documents are read through one reused StoredFields (the
        // documented access path), inside the timed region; the id comes from the stored document.
        java.util.function.Function<String, ScoreDoc[]> run = q -> {
          try {
            return DV ? searchDocsDv(searcher, analyzer, q) : stored ? searchDocs(searcher, analyzer, q, storedFields) : search(searcher, analyzer, q, ids);
          } catch (IOException e) {
            throw new java.io.UncheckedIOException(e);
          }
        };
        double openS = (System.nanoTime() - start) / 1e9;
        // One contract for every system: the raw query string goes in, the engine's own analysis and query
        // construction are inside the timed region, and the top-100 documents come out.
        String[] prepared = new String[queries.size()];
        for (int i = 0; i < prepared.length; i++) prepared[i] = queries.get(i).text;
        run.apply(prepared[0]);
        double cold = (System.currentTimeMillis() * 1_000_000L - t0) / 1e9;
        for (int i = 0; i < Math.min(16, prepared.length); i++) run.apply(prepared[i]);
        // JIT warm-up: whole untimed passes until two consecutive passes agree within 5 percent (at least
        // 3, at most 30), so the timed passes run compiled code.
        int warmPasses = 0, stable = 0;
        double previous = -1;
        while (warmPasses < 30) {
          long t = System.nanoTime();
          for (String q : prepared) run.apply(q);
          double took = System.nanoTime() - t;
          warmPasses++;
          stable = previous > 0 && Math.abs(took - previous) / previous < 0.05 ? stable + 1 : 0;
          previous = took;
          if (warmPasses >= 3 && stable >= 2) break;
        }
        ScoreDoc[][] first = new ScoreDoc[prepared.length][];
        long[][] passNs = new long[passes][];
        for (int n = 0; n < passes; n++) {
          long[] ns = new long[prepared.length];
          for (int i = 0; i < prepared.length; i++) {
            long t = System.nanoTime();
            ScoreDoc[] hits = run.apply(prepared[i]);
            ns[i] = System.nanoTime() - t;
            if (n == 0) first[i] = hits;
          }
          passNs[n] = ns;
        }
        // Streamed to the file: the report is never held as one string.
        try (java.io.BufferedWriter w = Files.newBufferedWriter(out, StandardCharsets.UTF_8)) {
          w.write("{\"open_s\":" + openS + ",\"cold_start_s\":" + cold + ",\"warmup_passes\":" + warmPasses + ",\"rows\":[");
          for (int i = 0; i < first.length; i++) {
            if (i > 0) w.write(',');
            w.write("{\"id\":" + esc(queries.get(i).id) + ",\"results\":[");
            if (stored) {
              // Untimed: what each hit carried, for the orchestrator to compare with the corpus.
              ScoreDoc[] hits = searcher.search(query(analyzer, prepared[i]), K).scoreDocs;
              for (int h = 0; h < hits.length; h++) {
                if (h > 0) w.write(',');
                String[] dvFields = DV ? fetchDv(searcher.getIndexReader(), hits[h].doc) : null;
                Document d = DV ? null : storedFields.document(hits[h].doc);
                byte[] body = (DV ? dvFields[1] : d.get("body")).getBytes(StandardCharsets.UTF_8);
                java.util.zip.CRC32 crc = new java.util.zip.CRC32();
                crc.update(body);
                w.write("{\"documentId\":" + esc(DV ? dvFields[0] : d.get("id")) + ",\"lexicalScore\":" + hits[h].score + ",\"check\":{\"crc\":" + crc.getValue()
                    + ",\"len\":" + body.length + ",\"start\":0,\"end\":" + body.length + ",\"bucket\":" + (DV ? dvFields[2] : d.getField("bucket").numericValue()) + "}}");
              }
            } else {
              for (int h = 0; h < first[i].length; h++) {
                if (h > 0) w.write(',');
                w.write("{\"documentId\":" + esc(ids[first[i][h].doc]) + ",\"lexicalScore\":" + first[i][h].score + "}");
              }
            }
            w.write("]}");
          }
          w.write("],\"passes_ns\":[");
          for (int n = 0; n < passNs.length; n++) {
            if (n > 0) w.write(',');
            w.write(Arrays.toString(passNs[n]));
          }
          w.write("],\"peak_rss_bytes_self\":" + rssBytes() + "}");
        }
        reader.close();
        dir.close();
        return;
      }
      case "ops" -> {
        try (FSDirectory dir = FSDirectory.open(store);
            IndexWriter w = new IndexWriter(dir, config(analyzer, stored))) {
          long t = System.nanoTime();
          for (int i = 0; i < opsCount; i++) {
            Doc d = docs.get(i);
            w.updateDocument(new Term("id", d.id), doc(new Doc(d.id, d.text + " benchmarkrevision", d.bucket), stored));
            w.commit();
          }
          double replace = (System.nanoTime() - t) / 1e6 / opsCount;
          t = System.nanoTime();
          for (int i = opsCount; i < 2 * opsCount; i++) {
            w.deleteDocuments(new Term("id", docs.get(i).id));
            w.commit();
          }
          double delete = (System.nanoTime() - t) / 1e6 / opsCount;
          json.append("\"count\":").append(opsCount)
              .append(",\"replace\":{\"supported\":true,\"per_op_ms_durable\":").append(replace)
              .append("},\"delete\":{\"supported\":true,\"per_op_ms_durable\":").append(delete).append('}');
        }
      }
      default -> throw new IllegalArgumentException(phase);
    }
    json.append(",\"peak_rss_bytes_self\":").append(rssBytes()).append('}');
    Files.writeString(out, json.toString());
  }

  static IndexWriterConfig config(Analyzer analyzer, boolean stored) {
    IndexWriterConfig c = new IndexWriterConfig(analyzer).setSimilarity(new BM25Similarity());
    // The default codec's stored-fields mode is BEST_SPEED (fast, lightly compressed); set explicitly so the choice is on record.
    if (stored) c.setCodec(new org.apache.lucene.codecs.lucene104.Lucene104Codec(org.apache.lucene.codecs.lucene104.Lucene104Codec.Mode.BEST_SPEED));
    return c;
  }

  static Document doc(Doc doc, boolean stored) {
    Document d = new Document();
    if (DV) {
      // Payload kept in doc values (O(1) per-document access, no block decompression) instead of stored fields.
      d.add(new StringField("id", doc.id, Field.Store.NO));
      d.add(new TextField("body", doc.text, Field.Store.NO));
      d.add(new org.apache.lucene.document.BinaryDocValuesField("idv", new org.apache.lucene.util.BytesRef(doc.id)));
      d.add(new org.apache.lucene.document.BinaryDocValuesField("bodydv", new org.apache.lucene.util.BytesRef(doc.text)));
      d.add(new org.apache.lucene.document.NumericDocValuesField("bucketdv", doc.bucket));
      return d;
    }
    d.add(new StringField("id", doc.id, Field.Store.YES));
    d.add(new TextField("body", doc.text, stored ? Field.Store.YES : Field.Store.NO));
    if (stored) d.add(new org.apache.lucene.document.StoredField("bucket", doc.bucket));
    return d;
  }

  /** The timed unit: analyze the query string, run it, then map each hit to its document id through the table. */
  static ScoreDoc[] search(IndexSearcher searcher, Analyzer analyzer, String text, String[] ids) throws IOException {
    ScoreDoc[] hits = searcher.search(query(analyzer, text), K).scoreDocs;
    for (ScoreDoc hit : hits) SINK = ids[hit.doc];
    return hits;
  }

  /** Documents contract: search, then read every hit's stored document (id, body, bucket) through the reused StoredFields. */
  static ScoreDoc[] searchDocs(IndexSearcher searcher, Analyzer analyzer, String text, StoredFields storedFields) throws IOException {
    ScoreDoc[] hits = searcher.search(query(analyzer, text), K).scoreDocs;
    for (ScoreDoc hit : hits) {
      Document d = storedFields.document(hit.doc);
      SINK = d.get("id");
      SINK = d.get("body");
      SINK_NUM = d.getField("bucket").numericValue();
    }
    return hits;
  }

  static boolean DV;

  /** Documents contract with the payload in doc values: fresh iterators per query, hits read in docid order (advanceExact only moves forward), results kept by rank. */
  static ScoreDoc[] searchDocsDv(IndexSearcher searcher, Analyzer analyzer, String text) throws IOException {
    ScoreDoc[] hits = searcher.search(query(analyzer, text), K).scoreDocs;
    int n = hits.length;
    long[] order = new long[n];
    for (int r = 0; r < n; r++) order[r] = ((long) hits[r].doc << 32) | r;
    Arrays.sort(order);
    java.util.List<org.apache.lucene.index.LeafReaderContext> leaves = searcher.getIndexReader().leaves();
    String[] ids = new String[n], bodies = new String[n];
    long[] buckets = new long[n];
    int leafIdx = -1;
    org.apache.lucene.index.BinaryDocValues idv = null, bodydv = null;
    org.apache.lucene.index.NumericDocValues bucketdv = null;
    org.apache.lucene.index.LeafReaderContext ctx = null;
    for (int i = 0; i < n; i++) {
      int r = (int) (order[i] & 0xffffffffL), doc = hits[r].doc;
      int want = org.apache.lucene.index.ReaderUtil.subIndex(doc, leaves);
      if (want != leafIdx) {
        leafIdx = want;
        ctx = leaves.get(want);
        idv = ctx.reader().getBinaryDocValues("idv");
        bodydv = ctx.reader().getBinaryDocValues("bodydv");
        bucketdv = ctx.reader().getNumericDocValues("bucketdv");
      }
      int local = doc - ctx.docBase;
      idv.advanceExact(local);
      org.apache.lucene.util.BytesRef b = idv.binaryValue();
      ids[r] = new String(b.bytes, b.offset, b.length, StandardCharsets.UTF_8);
      bodydv.advanceExact(local);
      b = bodydv.binaryValue();
      bodies[r] = new String(b.bytes, b.offset, b.length, StandardCharsets.UTF_8);
      bucketdv.advanceExact(local);
      buckets[r] = bucketdv.longValue();
    }
    if (n > 0) {
      SINK = ids[n - 1];
      SINK = bodies[n - 1];
      SINK_NUM = buckets[n - 1];
    }
    return hits;
  }

  /** Untimed: id, body and bucket of one document from doc values. */
  static String[] fetchDv(org.apache.lucene.index.IndexReader reader, int doc) throws IOException {
    org.apache.lucene.index.LeafReaderContext ctx = reader.leaves().get(org.apache.lucene.index.ReaderUtil.subIndex(doc, reader.leaves()));
    int local = doc - ctx.docBase;
    org.apache.lucene.index.BinaryDocValues idv = ctx.reader().getBinaryDocValues("idv"), bodydv = ctx.reader().getBinaryDocValues("bodydv");
    org.apache.lucene.index.NumericDocValues bucketdv = ctx.reader().getNumericDocValues("bucketdv");
    idv.advanceExact(local);
    bodydv.advanceExact(local);
    bucketdv.advanceExact(local);
    return new String[] {idv.binaryValue().utf8ToString(), bodydv.binaryValue().utf8ToString(), Long.toString(bucketdv.longValue())};
  }

  // Keeps the id reads observable so the JIT cannot drop them.
  static volatile String SINK;
  static volatile Number SINK_NUM;
}
