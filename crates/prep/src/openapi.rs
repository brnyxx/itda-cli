//! OpenAPI 3.x and Swagger 2.0 to a draft definition.
use crate::{Draft, Options, insert_after, reorder, slug};
use itda_types::error::{AppResult, usage};
use serde_json::Value;
use serde_norway::{Mapping, Value as Yaml};
use std::collections::BTreeSet;

const METHODS: &[&str] = &["get", "head", "post", "put", "patch", "delete"];
/// A definition is for an agent to read; huge APIs should be narrowed first.
const MAX_OPERATIONS: usize = 200;

pub fn from_openapi(text: &[u8], options: &Options) -> AppResult<Draft> {
    let doc: Value = serde_norway::from_slice(text).map_err(|_| not_openapi())?;
    let swagger2 = doc["swagger"].as_str() == Some("2.0");
    if !swagger2 && !doc["openapi"].as_str().is_some_and(|v| v.starts_with('3')) {
        return Err(not_openapi());
    }
    let title = doc["info"]["title"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_owned();
    let version = doc["info"]["version"].as_str().unwrap_or("").to_owned();
    let site = options
        .name
        .as_deref()
        .map(slug)
        .filter(|s| !s.is_empty())
        .or_else(|| Some(slug(&title)).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "site".into());
    let base_url = match &options.base_url {
        Some(url) => url.clone(),
        None => server_url(&doc, swagger2).ok_or_else(|| {
            usage(
                "The document names no absolute server URL",
                "Pass --base-url with the API's address.",
            )
        })?,
    };
    let base_url = base_url.trim_end_matches('/').to_owned();

    let mut notes = Vec::new();
    let (auth, schemes) = auth_entries(&doc, swagger2, &site, &mut notes);
    let global_security = first_security(&doc["security"], &schemes);

    let mut operations = Vec::new();
    let mut names = BTreeSet::new();
    let empty = serde_json::Map::new();
    let paths = doc["paths"].as_object().unwrap_or(&empty);
    'paths: for (path, item) in paths {
        for method in METHODS {
            let Some(op) = item.get(*method) else {
                continue;
            };
            let tags: Vec<&str> = op["tags"]
                .as_array()
                .map(|t| t.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let id = op["operationId"].as_str().unwrap_or("");
            if let Some(filter) = &options.include
                && ![path.as_str(), id]
                    .iter()
                    .chain(tags.iter())
                    .any(|t| t.to_lowercase().contains(&filter.to_lowercase()))
            {
                continue;
            }
            if operations.len() == MAX_OPERATIONS {
                notes.push(format!(
                    "Stopped at {MAX_OPERATIONS} operations; narrow the draft with --include."
                ));
                break 'paths;
            }
            let label = format!("{} {path}", method.to_uppercase());
            if op["deprecated"].as_bool() == Some(true) {
                notes.push(format!("{label}: skipped because it is deprecated."));
                continue;
            }
            let mut name = slug(if id.is_empty() { &label } else { id });
            if name.is_empty() {
                name = slug(&label);
            }
            let mut unique = name.clone();
            let mut n = 2;
            while !names.insert(unique.clone()) {
                unique = format!("{name}-{n}");
                n += 1;
            }
            match operation(&doc, swagger2, path, method, item, op, &label, &mut notes) {
                Some(mut entry) => {
                    let security = if op.get("security").is_some() {
                        first_security(&op["security"], &schemes)
                    } else {
                        global_security.clone()
                    };
                    if let Some(auth_name) = security {
                        insert_after(&mut entry, "method", "auth", auth_name.into());
                    }
                    entry.insert("name".into(), unique.clone().into());
                    entry.insert(
                        "evidence".into(),
                        format!("OpenAPI {title} {version}: {label}").trim().into(),
                    );
                    operations.push((unique, reorder(entry)));
                }
                None => {
                    names.remove(&unique);
                }
            }
        }
    }
    if operations.is_empty() {
        return Err(usage(
            "No operations could be drafted",
            "Check the document's paths, or loosen --include.",
        ));
    }

    let source = if title.is_empty() {
        "an OpenAPI document".to_owned()
    } else {
        title.clone()
    };
    Ok(crate::finish(
        &site,
        &title,
        &base_url,
        auth,
        operations.into_iter().map(|(_, op)| op).collect(),
        notes,
        &format!("`itda draft openapi` from {source}"),
    ))
}

fn not_openapi() -> itda_types::error::AppError {
    usage(
        "This is not an OpenAPI 3 or Swagger 2.0 document",
        "Pass the API's openapi.json or openapi.yaml.",
    )
}

fn server_url(doc: &Value, swagger2: bool) -> Option<String> {
    let url = if swagger2 {
        let host = doc["host"].as_str()?;
        let scheme = doc["schemes"]
            .as_array()
            .and_then(|s| {
                s.iter()
                    .filter_map(Value::as_str)
                    .find(|s| *s == "https")
                    .or_else(|| s.first().and_then(Value::as_str))
            })
            .unwrap_or("https");
        format!(
            "{scheme}://{host}{}",
            doc["basePath"].as_str().unwrap_or("")
        )
    } else {
        let server = doc["servers"].as_array()?.first()?;
        let mut url = server["url"].as_str()?.to_owned();
        if let Some(variables) = server["variables"].as_object() {
            for (name, variable) in variables {
                if let Some(default) = variable["default"].as_str() {
                    url = url.replace(&format!("{{{name}}}"), default);
                }
            }
        }
        url
    };
    (url.starts_with("https://") || url.starts_with("http://")).then_some(url)
}

/// Header API keys and bearer tokens become `header` auth entries; other
/// schemes need a browser or a flow itda does not run yet.
fn auth_entries(
    doc: &Value,
    swagger2: bool,
    site: &str,
    notes: &mut Vec<String>,
) -> (Vec<Yaml>, Vec<(String, String)>) {
    let schemes = if swagger2 {
        &doc["securityDefinitions"]
    } else {
        &doc["components"]["securitySchemes"]
    };
    let (mut entries, mut known) = (Vec::new(), Vec::new());
    let Some(schemes) = schemes.as_object() else {
        return (entries, known);
    };
    for (scheme, definition) in schemes {
        let name = slug(scheme);
        let (header, prefix) = match (
            definition["type"].as_str(),
            definition["in"].as_str(),
            definition["scheme"].as_str().map(str::to_ascii_lowercase),
        ) {
            (Some("apiKey"), Some("header"), _) => {
                (definition["name"].as_str().unwrap_or("").to_owned(), "")
            }
            (Some("http"), _, Some(s)) if s == "bearer" => ("Authorization".into(), "Bearer "),
            (kind, _, _) => {
                notes.push(format!(
                    "Security scheme {scheme} ({}) is not supported yet; operations that need only it get no auth.",
                    kind.unwrap_or("unknown")
                ));
                continue;
            }
        };
        if header.is_empty() || name.is_empty() {
            continue;
        }
        let mut entry = Mapping::new();
        entry.insert("name".into(), name.clone().into());
        entry.insert("kind".into(), "header".into());
        entry.insert("header".into(), header.into());
        if !prefix.is_empty() {
            entry.insert("prefix".into(), prefix.into());
        }
        entry.insert("value_from".into(), format!("store:{site}/{name}").into());
        entries.push(Yaml::Mapping(entry));
        known.push((scheme.clone(), name));
    }
    (entries, known)
}

/// The first requirement this draft can satisfy. An empty requirement means
/// the operation also works signed out.
fn first_security(requirements: &Value, schemes: &[(String, String)]) -> Option<String> {
    let requirements = requirements.as_array()?;
    if requirements
        .iter()
        .any(|r| r.as_object().is_some_and(|m| m.is_empty()))
    {
        return None;
    }
    requirements.iter().find_map(|requirement| {
        let names: Vec<&String> = requirement.as_object()?.keys().collect();
        let [only] = names.as_slice() else {
            return None;
        };
        schemes
            .iter()
            .find(|(scheme, _)| scheme == *only)
            .map(|(_, name)| name.clone())
    })
}

#[allow(clippy::too_many_arguments)]
fn operation(
    doc: &Value,
    swagger2: bool,
    path: &str,
    method: &str,
    item: &Value,
    op: &Value,
    label: &str,
    notes: &mut Vec<String>,
) -> Option<Mapping> {
    let mut entry = Mapping::new();
    let summary = op["summary"]
        .as_str()
        .or_else(|| op["description"].as_str())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim();
    if !summary.is_empty() {
        entry.insert(
            "desc".into(),
            summary.chars().take(160).collect::<String>().into(),
        );
    }
    entry.insert("method".into(), method.to_uppercase().into());
    entry.insert("path".into(), path.into());
    let read = matches!(method, "get" | "head");
    entry.insert("effect".into(), if read { "read" } else { "write" }.into());

    let (mut params, mut query, mut form) = (Mapping::new(), Mapping::new(), Mapping::new());
    let mut body_schema = None;
    let declared = item["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(op["parameters"].as_array().into_iter().flatten());
    for parameter in declared {
        let parameter = resolve(doc, parameter);
        let Some(name) = parameter["name"].as_str() else {
            continue;
        };
        let schema = if swagger2 {
            parameter.clone()
        } else {
            resolve(doc, &parameter["schema"]).clone()
        };
        let required = parameter["required"].as_bool() == Some(true);
        match parameter["in"].as_str() {
            Some("path") => {
                params.insert(name.into(), param(&schema, true));
            }
            Some("query") => {
                query.insert(name.into(), param(&schema, required));
            }
            Some("body") => body_schema = Some(resolve(doc, &parameter["schema"]).clone()),
            Some("formData") => {
                form.insert(name.into(), param(&schema, required));
            }
            Some(other) => notes.push(format!(
                "{label}: {other} parameter {name} left out; add it under headers if the site needs it."
            )),
            None => {}
        }
    }
    let request_body = resolve(doc, &op["requestBody"]);
    if !swagger2 && let Some(content) = request_body["content"].as_object() {
        if let Some(json) = content.iter().find(|(k, _)| k.contains("json")) {
            body_schema = Some(resolve(doc, &json.1["schema"]).clone());
        } else if let Some(urlencoded) = content.get("application/x-www-form-urlencoded") {
            let schema = resolve(doc, &urlencoded["schema"]);
            form = object_fields(doc, schema);
        } else {
            notes.push(format!(
                "{label}: skipped; its request body is {} and only JSON and forms are supported.",
                content.keys().next().map(String::as_str).unwrap_or("empty")
            ));
            return None;
        }
    }
    if !params.is_empty() {
        entry.insert("params".into(), Yaml::Mapping(params));
    }
    if !query.is_empty() {
        entry.insert("query".into(), Yaml::Mapping(query));
    }
    let mut body = Mapping::new();
    if let Some(schema) = body_schema {
        let fields = object_fields(doc, &schema);
        if fields.is_empty() {
            notes.push(format!(
                "{label}: skipped; its JSON body is not an object with named fields."
            ));
            return None;
        }
        body.insert("json".into(), Yaml::Mapping(fields));
    } else if !form.is_empty() {
        body.insert("form".into(), Yaml::Mapping(form));
    }
    if !body.is_empty() {
        entry.insert("body".into(), Yaml::Mapping(body));
    }
    if let Some(response) = response(doc, swagger2, op) {
        entry.insert("response".into(), response);
    } else if read {
        notes.push(format!(
            "{label}: no JSON response schema; add a response check before relying on it."
        ));
    }
    Some(entry)
}

fn param(schema: &Value, required: bool) -> Yaml {
    let kind = match schema["type"].as_str() {
        Some("integer") => "int",
        Some("number") => "float",
        Some("boolean") => "bool",
        _ => "string",
    };
    let mut map = Mapping::new();
    map.insert("type".into(), kind.into());
    if required {
        map.insert("required".into(), true.into());
    }
    if let Some(default) = schema.get("default")
        && !default.is_object()
        && !default.is_array()
        && let Ok(value) = serde_norway::to_value(default)
    {
        map.insert("default".into(), value);
    }
    Yaml::Mapping(map)
}

/// Top-level fields of an object schema; nested objects become strings an
/// agent fills with JSON.
fn object_fields(doc: &Value, schema: &Value) -> Mapping {
    let schema = resolve(doc, schema);
    let required: BTreeSet<&str> = schema["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut fields = Mapping::new();
    if let Some(properties) = schema["properties"].as_object() {
        for (name, property) in properties {
            let property = resolve(doc, property);
            if property["readOnly"].as_bool() == Some(true) {
                continue;
            }
            fields.insert(
                name.clone().into(),
                param(property, required.contains(name.as_str())),
            );
        }
    }
    fields
}

/// The declared success response: required top-level fields of a JSON object
/// become pointers the answer must contain.
fn response(doc: &Value, swagger2: bool, op: &Value) -> Option<Yaml> {
    let responses = op["responses"].as_object()?;
    let (status, success) = responses.iter().find(|(code, _)| code.starts_with('2'))?;
    let success = resolve(doc, success);
    let mut map = Mapping::new();
    let schema = if swagger2 {
        success.get("schema").map(|s| resolve(doc, s))
    } else {
        match success["content"].as_object() {
            Some(content) => {
                let (_, json) = content.iter().find(|(k, _)| k.contains("json"))?;
                Some(resolve(doc, &json["schema"]))
            }
            None => None,
        }
    };
    let Some(schema) = schema else {
        if status == "204" {
            map.insert("format".into(), "empty".into());
            return Some(Yaml::Mapping(map));
        }
        return None;
    };
    map.insert("format".into(), "json".into());
    let pointers: Vec<Yaml> = schema["required"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(Value::as_str)
                .take(3)
                .map(|f| format!("/{}", f.replace('~', "~0").replace('/', "~1")).into())
                .collect()
        })
        .unwrap_or_default();
    if !pointers.is_empty() {
        map.insert("required_pointers".into(), Yaml::Sequence(pointers));
    }
    Some(Yaml::Mapping(map))
}

/// Follow local `$ref`s, at most a few levels deep.
fn resolve<'a>(doc: &'a Value, value: &'a Value) -> &'a Value {
    let mut current = value;
    for _ in 0..8 {
        let Some(reference) = current["$ref"].as_str() else {
            return current;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return current;
        };
        match doc.pointer(pointer) {
            Some(target) => current = target,
            None => return current,
        }
    }
    current
}
