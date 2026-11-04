//! Strongly-connected components of the (tiny) part-to-part reference graph. Recursive
//! Tarjan is safe here: nodes are the parts of one package, a handful at most.

use std::collections::{BTreeMap, BTreeSet};

pub(super) type Graph = BTreeMap<String, BTreeSet<String>>;

struct Tarjan<'a> {
    graph: &'a Graph,
    index: BTreeMap<&'a str, usize>,
    low: BTreeMap<&'a str, usize>,
    stack: Vec<&'a str>,
    on_stack: BTreeSet<&'a str>,
    sccs: Vec<Vec<String>>,
}

impl<'a> Tarjan<'a> {
    fn visit(&mut self, node: &'a str) {
        let idx = self.index.len();
        self.index.insert(node, idx);
        self.low.insert(node, idx);
        self.stack.push(node);
        self.on_stack.insert(node);
        for next in self.graph.get(node).into_iter().flatten() {
            let next = next.as_str();
            if !self.index.contains_key(next) {
                self.visit(next);
                let low = self.low[node].min(self.low[next]);
                self.low.insert(node, low);
            } else if self.on_stack.contains(next) {
                let low = self.low[node].min(self.index[next]);
                self.low.insert(node, low);
            }
        }
        if self.low[node] == self.index[node] {
            let mut scc = Vec::new();
            while let Some(top) = self.stack.pop() {
                self.on_stack.remove(top);
                scc.push(top.to_string());
                if top == node {
                    break;
                }
            }
            scc.sort();
            self.sccs.push(scc);
        }
    }
}

/// SCCs with more than one node (a self-loop never forms a cross-part cycle), each sorted,
/// the list sorted — deterministic regardless of traversal order.
pub(super) fn cyclic_components(graph: &Graph) -> Vec<Vec<String>> {
    let mut nodes: BTreeSet<&str> = graph.keys().map(String::as_str).collect();
    nodes.extend(graph.values().flatten().map(String::as_str));
    let mut t = Tarjan {
        graph,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        sccs: Vec::new(),
    };
    for node in nodes {
        if !t.index.contains_key(node) {
            t.visit(node);
        }
    }
    let mut out: Vec<Vec<String>> = t.sccs.into_iter().filter(|s| s.len() > 1).collect();
    out.sort();
    out
}
