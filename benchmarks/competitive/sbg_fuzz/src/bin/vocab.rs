// Document frequency of every term of a search-benchmark-game corpus (one JSON object with a `text`
// field per line, as build_index reads it). usage: vocab <corpus.jsonl> > vocab.tsv
// Output: `#docs <documents with words>` then `term<TAB>df`, df descending then term ascending.
// Words are the whitespace tokens, which is how the benchmark's pre-transformed corpus is indexed.
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};

#[derive(Deserialize)]
struct Doc {
    text: String,
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: vocab <corpus.jsonl>"))?;
    let mut df: HashMap<String, u32> = HashMap::new();
    let mut docs = 0u64;
    let mut words: Vec<&str>;
    for line in BufReader::new(std::fs::File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let doc: Doc = serde_json::from_str(&line)?;
        words = doc.text.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        docs += 1;
        words.sort_unstable();
        words.dedup();
        for w in words {
            if let Some(c) = df.get_mut(w) {
                *c += 1;
            } else {
                df.insert(w.to_owned(), 1);
            }
        }
    }
    let mut rows: Vec<(&String, &u32)> = df.iter().collect();
    rows.sort_unstable_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut out = BufWriter::new(std::io::stdout().lock());
    writeln!(out, "#docs {docs}")?;
    for (t, c) in rows {
        writeln!(out, "{t}\t{c}")?;
    }
    out.flush()?;
    Ok(())
}
