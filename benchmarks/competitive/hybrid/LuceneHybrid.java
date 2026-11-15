// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//
// Apache Lucene hybrid comparator of the hybrid contract (README.md): EnglishAnalyzer BM25 plus a float vector field
// (VectorSimilarityFunction.DOT_PRODUCT on the unit-length shared vectors, Lucene's recommendation for them),
// fused by Lucene's own TopDocs.rrf. Two vector arms: `hnsw` (KnnFloatVectorQuery on the HNSW graph, default codec
// parameters) and `exact` (a scan of every vector of the single force-merged segment with Lucene's similarity function).
//
// build <corpus.tsv> <doc.f32> <store> <out.json>
// serve <queries.tsv> <query.f32> <store> <out.json> <hnsw|exact> <rrf constants,comma> <passes> <including> <timed> <embed-bin> [lists]
// TSV lines are `id<TAB>text` with tabs and line breaks in the text replaced by spaces.

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.lang.management.ManagementFactory;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.en.EnglishAnalyzer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TotalHits;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.store.FSDirectory;

public final class LuceneHybrid {
  static final int K = 100;
  static final int DIM = 384;
  static final Pattern WORD = Pattern.compile("[\\p{L}\\p{N}]+");

  static String[][] readTsv(Path p) throws IOException {
    List<String[]> rows = new ArrayList<>();
    for (String line : Files.readAllLines(p, StandardCharsets.UTF_8)) {
      int t = line.indexOf('\t');
      rows.add(new String[] {line.substring(0, t), line.substring(t + 1)});
    }
    return rows.toArray(new String[0][]);
  }

  static float[][] readVectors(Path p, int count) throws IOException {
    ByteBuffer b = ByteBuffer.wrap(Files.readAllBytes(p)).order(ByteOrder.LITTLE_ENDIAN);
    if (b.remaining() != count * DIM * 4) throw new IOException(p + ": wrong size");
    float[][] out = new float[count][DIM];
    for (float[] row : out) for (int i = 0; i < DIM; i++) row[i] = b.getFloat();
    return out;
  }

  /** The lowercase alphanumeric word set, as the lexical contract reduces every raw query. */
  static Set<String> words(String text) {
    Set<String> out = new LinkedHashSet<>();
    Matcher m = WORD.matcher(text.toLowerCase());
    while (m.find()) out.add(m.group());
    return out;
  }

  static Query lexicalQuery(Analyzer analyzer, String text) throws IOException {
    BooleanQuery.Builder b = new BooleanQuery.Builder();
    Set<String> seen = new LinkedHashSet<>();
    for (String w : words(text)) {
      try (TokenStream ts = analyzer.tokenStream("body", w)) {
        CharTermAttribute term = ts.addAttribute(CharTermAttribute.class);
        ts.reset();
        while (ts.incrementToken()) seen.add(term.toString());
        ts.end();
      }
    }
    for (String t : seen) b.add(new TermQuery(new Term("body", t)), BooleanClause.Occur.SHOULD);
    return b.build();
  }

  static void build(String[] a) throws Exception {
    String[][] docs = readTsv(Paths.get(a[0]));
    float[][] vectors = readVectors(Paths.get(a[1]), docs.length);
    Path store = Paths.get(a[2]);
    long start = System.nanoTime();
    IndexWriterConfig cfg = new IndexWriterConfig(new EnglishAnalyzer());
    cfg.setSimilarity(new BM25Similarity());
    cfg.setRAMBufferSizeMB(256);
    try (FSDirectory dir = FSDirectory.open(store); IndexWriter w = new IndexWriter(dir, cfg)) {
      for (int i = 0; i < docs.length; i++) {
        Document d = new Document();
        d.add(new StringField("id", docs[i][0], Field.Store.YES));
        d.add(new TextField("body", docs[i][1], Field.Store.NO));
        d.add(new KnnFloatVectorField("vec", vectors[i], VectorSimilarityFunction.DOT_PRODUCT));
        w.addDocument(d);
      }
      w.forceMerge(1);
      w.commit();
    }
    double s = (System.nanoTime() - start) / 1e9;
    Files.writeString(Paths.get(a[3]), "{\"documents\":" + docs.length + ",\"ingest_s\":" + s + ",\"cpu_s\":" + cpuSeconds() + "}");
  }

  static double cpuSeconds() {
    return ((com.sun.management.OperatingSystemMXBean) ManagementFactory.getOperatingSystemMXBean()).getProcessCpuTime() / 1e9;
  }

  static final class Engine {
    final IndexSearcher searcher;
    final Analyzer analyzer = new EnglishAnalyzer();
    final String[] ids;
    final boolean exact;
    final DirectoryReader reader;

    Engine(Path store, boolean exact) throws IOException {
      reader = DirectoryReader.open(FSDirectory.open(store));
      searcher = new IndexSearcher(reader);
      searcher.setSimilarity(new BM25Similarity());
      searcher.setQueryCache(null);
      this.exact = exact;
      ids = new String[reader.maxDoc()];
      for (int i = 0; i < ids.length; i++) ids[i] = reader.storedFields().document(i).get("id");
    }

    TopDocs lexical(String text) throws IOException {
      return searcher.search(lexicalQuery(analyzer, text), K);
    }

    TopDocs vector(float[] q) throws IOException {
      if (!exact) return searcher.search(new KnnFloatVectorQuery("vec", q, K), K);
      // Every vector of every segment, scored with the field's similarity function; the top K by score.
      java.util.PriorityQueue<ScoreDoc> heap = new java.util.PriorityQueue<>(K + 1, (x, y) -> Float.compare(x.score, y.score));
      for (LeafReaderContext ctx : reader.leaves()) {
        FloatVectorValues values = ctx.reader().getFloatVectorValues("vec");
        if (values == null) continue;
        var it = values.iterator();
        for (int doc = it.nextDoc(); doc != org.apache.lucene.search.DocIdSetIterator.NO_MORE_DOCS; doc = it.nextDoc()) {
          float s = VectorSimilarityFunction.DOT_PRODUCT.compare(q, values.vectorValue(it.index()));
          if (heap.size() < K) heap.add(new ScoreDoc(ctx.docBase + doc, s));
          else if (s > heap.peek().score) { heap.poll(); heap.add(new ScoreDoc(ctx.docBase + doc, s)); }
        }
      }
      ScoreDoc[] top = heap.toArray(new ScoreDoc[0]);
      Arrays.sort(top, (x, y) -> Float.compare(y.score, x.score));
      return new TopDocs(new TotalHits(top.length, TotalHits.Relation.EQUAL_TO), top);
    }

    ScoreDoc[] hybrid(String text, float[] q, int constant) throws IOException {
      return TopDocs.rrf(K, constant, new TopDocs[] {lexical(text), vector(q)}).scoreDocs;
    }
  }

  static final class Helper {
    final Process proc;
    final DataOutputStream out;
    final DataInputStream in;
    final byte[] buf = new byte[DIM * 4];

    Helper(String bin) throws IOException {
      proc = new ProcessBuilder(bin, "embed-serve").redirectError(ProcessBuilder.Redirect.INHERIT).start();
      out = new DataOutputStream(proc.getOutputStream());
      in = new DataInputStream(proc.getInputStream());
    }

    float[] embed(String text) throws IOException {
      byte[] raw = text.getBytes(StandardCharsets.UTF_8);
      ByteBuffer req = ByteBuffer.allocate(4 + raw.length).order(ByteOrder.LITTLE_ENDIAN);
      req.putInt(raw.length).put(raw);
      out.write(req.array());
      out.flush();
      in.readFully(buf);
      ByteBuffer b = ByteBuffer.wrap(buf).order(ByteOrder.LITTLE_ENDIAN);
      float[] v = new float[DIM];
      for (int i = 0; i < DIM; i++) v[i] = b.getFloat();
      return v;
    }

    double close() throws Exception {
      out.write(new byte[] {-1, -1, -1, -1});
      out.flush();
      byte[] eight = new byte[8];
      in.readFully(eight);
      proc.waitFor();
      return ByteBuffer.wrap(eight).order(ByteOrder.LITTLE_ENDIAN).getDouble();
    }
  }

  static void hits(StringBuilder sb, Engine e, ScoreDoc[] sd) {
    sb.append('[');
    for (int i = 0; i < sd.length; i++) {
      if (i > 0) sb.append(',');
      sb.append("[\"").append(e.ids[sd[i].doc].replace("\\", "\\\\").replace("\"", "\\\"")).append("\",").append(sd[i].score).append(']');
    }
    sb.append(']');
  }

  static void longs(StringBuilder sb, long[] v, int n) {
    sb.append('[');
    for (int i = 0; i < n; i++) sb.append(i > 0 ? "," : "").append(v[i]);
    sb.append(']');
  }

  static String ids(Engine e, ScoreDoc[] sd) {
    StringBuilder sb = new StringBuilder();
    for (ScoreDoc d : sd) sb.append(e.ids[d.doc]).append('\n');
    return sb.toString();
  }

  static void serve(String[] a) throws Exception {
    String[][] queries = readTsv(Paths.get(a[0]));
    float[][] vectors = readVectors(Paths.get(a[1]), queries.length);
    boolean exact = a[4].equals("exact");
    String[] constants = a[5].split(",");
    int passes = Integer.parseInt(a[6]);
    int including = Math.min(Integer.parseInt(a[7]), queries.length);
    int timed = Math.min(Integer.parseInt(a[8]), queries.length);
    boolean lists = a.length > 10 && a[10].equals("lists");
    int rowsLimit = a.length > 11 ? Integer.parseInt(a[11]) : Integer.MAX_VALUE;
    long start = System.nanoTime();
    Engine e = new Engine(Paths.get(a[2]), exact);
    double openS = (System.nanoTime() - start) / 1e9;
    StringBuilder sb = new StringBuilder("{\"open_s\":" + openS + ",");
    if (lists) {
      sb.append("\"lists\":[");
      for (int i = 0; i < queries.length; i++) {
        if (i > 0) sb.append(',');
        sb.append("{\"id\":\"").append(queries[i][0]).append("\",\"lex\":");
        hits(sb, e, e.lexical(queries[i][1]).scoreDocs);
        sb.append(",\"vec\":");
        hits(sb, e, e.vector(vectors[i]).scoreDocs);
        sb.append('}');
      }
      sb.append("],\"cpu_s\":").append(cpuSeconds()).append('}');
      Files.writeString(Paths.get(a[3]), sb.toString());
      return;
    }
    Helper helper = new Helper(a[9]);
    sb.append("\"configs\":{");
    boolean firstConfig = true;
    for (String c : constants) {
      int constant = (int) Double.parseDouble(c);
      // JIT warm-up over the timed prefix, untimed.
      for (int w = 0; w < 3; w++) for (int i = 0; i < timed; i++) e.hybrid(queries[i][1], vectors[i], constant);
      String[] reference = new String[queries.length];
      StringBuilder rows = new StringBuilder("[");
      for (int i = 0; i < Math.min(queries.length, rowsLimit); i++) {
        ScoreDoc[] sd = e.hybrid(queries[i][1], vectors[i], constant);
        reference[i] = ids(e, sd);
        if (i > 0) rows.append(',');
        rows.append("{\"id\":\"").append(queries[i][0]).append("\",\"results\":");
        hits(rows, e, sd);
        rows.append('}');
      }
      rows.append(']');
      long[] inc = new long[including], emb = new long[including];
      int mismatched = 0;
      for (int i = 0; i < including; i++) {
        long t0 = System.nanoTime();
        float[] v = helper.embed(queries[i][1]);
        long t1 = System.nanoTime();
        ScoreDoc[] sd = e.hybrid(queries[i][1], v, constant);
        long t2 = System.nanoTime();
        inc[i] = t2 - t0;
        emb[i] = t1 - t0;
        if (!ids(e, sd).equals(reference[i])) mismatched++;
      }
      if (!firstConfig) sb.append(',');
      firstConfig = false;
      sb.append('"').append(c).append("\":{\"rows\":").append(rows).append(",\"including_ns\":");
      longs(sb, inc, including);
      sb.append(",\"embed_ns\":");
      longs(sb, emb, including);
      sb.append(",\"excluding_passes_ns\":[");
      for (int p = 0; p < passes; p++) {
        long[] ns = new long[timed];
        for (int i = 0; i < timed; i++) {
          long t0 = System.nanoTime();
          e.hybrid(queries[i][1], vectors[i], constant);
          ns[i] = System.nanoTime() - t0;
        }
        if (p > 0) sb.append(',');
        longs(sb, ns, timed);
      }
      sb.append("],\"including_id_mismatches\":").append(mismatched).append('}');
    }
    double helperCpu = helper.close();
    sb.append("},\"cpu_s\":").append(cpuSeconds() + helperCpu).append(",\"versions\":{\"lucene\":\"")
        .append(org.apache.lucene.util.Version.LATEST).append("\",\"jvm\":\"").append(System.getProperty("java.vm.version"))
        .append("\"}}");
    Files.writeString(Paths.get(a[3]), sb.toString());
  }

  public static void main(String[] a) throws Exception {
    String[] rest = Arrays.copyOfRange(a, 1, a.length);
    if (a[0].equals("build")) build(rest);
    else if (a[0].equals("serve")) serve(rest);
    else throw new IllegalArgumentException("build|serve");
  }
}
