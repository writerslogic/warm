// COUNT of each query read from stdin, one per line, through the served path; prints one count per line
// (or `ERR` for a query that does not parse or run). usage: count_queries <index> <threads> < queries
use anyhow::{Context, Result};
use std::io::BufRead;
use writerslogic_warm::core::documents::{LexicalOptions, LexicalQuery};
use writerslogic_warm::WarmCore;
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let threads: usize = a.get(2).context("threads")?.parse()?;
    let core = WarmCore::new(1024, Some(a.get(1).context("index")?.clone()), None)?;
    let qs: Vec<String> = std::io::stdin().lock().lines().collect::<std::io::Result<_>>()?;
    let chunk = qs.len().div_ceil(threads.max(1)).max(1);
    let o = LexicalOptions { k: 0, count: true, ..Default::default() };
    let parts: Vec<Vec<String>> = std::thread::scope(|s| {
        let hs: Vec<_> = qs
            .chunks(chunk)
            .map(|part| {
                let (core, o) = (&core, &o);
                s.spawn(move || {
                    part.iter()
                        .map(|q| match LexicalQuery::parse(q).and_then(|p| core.search_lexical(&p, o)) {
                            Ok(r) => r.count.map_or("ERR".into(), |c| c.to_string()),
                            Err(_) => "ERR".into(),
                        })
                        .collect()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("worker")).collect()
    });
    for line in parts.into_iter().flatten() {
        println!("{line}");
    }
    Ok(())
}
