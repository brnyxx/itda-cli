//! Fill `confirm` on a drafted change when the draft itself shows the read
//! that would list it: a `POST` whose list read is on the same path prefix
//! (`POST /api/notices` and `GET /api/notices`) and a string input of the
//! change that the read's records carry under the same name (`title`). Only
//! when that evidence is there; otherwise the change keeps no `confirm` and
//! a person may add one.
use serde_norway::{Mapping, Value as Yaml};

/// Add `confirm` to every change the operations give evidence for, and say
/// so in its `evidence`. `models` is the draft's `models` block, whose
/// fields are what a typed read's records carry.
pub(crate) fn fill(operations: &mut [Mapping], models: &Mapping) {
    let reads: Vec<ListRead> = operations
        .iter()
        .filter_map(|op| ListRead::of(op, models))
        .collect();
    for op in operations.iter_mut() {
        if text(op, "effect") != "write"
            || !text(op, "method").eq_ignore_ascii_case("POST")
            || op.contains_key("confirm")
        {
            continue;
        }
        let path = text(op, "path").to_owned();
        let base = text(op, "base_url").to_owned();
        let inputs = string_inputs(op);
        let found = reads
            .iter()
            .filter(|read| read.base == base && read.covers(&path))
            .find_map(|read| {
                inputs
                    .iter()
                    .find(|input| read.fields.iter().any(|f| f.eq_ignore_ascii_case(input)))
                    .map(|input| (read, input.clone()))
            });
        let Some((read, input)) = found else {
            continue;
        };
        let field = read
            .fields
            .iter()
            .find(|f| f.eq_ignore_ascii_case(&input))
            .cloned()
            .unwrap_or_else(|| input.clone());
        let mut matching = Mapping::new();
        matching.insert("pointer".into(), format!("{}/{field}", read.pointer).into());
        matching.insert("from".into(), format!("body.{input}").into());
        let mut confirm = Mapping::new();
        confirm.insert("read".into(), read.name.clone().into());
        confirm.insert("match".into(), Yaml::Mapping(matching));
        let after = if op.contains_key("response") {
            "response"
        } else {
            "body"
        };
        crate::insert_after(op, after, "confirm", Yaml::Mapping(confirm));
        let evidence = format!(
            "{}; GET {} lists it under {field}, so {} confirms it after a lost answer",
            text(op, "evidence"),
            read.path,
            read.name
        );
        op.insert(
            "evidence".into(),
            evidence.trim_start_matches("; ").to_owned().into(),
        );
    }
}

/// A read that lists records with named fields, runnable on its defaults.
struct ListRead {
    name: String,
    base: String,
    path: String,
    /// The pointer to one record's field, less the field: `/items/*` for
    /// HTML records, `/*` for a JSON list.
    pointer: &'static str,
    fields: Vec<String>,
}

impl ListRead {
    fn of(op: &Mapping, models: &Mapping) -> Option<Self> {
        if text(op, "effect") != "read" || !matches!(text(op, "method"), "" | "GET") {
            return None;
        }
        if op.contains_key("params") || needs_input(op) {
            return None;
        }
        let response = op.get("response")?.as_mapping()?;
        let (pointer, fields) = if let Some(html) = response.get("html").and_then(Yaml::as_mapping)
            && !text(html, "items").is_empty()
        {
            ("/items/*", keys(html.get("fields")))
        } else if let Some(model) = op.get("model").and_then(Yaml::as_str)
            && lists(response)
        {
            let fields = keys(models.get(model));
            if fields.is_empty() {
                return None;
            }
            ("/*", fields)
        } else {
            let fields: Vec<String> = response
                .get("required_pointers")
                .and_then(Yaml::as_sequence)
                .into_iter()
                .flatten()
                .filter_map(Yaml::as_str)
                .filter_map(|p| p.strip_prefix("/0/"))
                .filter(|rest| !rest.contains('/'))
                .map(str::to_owned)
                .collect();
            if fields.is_empty() {
                return None;
            }
            ("/*", fields)
        };
        Some(Self {
            name: text(op, "name").to_owned(),
            base: text(op, "base_url").to_owned(),
            path: text(op, "path").to_owned(),
            pointer,
            fields,
        })
    }

    /// The read lists what `path` posts to: the same path, or a path under it
    /// (`/issues/new` posts what `/issues` lists).
    fn covers(&self, path: &str) -> bool {
        let own = self.path.trim_end_matches('/');
        path.trim_end_matches('/') == own
            || path
                .strip_prefix(own)
                .is_some_and(|rest| rest.starts_with('/') && !own.is_empty())
    }
}

/// A typed read returns a list when it extracts one or checks a first
/// record; its records then carry the fields the model keeps.
fn lists(response: &Mapping) -> bool {
    !text(response, "extract_pointer").is_empty()
        || response
            .get("required_pointers")
            .and_then(Yaml::as_sequence)
            .is_some_and(|ps| {
                ps.iter()
                    .filter_map(Yaml::as_str)
                    .any(|p| p.starts_with("/0/"))
            })
}

/// The change's inputs an agent gives and the read could show: string
/// inputs that are required or have a plain default, in `body.form` or
/// `body.json`, `title` first.
fn string_inputs(op: &Mapping) -> Vec<String> {
    let body = op.get("body").and_then(Yaml::as_mapping);
    let mut out: Vec<String> = ["form", "json"]
        .iter()
        .filter_map(|key| body?.get(*key)?.as_mapping())
        .flat_map(|inputs| inputs.iter())
        .filter_map(|(name, param)| {
            let param = param.as_mapping()?;
            let name = name.as_str()?;
            let kind = param.get("type").and_then(Yaml::as_str).unwrap_or("string");
            let required = param.get("required").and_then(Yaml::as_bool) == Some(true);
            let default = param.get("default").and_then(Yaml::as_str);
            let from_step = default.is_some_and(|d| d.trim_start().starts_with("{{step."));
            (kind == "string" && (required || default.is_some()) && !from_step)
                .then(|| name.to_owned())
        })
        .collect();
    out.sort_by_key(|name| (name != "title", name.clone()));
    out
}

/// True when the read has an input it cannot run without.
fn needs_input(op: &Mapping) -> bool {
    let body = op.get("body").and_then(Yaml::as_mapping);
    op.get("query")
        .and_then(Yaml::as_mapping)
        .into_iter()
        .chain(
            ["form", "json"]
                .iter()
                .filter_map(|key| body?.get(*key)?.as_mapping()),
        )
        .flat_map(|inputs| inputs.values())
        .filter_map(Yaml::as_mapping)
        .any(|param| {
            param.get("required").and_then(Yaml::as_bool) == Some(true)
                && !param.contains_key("default")
        })
}

fn keys(value: Option<&Yaml>) -> Vec<String> {
    value
        .and_then(Yaml::as_mapping)
        .into_iter()
        .flat_map(|m| m.keys())
        .filter_map(Yaml::as_str)
        .map(str::to_owned)
        .collect()
}

fn text<'a>(map: &'a Mapping, key: &str) -> &'a str {
    map.get(key).and_then(Yaml::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(yaml: &str) -> Mapping {
        serde_norway::from_str(yaml).unwrap_or_default()
    }

    #[test]
    fn a_post_under_a_list_read_with_a_shared_field_is_confirmed() {
        let mut ops = vec![
            op(
                "{name: list-notices, method: GET, path: /api/notices, effect: read, evidence: doc, query: {page: {type: int, default: 1}}, response: {format: json, required_pointers: [/0/id, /0/title]}}",
            ),
            op(
                "{name: create-notice, method: POST, path: /api/notices/new, effect: write, evidence: doc, body: {json: {body: {type: string}, title: {type: string, required: true}}}}",
            ),
            op(
                "{name: list-items, method: GET, path: /items, effect: read, evidence: doc, response: {format: json, required_pointers: [/items]}}",
            ),
            op(
                "{name: create-item, method: POST, path: /items, effect: write, evidence: doc, body: {json: {title: {type: string, required: true}}}}",
            ),
            op(
                "{name: search, method: GET, path: /api/notices/search, effect: read, evidence: doc, query: {q: {type: string, required: true}}, response: {format: json, required_pointers: [/0/title]}}",
            ),
        ];
        fill(&mut ops, &Mapping::new());
        let confirm = ops[1].get("confirm").unwrap();
        assert_eq!(confirm["read"].as_str(), Some("list-notices"));
        assert_eq!(confirm["match"]["pointer"].as_str(), Some("/*/title"));
        assert_eq!(confirm["match"]["from"].as_str(), Some("body.title"));
        assert!(
            text(&ops[1], "evidence").contains("GET /api/notices lists it under title"),
            "{}",
            text(&ops[1], "evidence")
        );
        // The list read says nothing about its records: no evidence, no confirm.
        assert!(!ops[3].contains_key("confirm"));
    }

    #[test]
    fn html_records_and_a_list_model_name_their_fields() {
        let mut ops = vec![
            op(
                "{name: list-notices, method: GET, path: /notices, effect: read, evidence: doc, response: {format: html, html: {items: tr.notice, fields: {title: {selector: td.title}}}}}",
            ),
            op(
                "{name: post-notice, method: POST, path: /notices, effect: write, evidence: doc, body: {form: {title: {type: string, required: true}, _csrf: {type: string, default: '{{step.form.csrf}}'}}}}",
            ),
            op(
                "{name: list-posts, method: GET, path: /, effect: read, evidence: doc, model: post, response: {format: json, required_pointers: [/0/id]}}",
            ),
            op(
                "{name: create-post, method: POST, path: /, effect: write, evidence: doc, body: {json: {title: {type: string, required: true}, post: {type: string, required: true}}}}",
            ),
        ];
        let models = op("{post: {id: {path: id, type: int}, title: {path: title.rendered}}}");
        fill(&mut ops, &models);
        assert_eq!(
            ops[1]["confirm"]["match"]["pointer"].as_str(),
            Some("/items/*/title")
        );
        // The model's fields, not its name, are what the records carry.
        assert_eq!(
            ops[3]["confirm"]["match"]["pointer"].as_str(),
            Some("/*/title")
        );
        assert_eq!(
            ops[3]["confirm"]["match"]["from"].as_str(),
            Some("body.title")
        );
        // Without the model block a typed read says nothing about its records.
        let mut post = ops[3].clone();
        post.remove("confirm");
        let mut again = vec![ops[2].clone(), post];
        fill(&mut again, &Mapping::new());
        assert!(!again[1].contains_key("confirm"));
    }

    #[test]
    fn reads_that_need_an_input_or_live_elsewhere_do_not_confirm() {
        let mut ops = vec![
            op(
                "{name: one, method: GET, path: /notices/{id}, effect: read, evidence: doc, params: {id: {type: int}}, response: {format: json, required_pointers: [/0/title]}}",
            ),
            op(
                "{name: list, method: GET, path: /notices, base_url: https://api.example, effect: read, evidence: doc, response: {format: json, required_pointers: [/0/title]}}",
            ),
            op(
                "{name: post-notice, method: POST, path: /notices, effect: write, evidence: doc, body: {form: {title: {type: string, required: true}}}}",
            ),
            op(
                "{name: post-other, method: POST, path: /other, effect: write, evidence: doc, body: {form: {title: {type: string}}}}",
            ),
        ];
        fill(&mut ops, &Mapping::new());
        assert!(ops.iter().all(|op| !op.contains_key("confirm")));
    }
}
