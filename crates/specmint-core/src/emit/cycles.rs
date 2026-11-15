//! Schema `$ref` graph + cycle detection (Kubb `circularNames` / `hasIndirectRef`).
//!
//! Factories use this instead of a numeric `maxDepth`: acyclic trees materialize
//! fully; only real cycles are broken (self-ref → `undefined`, mutual → lazy getter).
//!
//! Cycle SCCs and reachability use [`petgraph`]; the adjacency list stays an
//! [`IndexMap`] for stable IR ordering.

use indexmap::{IndexMap, IndexSet};
use petgraph::algo::{has_path_connecting, tarjan_scc};
use petgraph::graphmap::DiGraphMap;

use crate::adapter::SchemaNode;
use crate::emit::faker_expr::SchemaMap;

pub type RefGraph = IndexMap<String, IndexSet<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleKind {
    /// No cycle through this property relative to the factory root.
    None,
    /// Property (or array item) refs the factory being generated.
    SelfRef,
    /// Property refs another schema that can reach back to the root.
    Mutual,
}

/// Direct `$ref` names appearing anywhere under `schema` (one schema body only).
pub fn collect_refs(schema: &SchemaNode) -> IndexSet<String> {
    let mut out = IndexSet::new();
    collect_refs_into(schema, &mut out);
    out
}

fn collect_refs_into(schema: &SchemaNode, out: &mut IndexSet<String>) {
    match schema {
        SchemaNode::Ref { name } => {
            out.insert(name.clone());
        }
        SchemaNode::Object {
            properties,
            additional_properties,
            ..
        } => {
            for prop in properties.values() {
                collect_refs_into(prop, out);
            }
            if let Some(ap) = additional_properties {
                collect_refs_into(ap, out);
            }
        }
        SchemaNode::Array { items, .. } => collect_refs_into(items, out),
        SchemaNode::AllOf { items, .. }
        | SchemaNode::OneOf { items, .. }
        | SchemaNode::AnyOf { items, .. } => {
            for item in items {
                collect_refs_into(item, out);
            }
        }
        SchemaNode::Enum { .. } | SchemaNode::Primitive { .. } | SchemaNode::Unknown { .. } => {}
    }
}

pub fn build_ref_graph(schemas: &SchemaMap) -> RefGraph {
    let mut graph = RefGraph::new();
    for (name, schema) in schemas {
        graph.insert(name.clone(), collect_refs(schema));
    }
    graph
}

fn as_digraph(graph: &RefGraph) -> DiGraphMap<&str, ()> {
    let mut g = DiGraphMap::new();
    for (from, edges) in graph {
        g.add_node(from.as_str());
        for to in edges {
            g.add_edge(from.as_str(), to.as_str(), ());
        }
    }
    g
}

/// Schemas that participate in at least one cycle in the ref graph.
pub fn find_circular_names(graph: &RefGraph) -> IndexSet<String> {
    let g = as_digraph(graph);
    let mut circular = IndexSet::new();
    for scc in tarjan_scc(&g) {
        if scc.len() > 1 {
            for name in scc {
                circular.insert(name.to_string());
            }
        } else if let Some(name) = scc.first() {
            // Self-loop counts as a cycle (single-node SCC).
            if g.contains_edge(*name, *name) {
                circular.insert((*name).to_string());
            }
        }
    }
    circular
}

pub fn can_reach(graph: &RefGraph, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    has_path_connecting(&as_digraph(graph), from, to, None)
}

/// Follow alias refs (`Foo: { $ref: Bar }`) to the concrete schema name.
pub fn resolve_ref_name<'a>(name: &'a str, schemas: &'a SchemaMap) -> &'a str {
    let mut current = name;
    let mut guard = 0;
    while let Some(SchemaNode::Ref { name: next }) = schemas.get(current) {
        current = next.as_str();
        guard += 1;
        if guard > 32 {
            break;
        }
    }
    current
}

/// How `prop_schema` should be emitted inside factory `root`.
pub fn property_cycle_kind(
    root: &str,
    prop_schema: &SchemaNode,
    schemas: &SchemaMap,
    graph: &RefGraph,
    circular: &IndexSet<String>,
) -> CycleKind {
    let refs = collect_refs(prop_schema);
    if refs.is_empty() {
        return CycleKind::None;
    }

    let mut saw_mutual = false;
    for name in &refs {
        let resolved = resolve_ref_name(name, schemas);
        if resolved == root {
            return CycleKind::SelfRef;
        }
        if circular.contains(resolved) && can_reach(graph, resolved, root) {
            saw_mutual = true;
        }
    }
    if saw_mutual {
        CycleKind::Mutual
    } else {
        CycleKind::None
    }
}

pub fn schema_is_array(schema: &SchemaNode) -> bool {
    match schema {
        SchemaNode::Array { .. } => true,
        SchemaNode::AllOf { items, .. } => items.len() == 1 && schema_is_array(&items[0]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::SchemaNode;
    use indexmap::IndexMap;

    fn obj_with_ref(prop: &str, target: &str) -> SchemaNode {
        let mut properties = IndexMap::new();
        properties.insert(
            prop.into(),
            SchemaNode::Ref {
                name: target.into(),
            },
        );
        SchemaNode::Object {
            properties,
            required: vec![],
            readonly_properties: vec![],
            writeonly_properties: vec![],
            additional_properties: None,
            nullable: false,
            description: None,
        }
    }

    #[test]
    fn detects_mutual_cycle() {
        let mut schemas = SchemaMap::new();
        schemas.insert("A".into(), obj_with_ref("b", "B"));
        schemas.insert("B".into(), obj_with_ref("a", "A"));
        let graph = build_ref_graph(&schemas);
        let circular = find_circular_names(&graph);
        assert!(circular.contains("A"));
        assert!(circular.contains("B"));
        // Property `b` on A
        let b_prop = match schemas.get("A").unwrap() {
            SchemaNode::Object { properties, .. } => properties.get("b").unwrap(),
            _ => panic!(),
        };
        assert_eq!(
            property_cycle_kind("A", b_prop, &schemas, &graph, &circular),
            CycleKind::Mutual
        );
    }

    #[test]
    fn detects_self_ref() {
        let mut schemas = SchemaMap::new();
        schemas.insert("Node".into(), obj_with_ref("child", "Node"));
        let graph = build_ref_graph(&schemas);
        let circular = find_circular_names(&graph);
        assert!(circular.contains("Node"));
        let child = match schemas.get("Node").unwrap() {
            SchemaNode::Object { properties, .. } => properties.get("child").unwrap(),
            _ => panic!(),
        };
        assert_eq!(
            property_cycle_kind("Node", child, &schemas, &graph, &circular),
            CycleKind::SelfRef
        );
    }

    #[test]
    fn acyclic_chain_is_not_circular() {
        let mut schemas = SchemaMap::new();
        schemas.insert("Org".into(), obj_with_ref("product", "Product"));
        schemas.insert("Product".into(), obj_with_ref("country", "Country"));
        schemas.insert(
            "Country".into(),
            SchemaNode::Object {
                properties: IndexMap::new(),
                required: vec![],
                readonly_properties: vec![],
                writeonly_properties: vec![],
                additional_properties: None,
                nullable: false,
                description: None,
            },
        );
        let graph = build_ref_graph(&schemas);
        let circular = find_circular_names(&graph);
        assert!(circular.is_empty());
        let product = match schemas.get("Org").unwrap() {
            SchemaNode::Object { properties, .. } => properties.get("product").unwrap(),
            _ => panic!(),
        };
        assert_eq!(
            property_cycle_kind("Org", product, &schemas, &graph, &circular),
            CycleKind::None
        );
    }
}
