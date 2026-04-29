use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct GraphHealthInput {
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GraphHealthThresholds {
    pub hub_min_out_degree: usize,
    pub hub_max_in_degree: usize,
}

impl Default for GraphHealthThresholds {
    fn default() -> Self {
        Self {
            hub_min_out_degree: 3,
            hub_max_in_degree: 1,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HubStubMetric {
    pub node: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub total_degree: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FragileBridgeMetric {
    pub node: String,
    pub component_size: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct IsolatedCommunityMetric {
    pub community_id: String,
    pub size: usize,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GraphHealthReport {
    pub node_count: usize,
    pub edge_count: usize,
    pub hub_stub: Vec<HubStubMetric>,
    pub fragile_bridge: Vec<FragileBridgeMetric>,
    pub isolated_community: Vec<IsolatedCommunityMetric>,
    pub orphan: Vec<String>,
}

pub fn lint_graph_health(
    input: &GraphHealthInput,
    thresholds: &GraphHealthThresholds,
) -> GraphHealthReport {
    let mut nodes = BTreeSet::new();
    for node in &input.nodes {
        nodes.insert(node.clone());
    }
    for edge in &input.edges {
        nodes.insert(edge.from.clone());
        nodes.insert(edge.to.clone());
    }

    let mut dedup_edges = BTreeSet::<(String, String)>::new();
    for edge in &input.edges {
        if edge.from.trim().is_empty() || edge.to.trim().is_empty() {
            continue;
        }
        dedup_edges.insert((edge.from.clone(), edge.to.clone()));
    }

    let mut indegree = BTreeMap::<String, usize>::new();
    let mut outdegree = BTreeMap::<String, usize>::new();
    let mut undirected = BTreeMap::<String, BTreeSet<String>>::new();
    for node in &nodes {
        indegree.insert(node.clone(), 0);
        outdegree.insert(node.clone(), 0);
        undirected.insert(node.clone(), BTreeSet::new());
    }
    for (from, to) in &dedup_edges {
        *outdegree.entry(from.clone()).or_insert(0) += 1;
        *indegree.entry(to.clone()).or_insert(0) += 1;
        undirected
            .entry(from.clone())
            .or_default()
            .insert(to.clone());
        undirected
            .entry(to.clone())
            .or_default()
            .insert(from.clone());
    }

    let mut hub_stub = nodes
        .iter()
        .filter_map(|node| {
            let in_d = *indegree.get(node).unwrap_or(&0);
            let out_d = *outdegree.get(node).unwrap_or(&0);
            let total = in_d + out_d;
            if out_d >= thresholds.hub_min_out_degree && in_d <= thresholds.hub_max_in_degree {
                Some(HubStubMetric {
                    node: node.clone(),
                    in_degree: in_d,
                    out_degree: out_d,
                    total_degree: total,
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    hub_stub.sort_by(|a, b| a.node.cmp(&b.node));

    let orphan = nodes
        .iter()
        .filter(|node| undirected.get(*node).is_some_and(|neighbors| neighbors.is_empty()))
        .cloned()
        .collect::<Vec<_>>();

    let components = connected_components(&nodes, &undirected);
    let largest_index = components
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left.len()
                .cmp(&right.len())
                .then_with(|| component_key(left).cmp(&component_key(right)).reverse())
        })
        .map(|(idx, _)| idx);

    let mut isolated_community = components
        .iter()
        .enumerate()
        .filter_map(|(idx, component)| {
            if Some(idx) == largest_index || component.len() < 2 {
                return None;
            }
            Some(IsolatedCommunityMetric {
                community_id: format!("community_{}", idx + 1),
                size: component.len(),
                nodes: component.clone(),
            })
        })
        .collect::<Vec<_>>();
    isolated_community.sort_by(|a, b| a.community_id.cmp(&b.community_id));

    let articulation = articulation_points(&nodes, &undirected);
    let node_to_component_size = component_size_lookup(&components);
    let mut fragile_bridge = articulation
        .into_iter()
        .map(|node| FragileBridgeMetric {
            component_size: *node_to_component_size.get(&node).unwrap_or(&1),
            node,
        })
        .collect::<Vec<_>>();
    fragile_bridge.sort_by(|a, b| a.node.cmp(&b.node));

    GraphHealthReport {
        node_count: nodes.len(),
        edge_count: dedup_edges.len(),
        hub_stub,
        fragile_bridge,
        isolated_community,
        orphan,
    }
}

fn connected_components(
    nodes: &BTreeSet<String>,
    undirected: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Vec<String>> {
    let mut visited = BTreeSet::<String>::new();
    let mut components = Vec::<Vec<String>>::new();
    for node in nodes {
        if visited.contains(node) {
            continue;
        }
        let mut stack = vec![node.clone()];
        let mut component = Vec::<String>::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current.clone()) {
                continue;
            }
            component.push(current.clone());
            if let Some(neighbors) = undirected.get(&current) {
                for neighbor in neighbors.iter().rev() {
                    if !visited.contains(neighbor) {
                        stack.push(neighbor.clone());
                    }
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort_by(|left, right| component_key(left).cmp(&component_key(right)));
    components
}

fn component_key(component: &Vec<String>) -> (usize, String) {
    (
        usize::MAX - component.len(),
        component.first().cloned().unwrap_or_default(),
    )
}

fn component_size_lookup(components: &[Vec<String>]) -> BTreeMap<String, usize> {
    let mut lookup = BTreeMap::new();
    for component in components {
        for node in component {
            lookup.insert(node.clone(), component.len());
        }
    }
    lookup
}

fn articulation_points(
    nodes: &BTreeSet<String>,
    undirected: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    struct TarjanState {
        time: usize,
        disc: BTreeMap<String, usize>,
        low: BTreeMap<String, usize>,
        parent: BTreeMap<String, Option<String>>,
        articulation: BTreeSet<String>,
    }

    fn dfs(node: &str, undirected: &BTreeMap<String, BTreeSet<String>>, state: &mut TarjanState) {
        state.time += 1;
        state.disc.insert(node.to_string(), state.time);
        state.low.insert(node.to_string(), state.time);

        let mut children = 0usize;
        let neighbors = undirected.get(node).cloned().unwrap_or_default();
        for neighbor in neighbors {
            if !state.disc.contains_key(&neighbor) {
                children += 1;
                state
                    .parent
                    .insert(neighbor.clone(), Some(node.to_string()));
                dfs(&neighbor, undirected, state);

                let low_neighbor = *state.low.get(&neighbor).unwrap_or(&usize::MAX);
                let low_node = *state.low.get(node).unwrap_or(&usize::MAX);
                state
                    .low
                    .insert(node.to_string(), low_node.min(low_neighbor));

                let is_root = state.parent.get(node).is_none_or(|parent| parent.is_none());
                if is_root && children > 1 {
                    state.articulation.insert(node.to_string());
                }
                if !is_root {
                    let disc_node = *state.disc.get(node).unwrap_or(&0);
                    if low_neighbor >= disc_node {
                        state.articulation.insert(node.to_string());
                    }
                }
            } else {
                let parent = state.parent.get(node).and_then(|entry| entry.clone());
                if parent.as_deref() != Some(neighbor.as_str()) {
                    let disc_neighbor = *state.disc.get(&neighbor).unwrap_or(&usize::MAX);
                    let low_node = *state.low.get(node).unwrap_or(&usize::MAX);
                    state
                        .low
                        .insert(node.to_string(), low_node.min(disc_neighbor));
                }
            }
        }
    }

    let mut state = TarjanState {
        time: 0,
        disc: BTreeMap::new(),
        low: BTreeMap::new(),
        parent: BTreeMap::new(),
        articulation: BTreeSet::new(),
    };

    for node in nodes {
        if state.disc.contains_key(node) {
            continue;
        }
        state.parent.insert(node.clone(), None);
        dfs(node, undirected, &mut state);
    }
    state.articulation
}

#[cfg(test)]
mod tests {
    use super::{GraphEdge, GraphHealthInput, GraphHealthThresholds, lint_graph_health};

    #[test]
    fn graph_health_metrics_cover_four_classes_with_stable_output() {
        let input = GraphHealthInput {
            nodes: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                "e".into(),
                "x".into(),
                "y".into(),
                "z".into(),
            ],
            edges: vec![
                GraphEdge {
                    from: "a".into(),
                    to: "b".into(),
                },
                GraphEdge {
                    from: "a".into(),
                    to: "c".into(),
                },
                GraphEdge {
                    from: "a".into(),
                    to: "d".into(),
                },
                GraphEdge {
                    from: "b".into(),
                    to: "c".into(),
                },
                GraphEdge {
                    from: "c".into(),
                    to: "e".into(),
                },
                GraphEdge {
                    from: "x".into(),
                    to: "y".into(),
                },
            ],
        };
        let report = lint_graph_health(&input, &GraphHealthThresholds::default());
        assert_eq!(report.node_count, 8);
        assert_eq!(report.edge_count, 6);

        assert_eq!(report.hub_stub.len(), 1);
        assert_eq!(report.hub_stub[0].node, "a");
        assert_eq!(report.hub_stub[0].out_degree, 3);
        assert_eq!(report.hub_stub[0].in_degree, 0);

        let fragile_nodes = report
            .fragile_bridge
            .iter()
            .map(|item| item.node.as_str())
            .collect::<Vec<_>>();
        assert_eq!(fragile_nodes, vec!["a", "c"]);

        assert_eq!(report.isolated_community.len(), 1);
        assert_eq!(report.isolated_community[0].nodes, vec!["x", "y"]);
        assert_eq!(report.orphan, vec!["z"]);
    }

    #[test]
    fn graph_health_dedup_and_order_is_deterministic() {
        let input = GraphHealthInput {
            nodes: vec!["a".into(), "b".into(), "c".into()],
            edges: vec![
                GraphEdge {
                    from: "a".into(),
                    to: "b".into(),
                },
                GraphEdge {
                    from: "a".into(),
                    to: "b".into(),
                },
                GraphEdge {
                    from: "b".into(),
                    to: "c".into(),
                },
            ],
        };
        let first = lint_graph_health(&input, &GraphHealthThresholds::default());
        let second = lint_graph_health(&input, &GraphHealthThresholds::default());
        assert_eq!(first, second);
        assert_eq!(first.edge_count, 2);
        assert_eq!(first.fragile_bridge.len(), 1);
        assert_eq!(first.fragile_bridge[0].node, "b");
    }
}
