/// PageRank via power iteration. Dangling nodes (no outbound links) have
/// their mass redistributed uniformly each iteration so the total stays 1.

pub fn compute(link_graph: &[Vec<u32>], damping: f32, iterations: usize) -> Vec<f32> {
    let n = link_graph.len();
    if n == 0 {
        return Vec::new();
    }

    let nf = n as f32;
    let mut pr = vec![1.0 / nf; n];
    let mut next = vec![0.0f32; n];

    // Reverse adjacency so we can sum inbound per-node in one pass.
    let mut inbound: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (src, outs) in link_graph.iter().enumerate() {
        for &dst in outs {
            let d = dst as usize;
            if d < n && d != src {
                inbound[d].push(src as u32);
            }
        }
    }

    let out_degree: Vec<f32> = link_graph.iter().map(|v| v.len() as f32).collect();
    let base = (1.0 - damping) / nf;

    for _ in 0..iterations {
        let mut dangling = 0.0f32;
        for i in 0..n {
            if out_degree[i] == 0.0 {
                dangling += pr[i];
            }
        }
        let dangling_contrib = damping * dangling / nf;

        for i in 0..n {
            let mut sum = 0.0f32;
            for &src in &inbound[i] {
                let s = src as usize;
                if out_degree[s] > 0.0 {
                    sum += pr[s] / out_degree[s];
                }
            }
            next[i] = base + damping * sum + dangling_contrib;
        }

        std::mem::swap(&mut pr, &mut next);
    }

    pr
}
