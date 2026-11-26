// Differential: served (Auto) vs Exhaustive, all modes, threaded. usage: qb7d <idx> <queries.jsonl> <threads> [modes comma list]
use anyhow::{Context, Result};
use writerslogic_warm::core::documents::{LexicalExecution, LexicalOptions, LexicalQuery, LexicalResults};
use writerslogic_warm::WarmCore;
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn digest(r: &LexicalResults) -> (usize, u64) {
    let mut h: u64 = 0xcbf29ce484222325;
    for hit in &r.hits {
        for b in hit.document_id.bytes().chain((hit.chunk_index as u64).to_le_bytes()).chain(hit.score.to_bits().to_le_bytes()) {
            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
    }
    (r.count.unwrap_or(usize::MAX), h)
}

fn run(core: &WarmCore, qs: &[String], threads: usize, o: LexicalOptions) -> Result<Vec<(usize, u64)>> {
    let chunk = qs.len().div_ceil(threads);
    let parts: Vec<Result<Vec<(usize, u64)>>> = std::thread::scope(|s| {
        let hs: Vec<_> = qs.chunks(chunk).map(|part| {
            let o = o.clone();
            s.spawn(move || part.iter().map(|q| Ok(digest(&core.search_lexical(&LexicalQuery::parse(q)?, &o)?))).collect())
        }).collect();
        hs.into_iter().map(|h| h.join().expect("worker")).collect()
    });
    let mut out = Vec::new();
    for p in parts { out.extend(p?); }
    Ok(out)
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let threads: usize = a[3].parse()?;
    let core = WarmCore::new(1024, Some(a[1].clone()), None)?;
    let mut qs = Vec::new();
    for line in std::fs::read_to_string(&a[2])?.lines() {
        let v: serde_json::Value = serde_json::from_str(line)?;
        qs.push(v["query"].as_str().context("query")?.to_owned());
    }
    let wanted = a.get(4).cloned().unwrap_or_else(|| "TOP_10,TOP_100,TOP_1000,COUNT,TOP_100_COUNT".into());
    let mut total = 0;
    for mode in wanted.split(',') {
        let (k, count) = match mode { "TOP_10" => (10, false), "TOP_100" => (100, false), "TOP_1000" => (1000, false), "COUNT" => (0, true), "TOP_10_COUNT" => (10, true), "TOP_100_COUNT" => (100, true), _ => anyhow::bail!("mode") };
        let mk = |execution| LexicalOptions { k, count, execution, ..Default::default() };
        let auto = run(&core, &qs, threads, mk(LexicalExecution::Auto))?;
        let exh = run(&core, &qs, threads, mk(LexicalExecution::Exhaustive))?;
        let bad: Vec<_> = (0..qs.len()).filter(|&i| auto[i] != exh[i]).collect();
        println!("{mode} n={} differences={}", qs.len(), bad.len());
        for &i in bad.iter().take(8) { println!("  {:?} auto={:?} exh={:?}", qs[i], auto[i], exh[i]); }
        total += bad.len();
    }
    println!("TOTAL differences={total}");
    Ok(())
}
