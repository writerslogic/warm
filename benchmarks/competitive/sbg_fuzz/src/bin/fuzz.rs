// Randomized exactness differential: served (Auto) against LexicalExecution::Exhaustive.
//
// usage: fuzz <index> <vocab.tsv> --seed N --queries N [--threads N] [--out DIR]
//             [--corpus corpus.jsonl [--corpus-docs N]] [--slots N] [--threshold NAME=VALUE]... [--lengths 63,64,65] [--length-reps 3]
//
// Queries are seeded and reproducible. Terms are drawn from document-frequency strata of `vocab.tsv`
// (written by the `vocab` binary), and half of the multi-term queries draw their terms from one window of
// a real document (with --corpus) so that intersections and phrases are not all empty. Shapes: term,
// union, intersection, phrase, required plus optional, and required plus negated, with 1 to 8 terms.
// Each query runs with a k from a fixed edge list or a random k up to 10,000, and one of the commands
// top-k, count, top-k plus count. Boundary terms (df at limit-1, limit and limit+1 of every named
// threshold) are always exercised. Digest: identical to qb7d (count, FNV-1a over id, chunk, score bits).
// A one-sided error is a mismatch; the same error on both sides is a match.
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::io::{BufRead, BufReader};
use writerslogic_warm::core::documents::{
    LexicalExecution, LexicalOptions, LexicalQuery, LexicalResults,
};
use writerslogic_warm::WarmCore;
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Same digest as `qb7d.rs`; the two must stay byte-identical in behavior.
fn digest(r: &LexicalResults) -> (usize, u64) {
    let mut h: u64 = 0xcbf29ce484222325;
    for hit in &r.hits {
        for b in hit
            .document_id
            .bytes()
            .chain((hit.chunk_index as u64).to_le_bytes())
            .chain(hit.score.to_bits().to_le_bytes())
        {
            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
    }
    (r.count.unwrap_or(usize::MAX), h)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const KS: [usize; 15] = [
    1, 2, 7, 9, 10, 11, 99, 100, 101, 999, 1000, 1001, 4095, 4096, 4097,
];
/// df strata (inclusive bounds); the last two are the longest lists.
const STRATA: [(u32, u32); 8] = [
    (1, 1),
    (2, 9),
    (10, 99),
    (100, 999),
    (1000, 9999),
    (10000, 99999),
    (100000, 999999),
    (1000000, u32::MAX),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cmd {
    Top,
    Count,
    TopCount,
}

struct Case {
    query: String,
    k: usize,
    cmd: Cmd,
}

struct Vocab {
    docs: u64,
    terms: Vec<(String, u32)>, // df descending
}

fn load_vocab(path: &str) -> Result<Vocab> {
    let mut docs = 0;
    let mut terms = Vec::new();
    for line in BufReader::new(std::fs::File::open(path)?).lines() {
        let line = line?;
        if let Some(n) = line.strip_prefix("#docs ") {
            docs = n.parse()?;
        } else if let Some((t, c)) = line.split_once('\t') {
            terms.push((t.to_owned(), c.parse()?));
        }
    }
    Ok(Vocab { docs, terms })
}

/// The terms nearest to df = `value` - 1, `value`, `value` + 1 (the closest df at or below the target for
/// the first two, at or above for the last), with the df actually found.
fn boundary_terms(v: &Vocab, value: u32) -> Vec<(String, u32)> {
    let asc = |i: usize| &v.terms[v.terms.len() - 1 - i];
    let n = v.terms.len();
    let mut out = Vec::new();
    for target in [value.saturating_sub(1), value, value + 1] {
        let (mut l, mut h) = (0, n);
        while l < h {
            let mid = (l + h) / 2;
            if asc(mid).1 >= target {
                h = mid
            } else {
                l = mid + 1
            }
        }
        // Closest df to the target on either side.
        let cand: Vec<usize> = [l.checked_sub(1), (l < n).then_some(l)]
            .into_iter()
            .flatten()
            .collect();
        if let Some(&i) = cand.iter().min_by_key(|&&i| asc(i).1.abs_diff(target)) {
            out.push(asc(i).clone());
        }
    }
    out.dedup();
    out
}

struct Generator<'a> {
    rng: Rng,
    strata: Vec<Vec<usize>>, // indexes into vocab.terms
    vocab: &'a Vocab,
    windows: Vec<Vec<String>>, // word windows of real documents
}

impl Generator<'_> {
    fn term(&mut self) -> String {
        loop {
            let s = self.rng.below(self.strata.len());
            if !self.strata[s].is_empty() {
                let i = self.strata[s][self.rng.below(self.strata[s].len())];
                return self.vocab.terms[i].0.clone();
            }
        }
    }

    fn terms(&mut self, n: usize) -> Vec<String> {
        if !self.windows.is_empty() && self.rng.below(2) == 0 {
            let w = &self.windows[self.rng.below(self.windows.len())];
            let start = self.rng.below(w.len().saturating_sub(n) + 1);
            let mut out: Vec<String> = w[start..].iter().take(n).cloned().collect();
            while out.len() < n {
                out.push(self.term());
            }
            return out;
        }
        (0..n).map(|_| self.term()).collect()
    }

    fn query(&mut self) -> String {
        let n = 1 + self.rng.below(8);
        let t = self.terms(n);
        match (n, self.rng.below(6)) {
            (1, _) => t[0].clone(),
            (_, 0) => t.join(" "),
            (_, 1) => t
                .iter()
                .map(|w| format!("+{w}"))
                .collect::<Vec<_>>()
                .join(" "),
            (_, 2) => format!("\"{}\"", t.join(" ")),
            (_, 3) => {
                // Required plus optional: at least one required term.
                let req = self.rng.below(n);
                t.iter()
                    .enumerate()
                    .map(|(i, w)| {
                        if i == req || self.rng.below(3) == 0 {
                            format!("+{w}")
                        } else {
                            w.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            (_, 4) => {
                // Required plus negated.
                let negated = 1 + self.rng.below(n - 1);
                t.iter()
                    .enumerate()
                    .map(|(i, w)| {
                        if i < n - negated {
                            format!("+{w}")
                        } else {
                            format!("-{w}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            _ => {
                // A phrase with a required term and an exclusion.
                let tail = self.term();
                format!("+\"{}\" -{}", t.join(" "), tail)
            }
        }
    }

    fn case(&mut self, query: String) -> Case {
        let k = if self.rng.below(4) == 0 {
            1 + self.rng.below(10_000)
        } else {
            KS[self.rng.below(KS.len())]
        };
        let cmd = [Cmd::Top, Cmd::Count, Cmd::TopCount][self.rng.below(3)];
        Case { query, k, cmd }
    }
}

type Outcome = std::result::Result<(usize, u64), String>;

fn options(c: &Case, execution: LexicalExecution) -> LexicalOptions {
    let (k, count) = match c.cmd {
        Cmd::Top => (c.k, false),
        Cmd::Count => (0, true),
        Cmd::TopCount => (c.k, true),
    };
    LexicalOptions {
        k,
        count,
        execution,
        ..Default::default()
    }
}

fn run(
    core: &WarmCore,
    c: &Case,
    execution: LexicalExecution,
) -> std::result::Result<LexicalResults, String> {
    let q = LexicalQuery::parse(&c.query).map_err(|e| format!("{e:#}"))?;
    core.search_lexical(&q, &options(c, execution))
        .map_err(|e| format!("{e:#}"))
}

fn summarize(r: &std::result::Result<LexicalResults, String>) -> Outcome {
    r.as_ref().map(digest).map_err(Clone::clone)
}

fn lists(r: &std::result::Result<LexicalResults, String>) -> serde_json::Value {
    match r {
        Ok(r) => json!({
            "count": r.count,
            "hits": r.hits.iter().map(|h| json!([h.document_id, h.chunk_index, format!("{:016x}", h.score.to_bits())])).collect::<Vec<_>>(),
        }),
        Err(e) => json!({ "error": e }),
    }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (index, vocab_path) = (a.get(1).context("index")?, a.get(2).context("vocab")?);
    let (mut seed, mut n, mut threads, mut out) =
        (1u64, 1000usize, 4usize, String::from("fuzz-out"));
    let (mut corpus, mut corpus_docs, mut slots) = (None, 20_000usize, None);
    let mut thresholds: Vec<(String, u32)> = Vec::new();
    let mut lengths: Vec<usize> = vec![63, 64, 65];
    let mut length_reps = 3usize;
    let mut i = 3;
    while i < a.len() {
        let v = a
            .get(i + 1)
            .with_context(|| format!("value for {}", a[i]))?;
        match a[i].as_str() {
            "--seed" => seed = v.parse()?,
            "--queries" => n = v.parse()?,
            "--threads" => threads = v.parse()?,
            "--out" => out = v.clone(),
            "--corpus" => corpus = Some(v.clone()),
            "--corpus-docs" => corpus_docs = v.parse()?,
            "--lengths" => lengths = v.split(',').map(str::parse).collect::<std::result::Result<_, _>>()?,
            "--length-reps" => length_reps = v.parse()?,
            "--slots" => slots = Some(v.parse::<u32>()?),
            "--threshold" => {
                let (name, value) = v.split_once('=').context("NAME=VALUE")?;
                thresholds.push((name.to_owned(), value.parse()?));
            }
            other => bail!("unknown option {other}"),
        }
        i += 2;
    }
    let vocab = load_vocab(vocab_path)?;
    if let Some(s) = slots {
        // A list is tabled at postings * 32 >= slots (impact.rs `tabled`).
        thresholds.push(("SLOT_SHARE".into(), s.div_ceil(32)));
    }
    let strata: Vec<Vec<usize>> = STRATA
        .iter()
        .map(|&(lo, hi)| {
            (0..vocab.terms.len())
                .filter(|&i| (lo..=hi).contains(&vocab.terms[i].1))
                .collect()
        })
        .collect();
    let mut windows = Vec::new();
    if let Some(path) = &corpus {
        for line in BufReader::new(std::fs::File::open(path)?)
            .lines()
            .take(corpus_docs)
        {
            let v: serde_json::Value = serde_json::from_str(&line?)?;
            let words: Vec<String> = v["text"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            if words.len() >= 8 {
                windows.push(words.into_iter().take(400).collect());
            }
        }
    }
    let mut g = Generator {
        rng: Rng(seed),
        strata,
        vocab: &vocab,
        windows,
    };
    let mut cases: Vec<Case> = (0..n)
        .map(|_| {
            let q = g.query();
            g.case(q)
        })
        .collect();
    // Boundary terms: each alone and with a long-list term, across edge k values and every command.
    let long = vocab.terms[vocab.terms.len().min(20) / 2].0.clone();
    let mut boundary = Vec::new();
    for (name, value) in &thresholds {
        for (term, df) in boundary_terms(&vocab, *value) {
            boundary.push(json!({ "threshold": name, "value": value, "term": term, "df": df }));
            for q in [
                term.clone(),
                format!("{term} {long}"),
                format!("+{term} +{long}"),
                format!("+{long} -{term}"),
                format!("+{term} -{long}"),
            ] {
                for &k in &[1usize, 100, 4095, 4096, 4097] {
                    for cmd in [Cmd::Top, Cmd::Count, Cmd::TopCount] {
                        cases.push(Case {
                            query: q.clone(),
                            k,
                            cmd,
                        });
                    }
                }
            }
        }
    }
    // Query-length boundary (impact.rs MAX_TERMS = 64): 63, 64 and 65 distinct terms in every operator shape.
    for &len in &lengths {
        for _ in 0..length_reps {
            let t = g.terms(len);
            let mut distinct = t.clone();
            distinct.sort();
            distinct.dedup();
            let t = if distinct.len() == len { t } else { (0..len).map(|i| format!("{}", g.vocab.terms[i % g.vocab.terms.len()].0)).collect() };
            let half = len / 2;
            let queries = [
                t.join(" "),
                t.iter().map(|w| format!("+{w}")).collect::<Vec<_>>().join(" "),
                t.iter().enumerate().map(|(i, w)| if i < half { format!("+{w}") } else { w.clone() }).collect::<Vec<_>>().join(" "),
                t.iter().enumerate().map(|(i, w)| if i == 0 { format!("+{w}") } else { format!("-{w}") }).collect::<Vec<_>>().join(" "),
                t.iter().enumerate().map(|(i, w)| if i < 2 { format!("+{w}") } else if i % 2 == 0 { format!("-{w}") } else { w.clone() }).collect::<Vec<_>>().join(" "),
            ];
            // The exhaustive side scores every posting of every term, tens of seconds per case at 5M
            // documents, so each long query gets one k and one command; the repetitions rotate them.
            for query in queries {
                let k = [10usize, 100, 1000][g.rng.below(3)];
                let cmd = [Cmd::Top, Cmd::Count, Cmd::TopCount][g.rng.below(3)];
                cases.push(Case { query, k, cmd });
            }
        }
    }
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        format!("{out}/boundary_terms.json"),
        serde_json::to_string_pretty(&boundary)?,
    )?;
    let core = WarmCore::new(1024, Some(index.clone()), None)?;
    let chunk = cases.len().div_ceil(threads.max(1)).max(1);
    let (progress, total) = (std::sync::atomic::AtomicUsize::new(0), cases.len());
    let bad: Vec<Vec<(usize, serde_json::Value)>> = std::thread::scope(|s| {
        let hs: Vec<_> = cases
            .chunks(chunk)
            .enumerate()
            .map(|(p, part)| {
                let (core, progress) = (&core, &progress);
                s.spawn(move || {
                    let mut bad = Vec::new();
                    for (j, c) in part.iter().enumerate() {
                        if std::env::var_os("FUZZ_VERBOSE").is_some() {
                            eprintln!("start {} k={} {:?}", c.query, c.k, c.cmd);
                        }
                        let started = std::time::Instant::now();
                        let auto = run(core, c, LexicalExecution::Auto);
                        let auto_time = started.elapsed();
                        let exh = run(core, c, LexicalExecution::Exhaustive);
                        let done = progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if done % 250 == 0 || started.elapsed().as_secs() >= 5 {
                            eprintln!("progress {done}/{total} auto {auto_time:?} total {:?} {} k={} {:?}", started.elapsed(), c.query.chars().take(60).collect::<String>(), c.k, c.cmd);
                        }
                        if summarize(&auto) != summarize(&exh) {
                            let repro = json!({ "query": c.query, "k": c.k, "command": format!("{:?}", c.cmd), "auto": lists(&auto), "exhaustive": lists(&exh) });
                            bad.push((p * chunk + j, repro));
                        }
                    }
                    bad
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("worker")).collect()
    });
    let bad: Vec<_> = bad.into_iter().flatten().collect();
    for (idx, repro) in &bad {
        std::fs::write(
            format!("{out}/repro-{idx}.json"),
            serde_json::to_string_pretty(repro)?,
        )?;
    }
    println!(
        "{{\"index\": {index:?}, \"docs\": {}, \"seed\": {seed}, \"generated\": {n}, \"boundary_cases\": {}, \"total\": {}, \"mismatches\": {}}}",
        vocab.docs,
        cases.len() - n,
        cases.len(),
        bad.len()
    );
    if bad.is_empty() {
        Ok(())
    } else {
        std::process::exit(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(dfs: &[u32]) -> Vocab {
        let mut terms: Vec<(String, u32)> = dfs.iter().map(|&d| (format!("t{d}"), d)).collect();
        terms.sort_by(|a, b| b.1.cmp(&a.1));
        Vocab { docs: 0, terms }
    }

    #[test]
    fn boundary_picks_exact_neighbours_or_the_nearest() {
        let exact = boundary_terms(&vocab(&[1, 511, 512, 513, 9000]), 512);
        assert_eq!(exact.iter().map(|t| t.1).collect::<Vec<_>>(), [511, 512, 513]);
        // No df of 4,095 or 4,097: the nearest df stands in for each and the repeats collapse.
        let sparse = boundary_terms(&vocab(&[10, 4096, 9000]), 4096);
        assert_eq!(sparse.iter().map(|t| t.1).collect::<Vec<_>>(), [4096]);
    }
}
