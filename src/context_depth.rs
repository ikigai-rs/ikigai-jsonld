//! Bound term-definition recursion independently of JSON nesting.
//!
//! json-ld recursively defines referenced terms, prefixes and scoped contexts. A flat
//! context can therefore exhaust the stack even though its JSON is only two levels deep.
//! Build a conservative dependency graph before context processing: edges represent possible
//! recursive calls, not expanded IRI equivalence. The bound is independent of definition order
//! (even a dependency already defined earlier counts), and scoped definitions share the budget.
//! Wide contexts with independent terms have depth one. No JSON-LD values are rewritten.

use ikigai_core::{Error, Result};
use json_ld::syntax::{Object, Value};
use std::collections::HashMap;

/// Maximum potential term-definition call chain, including scoped contexts.
pub const MAX_CONTEXT_DEPENDENCY_DEPTH: usize = 64;

/// Check an already nesting-bounded JSON document or a bare context value.
pub(crate) fn check(value: &Value, arg: &str, context: bool) -> Result<()> {
    let mut graph = Graph::default();
    if context {
        graph.context(value, None);
    } else {
        graph.document(value);
    }
    graph.check(arg)
}

#[derive(Default)]
struct Graph {
    edges: Vec<Vec<usize>>,
}

impl Graph {
    fn document(&mut self, value: &Value) {
        match value {
            Value::Array(values) => {
                for value in values {
                    self.document(value);
                }
            }
            Value::Object(object) => {
                for entry in object {
                    match entry.key.as_str() {
                        "@context" => self.context(&entry.value, None),
                        // A JSON literal's contents are data, not contexts.
                        "@value" => (),
                        _ => self.document(&entry.value),
                    }
                }
            }
            _ => (),
        }
    }

    fn context(&mut self, value: &Value, parent: Option<usize>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    self.context(value, parent);
                }
            }
            Value::Object(object) => self.definition(object, parent),
            // Remote contexts and imports are refused by NoLoader, without recursion here.
            _ => (),
        }
    }

    fn definition(&mut self, object: &Object, parent: Option<usize>) {
        let mut terms = HashMap::new();
        for entry in object {
            let term = entry.key.as_str();
            if !term.starts_with('@') {
                let id = self.edges.len();
                self.edges.push(Vec::new());
                terms.insert(term, id);
                if let Some(parent) = parent {
                    self.edges[parent].push(id);
                }
            }
        }
        for entry in object {
            let term = entry.key.as_str();
            let Some(&id) = terms.get(term) else { continue };
            match &entry.value {
                Value::String(mapping) => {
                    if mapping.as_str() != term {
                        self.reference(id, mapping.as_str(), &terms);
                    }
                    self.prefix(id, term, &terms);
                }
                Value::Object(definition) => {
                    self.prefix(id, term, &terms);
                    for field in definition {
                        match field.key.as_str() {
                            "@id" | "@type" | "@reverse" => {
                                if let Some(mapping) = field.value.as_str() {
                                    // @id equal to the term is explicitly not expanded by
                                    // Create Term Definition; @type and @reverse still are.
                                    if field.key.as_str() != "@id" || mapping != term {
                                        self.reference(id, mapping, &terms);
                                    }
                                }
                            }
                            "@context" => self.context(&field.value, Some(id)),
                            _ => (),
                        }
                    }
                }
                _ => (),
            }
        }
    }

    fn reference(&mut self, id: usize, value: &str, terms: &HashMap<&str, usize>) {
        if value.starts_with('@') {
            return;
        }
        if let Some(&dependency) = terms.get(value) {
            self.edges[id].push(dependency);
            return; // With vocab=true, an exact term mapping wins over its prefix.
        }
        self.prefix(id, value, terms);
    }

    fn prefix(&mut self, id: usize, value: &str, terms: &HashMap<&str, usize>) {
        if let Some((prefix, suffix)) = value.split_once(':') {
            if !prefix.is_empty() && prefix != "_" && !suffix.starts_with("//") {
                if let Some(&dependency) = terms.get(prefix) {
                    self.edges[id].push(dependency);
                }
            }
        }
    }

    fn check(&self, arg: &str) -> Result<()> {
        // Iterative Tarjan traversal. Collapse cycles first: small cycles are left to the
        // JSON-LD processor to diagnose, while a large cycle must not reach its call stack.
        // Weight each component by its size, an upper bound on a simple call path inside it.
        let n = self.edges.len();
        let mut index = vec![None; n];
        let mut low = vec![0; n];
        let mut pending = vec![false; n];
        let mut component = vec![usize::MAX; n];
        let mut members = Vec::new();
        let mut components: Vec<Vec<usize>> = Vec::new();
        let mut serial = 0;
        for root in 0..n {
            if index[root].is_some() {
                continue;
            }
            index[root] = Some(serial);
            low[root] = serial;
            serial += 1;
            members.push(root);
            pending[root] = true;
            let mut stack = vec![(root, 0)];
            while let Some(&mut (node, ref mut next)) = stack.last_mut() {
                if let Some(&child) = self.edges[node].get(*next) {
                    *next += 1;
                    if let Some(child_index) = index[child] {
                        if pending[child] {
                            low[node] = low[node].min(child_index);
                        }
                    } else {
                        index[child] = Some(serial);
                        low[child] = serial;
                        serial += 1;
                        members.push(child);
                        pending[child] = true;
                        stack.push((child, 0));
                    }
                } else {
                    stack.pop();
                    if let Some(&(parent, _)) = stack.last() {
                        low[parent] = low[parent].min(low[node]);
                    }
                    if low[node] == index[node].unwrap() {
                        let mut group = Vec::new();
                        loop {
                            let member = members.pop().unwrap();
                            pending[member] = false;
                            component[member] = components.len();
                            group.push(member);
                            if member == node {
                                break;
                            }
                        }
                        if group.len() > MAX_CONTEXT_DEPENDENCY_DEPTH {
                            return Err(refusal(arg));
                        }
                        components.push(group);
                    }
                }
            }
        }
        // Tarjan emits sinks first, so every dependency outside a component already has a
        // depth. Shared structure is visited once, rather than once per possible call path.
        let mut depths = vec![0; components.len()];
        for (id, group) in components.iter().enumerate() {
            let mut dependency_depth = 0;
            for &node in group {
                for &child in &self.edges[node] {
                    let dependency = component[child];
                    if dependency != id {
                        dependency_depth = dependency_depth.max(depths[dependency]);
                    }
                }
            }
            depths[id] = group.len() + dependency_depth;
            if depths[id] > MAX_CONTEXT_DEPENDENCY_DEPTH {
                return Err(refusal(arg));
            }
        }
        Ok(())
    }
}

fn refusal(arg: &str) -> Error {
    Error::InvalidArgument {
        name: arg.to_string(),
        detail: format!(
            "the JSON-LD context has a potential term-definition dependency chain deeper than \
             {MAX_CONTEXT_DEPENDENCY_DEPTH} (MAX_CONTEXT_DEPENDENCY_DEPTH); use direct IRI \
             mappings to shorten term dependencies"
        ),
    }
}
