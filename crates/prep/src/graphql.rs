//! What a GraphQL endpoint says it can do, as reads.
//!
//! A draft that sees a GraphQL endpoint (a recorded call to one, or a page
//! naming a same-origin `/graphql` path) asks it once with the standard
//! introspection query. The caller sends that query with the honest client
//! and hands the answer in as a [`Schema`]; nothing here reaches the network.
//! Each root `Query` field whose arguments are scalars, enums or ids becomes
//! a read: the document is fixed, the arguments are typed inputs, and the
//! selection is the field's scalars, two levels deep, under a cap. Fields
//! under `Mutation` are listed in the notes and never drafted: a schema says
//! what a change takes, not what it does.
use crate::har::{Kind, graphql_operation, unique};
use crate::label::{Label, describe};
use crate::page::Page;
use crate::{reorder, slug};
use itda_types::util::secret_field;
use serde_json::Value;
use serde_norway::{Mapping, Value as Yaml};
use std::collections::{BTreeMap, BTreeSet};

/// The standard introspection query, as GraphQL clients send it.
pub const INTROSPECTION_QUERY: &str = "query IntrospectionQuery { __schema { queryType { name } mutationType { name } subscriptionType { name } types { ...FullType } directives { name description locations args { ...InputValue } } } } fragment FullType on __Type { kind name description fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason } inputFields { ...InputValue } interfaces { ...TypeRef } enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason } possibleTypes { ...TypeRef } } fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue } fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } } }";

/// How many fields one drafted read selects at most.
pub const FIELD_CAP: usize = 24;
/// How many reads one schema drafts at most.
pub const OPERATION_CAP: usize = 100;
/// How many endpoints one draft asks at most.
pub const ENDPOINT_CAP: usize = 3;

/// What one endpoint answered the introspection query with.
#[derive(Clone, Debug)]
pub struct Schema {
    pub url: String,
    pub json: Value,
}

/// The JSON body of the introspection request: the document and its name,
/// no variables.
pub fn introspection_body() -> Vec<u8> {
    serde_json::json!({
        "query": INTROSPECTION_QUERY,
        "operationName": "IntrospectionQuery",
    })
    .to_string()
    .into_bytes()
}

/// The endpoints a recording sent GraphQL documents to and got answers
/// from, each once, without query or fragment.
pub fn endpoints_in_recording(text: &[u8]) -> Vec<String> {
    let doc: Value = serde_json::from_slice(text).unwrap_or_default();
    let mut out = Vec::new();
    for entry in doc["log"]["entries"].as_array().into_iter().flatten() {
        let answered = entry["response"]["status"]
            .as_u64()
            .is_some_and(|s| (200..300).contains(&s));
        if entry["request"]["method"].as_str() != Some("POST")
            || !answered
            || graphql_operation(&entry["request"]["postData"]).is_none()
        {
            continue;
        }
        let Some(mut url) = entry["request"]["url"]
            .as_str()
            .and_then(|u| url::Url::parse(u).ok())
            .filter(|u| matches!(u.scheme(), "http" | "https"))
        else {
            continue;
        };
        url.set_query(None);
        url.set_fragment(None);
        let url = url.to_string();
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// The same-origin `/graphql` paths a page's HTML names, each once, as full
/// addresses: `fetch('/graphql')`, `"uri": "https://site/api/graphql"`.
/// Script files (`graphql.min.js`) are not endpoints.
pub fn endpoints_in_page(page: &Page) -> Vec<String> {
    let Ok(base) = url::Url::parse(&page.url) else {
        return Vec::new();
    };
    let origin = base.origin();
    let lower = page.html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let url_char = |b: u8| b.is_ascii_alphanumeric() || b":/.-_~%@+".contains(&b);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find("graphql") {
        let at = from + found;
        from = at + "graphql".len();
        let mut start = at;
        while start > 0 && url_char(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = from;
        while end < bytes.len() && url_char(bytes[end]) {
            end += 1;
        }
        let token = page.html[start..end].trim_end_matches(['.', ':']);
        let path = if token.starts_with("http://") || token.starts_with("https://") {
            match url::Url::parse(token) {
                Ok(url) if url.origin() == origin => url.path().to_owned(),
                _ => continue,
            }
        } else if token.starts_with('/') && !token.starts_with("//") {
            token.to_owned()
        } else {
            continue;
        };
        let path = path.trim_end_matches('/');
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let endpoint = segments.iter().any(|s| s.eq_ignore_ascii_case("graphql"))
            && segments.last().is_some_and(|last| !last.contains('.'));
        if !endpoint {
            continue;
        }
        let url = format!("{}{path}", origin.ascii_serialization());
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// A type reference with its wrappers taken off.
struct Shape<'a> {
    kind: &'a str,
    name: &'a str,
    list: bool,
    required: bool,
}

fn shape(type_ref: &Value) -> Shape<'_> {
    let mut current = type_ref;
    let required = current["kind"].as_str() == Some("NON_NULL");
    let mut list = false;
    for _ in 0..8 {
        match current["kind"].as_str() {
            Some("NON_NULL") => current = &current["ofType"],
            Some("LIST") => {
                list = true;
                current = &current["ofType"];
            }
            _ => break,
        }
    }
    Shape {
        kind: current["kind"].as_str().unwrap_or_default(),
        name: current["name"].as_str().unwrap_or_default(),
        list,
        required,
    }
}

fn leaf(kind: &str) -> bool {
    matches!(kind, "SCALAR" | "ENUM")
}

fn fields(type_def: &Value) -> impl Iterator<Item = &Value> {
    type_def["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["isDeprecated"] != true)
        .filter(|f| f["name"].as_str().is_some_and(|n| !n.starts_with("__")))
}

fn args(field: &Value) -> impl Iterator<Item = &Value> {
    field["args"].as_array().into_iter().flatten()
}

fn needs_input(field: &Value) -> bool {
    args(field).any(|a| shape(&a["type"]).required && a["defaultValue"].is_null())
}

/// The scalars of `type_name` and, one level down, of the objects it holds
/// that need no input, as a selection set. `None` when it has no scalar
/// to read.
fn selection(types: &BTreeMap<&str, &Value>, type_name: &str) -> Option<String> {
    let type_def = types.get(type_name)?;
    let mut parts: Vec<String> = Vec::new();
    let mut count = 0;
    for field in fields(type_def) {
        if count == FIELD_CAP {
            break;
        }
        let returns = shape(&field["type"]);
        if leaf(returns.kind) && !needs_input(field) {
            parts.push(field["name"].as_str().unwrap_or_default().to_owned());
            count += 1;
        }
    }
    for field in fields(type_def) {
        if count == FIELD_CAP {
            break;
        }
        let returns = shape(&field["type"]);
        if !matches!(returns.kind, "OBJECT" | "INTERFACE") || needs_input(field) {
            continue;
        }
        let Some(inner) = types.get(returns.name) else {
            continue;
        };
        let leaves: Vec<&str> = fields(inner)
            .filter(|f| leaf(shape(&f["type"]).kind) && !needs_input(f))
            .filter_map(|f| f["name"].as_str())
            .take(FIELD_CAP - count)
            .collect();
        if leaves.is_empty() {
            continue;
        }
        count += leaves.len();
        parts.push(format!(
            "{} {{ {} }}",
            field["name"].as_str().unwrap_or_default(),
            leaves.join(" ")
        ));
    }
    (!parts.is_empty()).then(|| format!("{{ {} }}", parts.join(" ")))
}

/// True when `type_name` has a `pageInfo` field whose type has `endCursor`
/// and `hasNextPage`.
fn page_info(types: &BTreeMap<&str, &Value>, type_name: &str) -> bool {
    let Some(type_def) = types.get(type_name) else {
        return false;
    };
    fields(type_def)
        .find(|f| f["name"].as_str() == Some("pageInfo"))
        .and_then(|f| types.get(shape(&f["type"]).name))
        .is_some_and(|info| {
            let names: Vec<&str> = fields(info).filter_map(|f| f["name"].as_str()).collect();
            names.contains(&"endCursor") && names.contains(&"hasNextPage")
        })
}

fn kind_of(name: &str) -> Kind {
    match name {
        "Int" => Kind::Int,
        "Float" => Kind::Float,
        "Boolean" => Kind::Bool,
        _ => Kind::Str,
    }
}

/// `countries` becomes the operation name `Countries`.
fn operation_name(field: &str) -> String {
    let mut chars = field.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => "Read".into(),
    }
}

/// The schema's root `Query` fields as reads at `path`, with what was left
/// out, and the `Mutation` fields, in `notes`. `include` keeps only fields
/// whose name contains it, when `path` does not.
pub(crate) fn operations(
    schema: &Schema,
    path: &str,
    lang: &str,
    include: Option<&str>,
    names: &mut BTreeSet<String>,
    notes: &mut Vec<String>,
) -> Vec<Mapping> {
    let root = match &schema.json["data"]["__schema"] {
        Value::Object(_) => &schema.json["data"]["__schema"],
        _ => &schema.json["__schema"],
    };
    let Some(all) = root["types"].as_array() else {
        return Vec::new();
    };
    let types: BTreeMap<&str, &Value> = all
        .iter()
        .filter_map(|t| t["name"].as_str().map(|name| (name, t)))
        .collect();
    let query_type = root["queryType"]["name"].as_str().unwrap_or("Query");
    let Some(query) = types.get(query_type) else {
        return Vec::new();
    };
    let wanted = |name: &str| {
        include.is_none_or(|text| {
            let text = text.to_lowercase();
            path.to_lowercase().contains(&text) || name.to_lowercase().contains(&text)
        })
    };
    let mut out = Vec::new();
    let mut complex = Vec::new();
    let mut dropped = Vec::new();
    let mut credentials = Vec::new();
    let mut changes_by_name = Vec::new();
    let mut nothing_to_read = Vec::new();
    let mut left_out = 0;
    for field in fields(query) {
        let name = field["name"].as_str().unwrap_or_default();
        if !wanted(name) {
            continue;
        }
        if itda_policy::dangerous_intent(name) {
            changes_by_name.push(name.to_owned());
            continue;
        }
        let mut variables: Vec<Variable> = Vec::new();
        let mut unsupported = false;
        for arg in args(field) {
            let arg_name = arg["name"].as_str().unwrap_or_default();
            let kind = shape(&arg["type"]);
            let required = kind.required && arg["defaultValue"].is_null();
            if secret_field(arg_name) {
                if required {
                    unsupported = true;
                    credentials.push(name.to_owned());
                    break;
                }
                continue;
            }
            if kind.list || !leaf(kind.kind) {
                if required {
                    unsupported = true;
                    complex.push(name.to_owned());
                    break;
                }
                dropped.push(format!("{name} ({arg_name})"));
                continue;
            }
            variables.push(Variable {
                argument: arg_name.to_owned(),
                input: input_name(name, arg_name),
                type_name: kind.name.to_owned(),
                kind: kind_of(kind.name),
                required,
            });
        }
        if unsupported {
            continue;
        }
        let returns = shape(&field["type"]);
        let mut selected = if leaf(returns.kind) {
            Some(String::new())
        } else if matches!(returns.kind, "OBJECT" | "INTERFACE") {
            selection(&types, returns.name)
        } else {
            None
        };
        // A connection: an `after` cursor in, `pageInfo { endCursor
        // hasNextPage }` out. The cursor is always selected so the read can
        // turn pages.
        let connection = variables
            .iter()
            .any(|v| v.argument == "after" && v.type_name == "String")
            && matches!(returns.kind, "OBJECT" | "INTERFACE")
            && page_info(&types, returns.name);
        if connection
            && let Some(chosen) = &mut selected
            && !chosen.contains("pageInfo")
        {
            chosen.insert_str(chosen.len() - 1, "pageInfo { endCursor hasNextPage } ");
        }
        let Some(selected) = selected else {
            nothing_to_read.push(name.to_owned());
            continue;
        };
        if out.len() == OPERATION_CAP {
            left_out += 1;
            continue;
        }
        let mut entry = read(name, &variables, &selected, path, query_type, lang, names);
        if connection {
            let after = variables
                .iter()
                .find(|v| v.argument == "after")
                .map(|v| v.input.as_str())
                .unwrap_or("after");
            entry.insert(
                "pagination".into(),
                crate::pagination_yaml(
                    "cursor",
                    &format!("/data/{name}/pageInfo/endCursor"),
                    &format!("/data/{name}/pageInfo/hasNextPage"),
                    after,
                    true,
                ),
            );
            entry = reorder(entry);
        }
        out.push(entry);
    }
    let list = |items: &[String]| {
        let shown: Vec<&str> = items.iter().take(8).map(String::as_str).collect();
        if items.len() > shown.len() {
            format!(
                "{} and {} more",
                shown.join(", "),
                items.len() - shown.len()
            )
        } else {
            shown.join(", ")
        }
    };
    if !complex.is_empty() {
        notes.push(format!(
            "{path}: left out {}; they need an argument that holds an object or a list. Add them by hand or record the site.",
            list(&complex)
        ));
    }
    if !dropped.is_empty() {
        notes.push(format!(
            "{path}: drafted {} without the argument in brackets, which holds an object or a list; add it to json_template by hand if agents need it.",
            list(&dropped)
        ));
    }
    if !credentials.is_empty() {
        notes.push(format!(
            "{path}: left out {}; they take a credential, which comes from sign-in, never from the agent.",
            list(&credentials)
        ));
    }
    if !changes_by_name.is_empty() {
        notes.push(format!(
            "{path}: left out {}; the name says it changes something. Record the site to draft it as a change.",
            list(&changes_by_name)
        ));
    }
    if !nothing_to_read.is_empty() {
        notes.push(format!(
            "{path}: left out {}; no plain field to read was found under it.",
            list(&nothing_to_read)
        ));
    }
    if left_out > 0 {
        notes.push(format!(
            "{path}: the schema lists {left_out} more reads than a draft holds ({OPERATION_CAP}); narrow it with --include."
        ));
    }
    if let Some(mutation_type) = root["mutationType"]["name"].as_str()
        && let Some(mutation) = types.get(mutation_type)
    {
        let changes: Vec<String> = fields(mutation)
            .filter_map(|f| f["name"].as_str().map(str::to_owned))
            .collect();
        if !changes.is_empty() {
            notes.push(format!(
                "{path}: {mutation_type} lists {} changes ({}). A schema does not say what a change does, so none is drafted; record yourself using the site to draft the ones you need.",
                changes.len(),
                list(&changes)
            ));
        }
    }
    out
}

/// One argument of a field as an input. The input is named after the
/// argument, except where that name is taken by the document itself.
struct Variable {
    argument: String,
    input: String,
    type_name: String,
    kind: Kind,
    required: bool,
}

/// `search(query: String!)` takes its text as `search_query`: `query` and
/// `operationName` are the inputs that hold the document.
fn input_name(field: &str, argument: &str) -> String {
    if matches!(argument, "query" | "operationName") {
        format!("{field}_{argument}")
    } else {
        argument.to_owned()
    }
}

fn read(
    field: &str,
    variables: &[Variable],
    selected: &str,
    path: &str,
    query_type: &str,
    lang: &str,
    names: &mut BTreeSet<String>,
) -> Mapping {
    let operation = operation_name(field);
    let definitions: Vec<String> = variables
        .iter()
        .map(|v| {
            format!(
                "${}: {}{}",
                v.input,
                v.type_name,
                if v.required { "!" } else { "" }
            )
        })
        .collect();
    let arguments: Vec<String> = variables
        .iter()
        .map(|v| format!("{}: ${}", v.argument, v.input))
        .collect();
    let document = format!(
        "query {operation}{} {{ {field}{}{} }}",
        if definitions.is_empty() {
            String::new()
        } else {
            format!("({})", definitions.join(", "))
        },
        if arguments.is_empty() {
            String::new()
        } else {
            format!("({})", arguments.join(", "))
        },
        if selected.is_empty() {
            String::new()
        } else {
            format!(" {selected}")
        },
    );
    let name = unique(names, &slug(field));
    let mut entry = Mapping::new();
    entry.insert("name".into(), name.into());
    entry.insert(
        "desc".into(),
        describe(
            &Label::Graphql {
                name: field,
                mutation: false,
            },
            lang,
        )
        .into(),
    );
    entry.insert("method".into(), "POST".into());
    entry.insert("path".into(), path.into());
    entry.insert("effect".into(), "read".into());
    entry.insert(
        "evidence".into(),
        format!("the GraphQL schema at {path} lists {field} under {query_type}").into(),
    );
    let mut json = Mapping::new();
    let mut fixed = |key: &str, value: &str| {
        let mut input = Mapping::new();
        input.insert("type".into(), "string".into());
        input.insert("default".into(), value.into());
        json.insert(key.into(), Yaml::Mapping(input));
    };
    fixed("query", &document);
    fixed("operationName", &operation);
    let mut template = serde_json::Map::new();
    template.insert("query".into(), "{{query}}".into());
    template.insert("operationName".into(), "{{operationName}}".into());
    let mut rendered = serde_json::Map::new();
    for variable in variables {
        let mut input = Mapping::new();
        input.insert("type".into(), variable.kind.name().into());
        if variable.required {
            input.insert("required".into(), true.into());
        }
        json.insert(variable.input.clone().into(), Yaml::Mapping(input));
        rendered.insert(
            variable.input.clone(),
            format!("{{{{{}}}}}", variable.input).into(),
        );
    }
    if !rendered.is_empty() {
        template.insert("variables".into(), Value::Object(rendered));
    }
    let mut body = Mapping::new();
    body.insert("json".into(), Yaml::Mapping(json));
    body.insert(
        "json_template".into(),
        serde_norway::to_value(Value::Object(template)).unwrap_or(Yaml::Null),
    );
    entry.insert("body".into(), Yaml::Mapping(body));
    let mut response = Mapping::new();
    response.insert("format".into(), "json".into());
    response.insert(
        "required_pointers".into(),
        Yaml::Sequence(vec![format!("/data/{field}").into()]),
    );
    entry.insert("response".into(), Yaml::Mapping(response));
    reorder(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_names_its_endpoint_but_not_its_scripts() {
        let page = Page {
            url: "https://app.example/board".into(),
            html: "<script src=\"/static/graphql.min.js\"></script>
                   <script>fetch('/api/graphql', {method: 'POST'}); const uri = \"https://app.example/graphql/\";
                   const other = 'https://other.example/graphql'; const cdn = '//cdn.example/graphql';</script>"
                .into(),
        };
        assert_eq!(
            endpoints_in_page(&page),
            [
                "https://app.example/api/graphql",
                "https://app.example/graphql"
            ]
        );
        assert_eq!(operation_name("countries"), "Countries");
        assert!(String::from_utf8_lossy(&introspection_body()).contains("__schema"));
    }
}
