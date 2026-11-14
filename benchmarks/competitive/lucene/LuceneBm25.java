// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//
// Apache Lucene BM25 comparator for warm-bench (version pinned in warmbench/jars.py): same phases and output shape as
// text_worker.py (build, serve, ops). EnglishAnalyzer (Porter stemming, English stop words)
// and the default BM25Similarity (k1 1.2, b 0.75) are Lucene's documented defaults.
//
// Usage: java -cp lucene-core.jar:lucene-analysis-common.jar:classes LuceneBm25
//        <build|serve|ops> <input.json> <store dir> <out.json> <t0 epoch ns> <passes> <ops> [analyzer]
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

  record Doc(String id, String text) {}

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
    Map<String, Object> root = (Map<String, Object>) new Json(Files.readString(input, StandardCharsets.UTF_8)).value();
    for (Object o : (List<Object>) root.get("documents")) {
      Map<String, Object> m = (Map<String, Object>) o;
      docs.add(new Doc((String) m.get("id"), (String) m.get("text")));
    }
    for (Object o : (List<Object>) root.get("queries")) {
      Map<String, Object> m = (Map<String, Object>) o;
      queries.add(new Q((String) m.get("id"), (String) m.get("text")));
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
    StringBuilder json = new StringBuilder("{");
    switch (phase) {
      case "build" -> {
        long start = System.nanoTime();
        try (FSDirectory dir = FSDirectory.open(store);
            IndexWriter w = new IndexWriter(dir, new IndexWriterConfig(analyzer).setSimilarity(new BM25Similarity()))) {
          for (Doc d : docs) w.addDocument(doc(d.id, d.text));
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
        String[] ids = new String[reader.maxDoc()];
        StoredFields stored = reader.storedFields();
        for (int d = 0; d < ids.length; d++) ids[d] = stored.document(d, Set.of("id")).get("id");
        double openS = (System.nanoTime() - start) / 1e9;
        // Queries are analyzed once, outside the timed region, as every other comparator receives its query.
        Query[] prepared = new Query[queries.size()];
        for (int i = 0; i < prepared.length; i++) prepared[i] = query(analyzer, queries.get(i).text);
        search(searcher, prepared[0], ids);
        double cold = (System.currentTimeMillis() * 1_000_000L - t0) / 1e9;
        for (int i = 0; i < Math.min(16, prepared.length); i++) search(searcher, prepared[i], ids);
        // JIT warm-up: whole untimed passes until two consecutive passes agree within 5 percent (at least
        // 3, at most 30), so the timed passes run compiled code.
        int warmPasses = 0, stable = 0;
        double previous = -1;
        while (warmPasses < 30) {
          long t = System.nanoTime();
          for (Query q : prepared) search(searcher, q, ids);
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
            ScoreDoc[] hits = search(searcher, prepared[i], ids);
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
            for (int h = 0; h < first[i].length; h++) {
              if (h > 0) w.write(',');
              w.write("{\"documentId\":" + esc(ids[first[i][h].doc]) + ",\"lexicalScore\":" + first[i][h].score + "}");
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
            IndexWriter w = new IndexWriter(dir, new IndexWriterConfig(analyzer).setSimilarity(new BM25Similarity()))) {
          long t = System.nanoTime();
          for (int i = 0; i < opsCount; i++) {
            Doc d = docs.get(i);
            w.updateDocument(new Term("id", d.id), doc(d.id, d.text + " benchmarkrevision"));
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

  static Document doc(String id, String text) {
    Document d = new Document();
    d.add(new StringField("id", id, Field.Store.YES));
    d.add(new TextField("body", text, Field.Store.NO));
    return d;
  }

  /** The timed unit: run the prepared query, then map each hit to its document id through the table. */
  static ScoreDoc[] search(IndexSearcher searcher, Query query, String[] ids) throws IOException {
    ScoreDoc[] hits = searcher.search(query, K).scoreDocs;
    for (ScoreDoc hit : hits) SINK = ids[hit.doc];
    return hits;
  }

  // Keeps the id reads observable so the JIT cannot drop them.
  static volatile String SINK;
}
