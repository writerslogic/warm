"""Generate computed-score bounds and unchanged numerical control methods."""
from semantic_scan_patch import replace_once


def patch(original):
    start = original.index('    fn upper(')
    end = original.index('\n}\n\n#[cfg(test)]', start)
    baseline = original[start:end]
    names = ('upper', 'search', 'search_where', 'search_where_limited', 'search_where_with_work')
    for name in names:
        baseline = baseline.replace('fn ' + name + '(', 'fn ' + name + '_baseline(')
        baseline = baseline.replace('fn ' + name + '<', 'fn ' + name + '_baseline<')
        baseline = baseline.replace('self.' + name + '(', 'self.' + name + '_baseline(')
        baseline = baseline.replace('self.' + name + '::', 'self.' + name + '_baseline::')
    candidate = replace_once(original,
        'rows below, and a query prunes a node when the outward-rounded `f64` upper bound of its\n//! dot product is below the current k-th score.',
        'rows below. A query prunes a node when the same-order endpoint dot bounds the\n//! computed row scores below the current k-th score.')
    old = original[start:original.index('\n    pub(super) fn search(', start)]
    new = '''    fn upper(&self, node: usize, query: &[f64]) -> f64 {
        let node = &self.nodes[node];
        // Rounded products and same-order additions are monotone, bounding the computed
        // score, including cancellation/subnormals. Keep Embedding::cosine's reduction.
        let upper = query.iter().enumerate().map(|(dimension, &value)| {
            let endpoint = if value >= 0.0 { node.high[dimension] } else { node.low[dimension] };
            value * f64::from(endpoint)
        }).sum::<f64>();
        if upper.is_nan() { 1.0 } else { upper.clamp(-1.0, 1.0) }
    }
'''
    candidate = replace_once(candidate, old, new)
    return candidate, '\n#[cfg(test)]\nimpl DenseIndex {\n' + baseline + '\n}\n'
