//! The installed actions as tool definitions: names, input schemas, effects
//! and a keyword search over them. Nothing here talks MCP; the server turns
//! entries into protocol types.
use itda_policy::{Effect, operation_effect};
use itda_spec::{Operation, Param, Spec};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// One callable action, named `<site>.<action>` as on the command line.
pub struct Entry {
    pub name: String,
    pub site: Arc<Spec>,
    pub action: String,
    pub description: String,
    pub effect: Effect,
    pub method: String,
    pub input_schema: Map<String, Value>,
}

pub struct Catalog {
    entries: Vec<Entry>,
}

impl Catalog {
    pub fn new(sites: Vec<Spec>) -> Self {
        let mut entries = Vec::new();
        for spec in sites.into_iter().map(Arc::new) {
            for op in &spec.operations {
                entries.push(Entry {
                    name: format!("{}.{}", spec.site.name, op.name),
                    site: spec.clone(),
                    action: op.name.clone(),
                    description: describe(&spec, op),
                    effect: operation_effect(op, &BTreeMap::new()),
                    method: op.method.clone(),
                    input_schema: input_schema(op),
                });
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Self { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Entries whose name or description contain the query's words, best
    /// match first. An empty query lists everything.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Entry> {
        let words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let mut scored: Vec<(usize, &Entry)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let name = entry.name.to_lowercase();
                let text = entry.description.to_lowercase();
                let score: usize = words
                    .iter()
                    .map(|w| {
                        3 * usize::from(name.contains(w.as_str()))
                            + usize::from(text.contains(w.as_str()))
                    })
                    .sum();
                (words.is_empty() || score > 0).then_some((score, entry))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(limit).map(|(_, e)| e).collect()
    }
}

fn describe(spec: &Spec, op: &Operation) -> String {
    let site = if spec.site.title.is_empty() {
        &spec.site.name
    } else {
        &spec.site.title
    };
    let what = if op.desc.is_empty() {
        format!("{} {}", op.method, op.path)
    } else {
        op.desc.clone()
    };
    format!("{what} ({site})")
}

/// JSON Schema for the action's inputs. Signed-in actions also take an
/// optional `account`; the agent only ever sees account ids.
fn input_schema(op: &Operation) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let inputs = op
        .params
        .iter()
        .map(|(n, p)| (n, p, true))
        .chain(op.query.iter().map(|(n, p)| (n, p, p.required)))
        .chain(
            op.body
                .iter()
                .flat_map(|b| b.inputs())
                .map(|(n, p)| (n, p, p.required)),
        );
    for (name, param, is_required) in inputs {
        properties.insert(name.clone(), param_schema(param));
        if is_required && param.default.is_none() && !required.contains(name) {
            required.push(name.clone());
        }
    }
    if !op.auth.is_empty() {
        properties.insert(
            "account".into(),
            json!({"type": "string", "description": "Account id to use; defaults to \"default\"."}),
        );
    }
    let mut schema = Map::new();
    schema.insert("type".into(), "object".into());
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("additionalProperties".into(), false.into());
    schema
}

fn param_schema(param: &Param) -> Value {
    let kind = match param.kind.as_str() {
        "int" | "integer" => "integer",
        "float" | "number" => "number",
        "bool" | "boolean" => "boolean",
        _ => "string",
    };
    let mut schema = json!({ "type": kind });
    if let Some(default) = &param.default {
        schema["default"] = default.clone();
    }
    schema
}
