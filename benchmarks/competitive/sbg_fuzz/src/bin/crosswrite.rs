// Cross-version write check helper. usage:
//   crosswrite write <index>   memorize a few documents, delete one, compact, flush
//   crosswrite probe <index>   print COUNT and the TOP_10 digest of fixed queries (served and exhaustive)
use anyhow::{Context, Result};
use writerslogic_warm::core::documents::{EncodeMode, IngestOptions, LexicalExecution, LexicalOptions, LexicalQuery};
use writerslogic_warm::{DocumentInput, WarmCore};
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn input(id: &str, text: &str) -> DocumentInput {
    DocumentInput {
        id: id.to_owned(),
        text: text.to_owned(),
        source_uri: None,
        version: None,
        metadata: None,
        chunk_words: Some(4096),
        overlap_words: Some(0),
        store_text: Some(false),
        embeddings: None,
        title_weight: None,
    }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (mode, path) = (a.get(1).context("mode")?, a.get(2).context("index")?);
    let core = WarmCore::new(1024, Some(path.clone()), None)?;
    match mode.as_str() {
        "write" => {
            let options = IngestOptions { encode: EncodeMode::Lexical, unstemmed: true, positions: true, whole_document: true };
            core.memorize_documents_with(
                vec![
                    input("x1", "zzcrossone the united states of america zzcrossshared"),
                    input("x2", "zzcrosstwo new york city zzcrossshared"),
                    input("x3", "zzcrossthree the who zzcrossshared"),
                ],
                &options,
            )?;
            println!("delete x2: {}", core.delete_document("x2")?);
            core.compact()?;
            core.flush()?;
        }
        "probe" => {
            for q in ["zzcrossshared", "zzcrossone", "zzcrosstwo", "the united states", "+new +york", "\"the who\"", "new york -city"] {
                let parsed = LexicalQuery::parse(q)?;
                let count = core.count_lexical(&parsed)?;
                for (name, execution) in [("auto", LexicalExecution::Auto), ("exhaustive", LexicalExecution::Exhaustive)] {
                    let r = core.search_lexical(&parsed, &LexicalOptions { k: 10, count: true, execution, ..Default::default() })?;
                    let mut h: u64 = 0xcbf29ce484222325;
                    for hit in &r.hits {
                        for b in hit.document_id.bytes().chain((hit.chunk_index as u64).to_le_bytes()).chain(hit.score.to_bits().to_le_bytes()) {
                            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
                        }
                    }
                    println!("{q:?} {name} count={count} top10_count={:?} digest={h:016x}", r.count);
                }
            }
        }
        other => anyhow::bail!("unknown mode {other}"),
    }
    Ok(())
}
