// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(super) fn run(
    graph: &Graph,
    index: &mut VGraph,
    stored: &[f32],
    q64: &[Vec<f64>],
    q32: &[Vec<f32>],
    truth: &[Vec<u32>],
    output: &mut impl Write,
) -> Result<()> {
    writeln!(
        output,
        "{}",
        json!({"type":"topology", "adjacency":index.diagnostic_adjacency()})
    )?;
    for ef in [128, 256] {
        for query in 0..QUERIES {
            let report = index.diagnose_navigation(
                graph,
                &q32[query],
                FETCH,
                VSearchParams {
                    ef,
                    rerank: ef,
                    ..Default::default()
                },
            )?;
            let selected = rank(&report.stages.results, stored, &q64[query])?;
            writeln!(
                output,
                "{}",
                json!({"type":"navigation", "ef":ef, "query":query,
                "entry":report.entry, "oracle":truth[query],
                "stages":trace(&report.stages, &selected, &truth[query])?,
                "evaluated":report.stages.evaluated, "beam":report.stages.beam,
                "events":report.events})
            )?;
            if query % 100 == 0 {
                output.flush()?;
                eprintln!("navigation ef={ef} queries={}", query + 1);
            }
        }
    }
    output.flush()?;
    Ok(())
}
