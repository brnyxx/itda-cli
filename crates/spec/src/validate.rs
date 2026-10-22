//! Structural checks for a parsed SiteSpec. Safety policy lives in `itda-policy`.
use crate::SPEC_VERSION;
use crate::model::{Issue, Param, Spec};
use crate::request::path_vars;

impl Spec {
    pub fn validate(&self) -> Vec<Issue> {
        let mut out: Vec<Issue> = Vec::new();
        {
            fn add(out: &mut Vec<Issue>, field: &str, detail: String, remedy: &str) {
                out.push(Issue {
                    field: field.into(),
                    detail,
                    remedy: remedy.into(),
                });
            }

            if self.spec_version != SPEC_VERSION && self.spec_version != 2 {
                add(
                    &mut out,
                    "spec_version",
                    format!("supported: 1|2, got: {}", self.spec_version),
                    "Set spec_version to 1 or 2",
                );
            }
            if self.site.name.is_empty() {
                add(
                    &mut out,
                    "site.name",
                    "empty".into(),
                    "Site identifier is required",
                );
            }
            if self.site.base_url.is_empty() {
                add(
                    &mut out,
                    "site.base_url",
                    "empty".into(),
                    "Provide an https:// absolute URL",
                );
            } else if !self.site.base_url.starts_with("http") {
                add(
                    &mut out,
                    "site.base_url",
                    "missing http(s) scheme".into(),
                    "",
                );
            }

            let mut auths = std::collections::HashSet::new();
            for a in &self.auth {
                if !auths.insert(a.name.clone()) {
                    add(
                        &mut out,
                        &format!("auth.{}", a.name),
                        "duplicate auth strategy name".into(),
                        "",
                    );
                }
                match a.kind.as_str() {
                    "none" => {}
                    "browser" => {
                        let named = ["cookie:", "query:"].iter().any(|prefix| {
                            a.value_from
                                .strip_prefix(prefix)
                                .is_some_and(|name| !name.trim().is_empty())
                        });
                        if !a.value_from.is_empty() && !named {
                            add(
                                &mut out,
                                &format!("auth.{}.value_from", a.name),
                                "must be empty, name the sign-in cookie as cookie:<name>, or name the query parameter the session goes in as query:<key>".into(),
                                "Example: cookie:session_id, or query:sid",
                            );
                        }
                    }
                    "header" => {
                        if a.header.is_empty() {
                            add(
                                &mut out,
                                &format!("auth.{}.header", a.name),
                                "missing header name for kind=header".into(),
                                "",
                            );
                        }
                        if !a.value_from.starts_with("store:") {
                            add(
                                &mut out,
                                &format!("auth.{}.value_from", a.name),
                                "must reference store:<site>/<key>".into(),
                                "Never embed raw secrets in a SiteSpec",
                            );
                        }
                    }
                    other => add(
                        &mut out,
                        &format!("auth.{}.kind", a.name),
                        format!("unsupported auth kind: {other}"),
                        "none|header|browser",
                    ),
                }
            }
            let models: std::collections::HashSet<&String> = self.models.keys().collect();
            let op_names: std::collections::HashSet<&String> =
                self.operations.iter().map(|o| &o.name).collect();

            for op in &self.operations {
                let id = format!("operations.{}", op.name);
                if op.method.is_empty() {
                    add(
                        &mut out,
                        &format!("{id}.method"),
                        "empty".into(),
                        "GET|POST|PUT|PATCH|DELETE",
                    );
                }
                if !op.path.starts_with('/') {
                    add(
                        &mut out,
                        &format!("{id}.path"),
                        format!("must start with /: {}", op.path),
                        "",
                    );
                }
                if !["", "http"].contains(&op.transport.as_str()) {
                    add(
                        &mut out,
                        &format!("{id}.transport"),
                        format!("unsupported transport: {}", op.transport),
                        "itda sends plain HTTP requests only; leave transport out",
                    );
                }
                if !op.auth.is_empty() && !auths.contains(&op.auth) {
                    add(
                        &mut out,
                        &format!("{id}.auth"),
                        format!("undefined auth strategy: {}", op.auth),
                        "Define in auth block",
                    );
                }
                if !op.model.is_empty() && !models.contains(&op.model) {
                    add(
                        &mut out,
                        &format!("{id}.model"),
                        format!("undefined model: {}", op.model),
                        "Define in models block",
                    );
                }
                for step in &op.preflight {
                    let valid_target = (!step.op.is_empty() && op_names.contains(&step.op))
                        || (!step.path.is_empty() && step.path.starts_with('/'));
                    if step.name.is_empty() || !valid_target {
                        add(
                            &mut out,
                            &format!("{id}.preflight"),
                            format!("invalid preflight step: {}", step.name),
                            "Reference a defined operation or a path starting with '/'",
                        );
                    }
                    if !matches!(step.method.to_ascii_uppercase().as_str(), "" | "GET") {
                        add(
                            &mut out,
                            &format!("{id}.preflight"),
                            format!("preflight step {} is not a GET", step.name),
                            "Preflight steps only read",
                        );
                    }
                    for (name, source) in &step.extract {
                        let known = ["json:/", "css:", "header:", "cookie:"]
                            .iter()
                            .any(|prefix| {
                                source.starts_with(prefix) && source.len() > prefix.len()
                            })
                            || ssr_source_valid(source);
                        if !known {
                            add(
                                &mut out,
                                &format!("{id}.preflight"),
                                format!("preflight step {} cannot extract {name}", step.name),
                                "Use json:/pointer, css:selector[@attribute], header:name, cookie:name or ssr:key#/pointer",
                            );
                        }
                    }
                }
                if let Some(pagination) = &op.pagination {
                    check_pagination(op, pagination, &id, &mut out);
                }
                if let Some(confirm) = &op.confirm {
                    check_confirm(self, op, confirm, &id, &mut out);
                }
                let path_names = path_vars(&op.path);
                for n in &path_names {
                    if !op.params.contains_key(n) {
                        add(
                            &mut out,
                            &format!("{id}.params"),
                            format!("undeclared path parameter {{{n}}}"),
                            "Declare in params",
                        );
                    }
                }
                for n in op.params.keys() {
                    if !path_names.contains(n) {
                        add(
                            &mut out,
                            &format!("{id}.params.{n}"),
                            "parameter not present in path template".into(),
                            &format!("Use {{{n}}} in path or move to query"),
                        );
                    }
                }
                let mut counts: std::collections::BTreeMap<&String, usize> = Default::default();
                for k in op.params.keys() {
                    *counts.entry(k).or_default() += 1;
                }
                for k in op.query.keys() {
                    *counts.entry(k).or_default() += 1;
                }
                if let Some(b) = &op.body {
                    if (!b.json.is_empty() || b.json_template.is_some()) && !b.form.is_empty() {
                        add(
                            &mut out,
                            &format!("{id}.body"),
                            "Choose one body encoding".into(),
                            "Use body.json/json_template or body.form, not both.",
                        );
                    }
                    for (k, _) in b.inputs() {
                        *counts.entry(k).or_default() += 1;
                    }
                }
                for (n, c) in &counts {
                    if *c > 1 {
                        add(
                            &mut out,
                            id.as_str(),
                            format!("input name conflict: {n}"),
                            "Keep input names unique across params/query/body",
                        );
                    }
                }
                let check = |where_: &str,
                             ps: &std::collections::BTreeMap<String, Param>,
                             out: &mut Vec<Issue>| {
                    for (n, p) in ps {
                        match p.kind.as_str() {
                            "string" | "int" | "float" | "bool" | "json" => {}
                            other => out.push(Issue {
                                field: format!("{where_}.{n}.type"),
                                detail: format!("unsupported type: {other}"),
                                remedy: "string|int|float|bool|json".into(),
                            }),
                        }
                        if p.required && p.default.is_some() {
                            out.push(Issue {
                                field: format!("{where_}.{n}"),
                                detail: "required parameter cannot also define a default".into(),
                                remedy: "Use required or default, not both".into(),
                            });
                        }
                    }
                };
                check(&format!("{id}.params"), &op.params, &mut out);
                check(&format!("{id}.query"), &op.query, &mut out);
                if let Some(b) = &op.body {
                    check(&format!("{id}.body.json"), &b.json, &mut out);
                    check(&format!("{id}.body.form"), &b.form, &mut out);
                }
            }
        }
        out
    }
}

/// `ssr:<key>#/pointer`: a value in the JSON a page embeds for its scripts.
/// The key may be `auto`; the pointer must start with `/`.
fn ssr_source_valid(source: &str) -> bool {
    source
        .strip_prefix("ssr:")
        .and_then(|rest| rest.split_once('#'))
        .is_some_and(|(key, pointer)| !key.is_empty() && pointer.starts_with('/'))
}

fn check_pagination(
    op: &crate::model::Operation,
    pagination: &crate::model::Pagination,
    id: &str,
    out: &mut Vec<Issue>,
) {
    let field = format!("{id}.pagination");
    let mut add = |detail: String, remedy: &str| {
        out.push(Issue {
            field: field.clone(),
            detail,
            remedy: remedy.into(),
        });
    };
    if !["page", "offset", "cursor"].contains(&pagination.style.as_str()) {
        add(
            format!("unsupported style: {}", pagination.style),
            "page|offset|cursor",
        );
    }
    let pointer = |text: &str| text.starts_with('/') && text.len() <= 512;
    if !pagination.next.is_empty() && !pointer(&pagination.next) && pagination.next != "header:link"
    {
        add(
            "next must be a JSON pointer starting with / or header:link".into(),
            "Example: next: /meta/next_cursor",
        );
    }
    if pagination.style == "cursor" && pagination.next.is_empty() {
        add(
            "a cursor read must say where the next cursor is".into(),
            "Set next to a JSON pointer or header:link",
        );
    }
    if !pagination.more.is_empty() && !pointer(&pagination.more) {
        add(
            "more must be a JSON pointer starting with /".into(),
            "Example: more: /pageInfo/hasNextPage",
        );
    }
    let (query, body) = (&pagination.send.query, &pagination.send.body);
    if query.is_empty() == body.is_empty() {
        add(
            "send must name one input: {query: name} or {body: name}".into(),
            "Example: send: {query: cursor}",
        );
        return;
    }
    if pagination.next == "header:link" && !body.is_empty() {
        add(
            "header:link carries a query value, so send must be a query input".into(),
            "Use send: {query: name}",
        );
    }
    let declared = if body.is_empty() {
        op.query.get(query)
    } else {
        op.body.as_ref().and_then(|b| b.json.get(body))
    };
    let Some(param) = declared else {
        add(
            format!(
                "send names {}, which is not declared under {}",
                if body.is_empty() { query } else { body },
                if body.is_empty() {
                    "query"
                } else {
                    "body.json"
                }
            ),
            "Declare the input the marker goes back in",
        );
        return;
    };
    let expected = if pagination.style == "cursor" {
        "string"
    } else {
        "int"
    };
    if param.kind != expected {
        add(
            format!(
                "the {} input must be {expected} for style {}",
                if body.is_empty() { query } else { body },
                pagination.style
            ),
            "Match the input's type to the style",
        );
    }
    if pagination.style == "cursor" && (param.required || param.default.is_some()) {
        add(
            "a cursor input is filled from the previous answer: not required, no default".into(),
            "Declare it as {type: string}",
        );
    }
}

/// A `confirm` block names a read of the same definition that runs on its
/// defaults alone, a pointer into its answer and one input of the change to
/// compare with. Whether the read really reads is for the policy layer.
fn check_confirm(
    spec: &Spec,
    op: &crate::model::Operation,
    confirm: &crate::model::Confirm,
    id: &str,
    out: &mut Vec<Issue>,
) {
    let field = format!("{id}.confirm");
    let mut add = |detail: String, remedy: &str| {
        out.push(Issue {
            field: field.clone(),
            detail,
            remedy: remedy.into(),
        });
    };
    if op.effect == crate::Effect::Read {
        add(
            "only a change names the read that confirms it".into(),
            "Remove confirm from the read",
        );
    }
    match spec.operations.iter().find(|o| o.name == confirm.read) {
        None => add(
            format!("read names {}, which is not defined", confirm.read),
            "Name a read in the same definition",
        ),
        Some(read) if read.name == op.name => add(
            "read must name another operation".into(),
            "Name the read that lists the record",
        ),
        Some(read) => {
            if read.effect != crate::Effect::Read {
                add(
                    format!("{} is not a read", confirm.read),
                    "Name an operation with effect: read",
                );
            }
            let needs: Vec<&str> = read
                .params
                .keys()
                .chain(
                    read.query
                        .iter()
                        .chain(read.body.iter().flat_map(|b| b.inputs()))
                        .filter(|(_, p)| p.required && p.default.is_none())
                        .map(|(n, _)| n),
                )
                .map(String::as_str)
                .collect();
            if !needs.is_empty() {
                add(
                    format!(
                        "{} needs {} and a confirming read runs with its defaults alone",
                        confirm.read,
                        needs.join(", ")
                    ),
                    "Give the inputs a default or name another read",
                );
            }
        }
    }
    let pointer = &confirm.matching.pointer;
    if !pointer.starts_with('/') || pointer.len() > 512 {
        add(
            "match.pointer must be a JSON pointer starting with /".into(),
            "Example: pointer: /items/*/title",
        );
    }
    match confirm.matching.input() {
        None => add(
            "match.from must name an input as params.<name>, query.<name> or body.<name>".into(),
            "Example: from: body.title",
        ),
        Some((place, name)) => {
            let declared = match place {
                "params" => op.params.get(name),
                "query" => op.query.get(name),
                _ => op
                    .body
                    .as_ref()
                    .and_then(|body| body.inputs().find(|(n, _)| *n == name).map(|(_, p)| p)),
            };
            match declared {
                None => add(
                    format!("match.from names {name}, which is not declared under {place}"),
                    "Name one of the change's inputs",
                ),
                Some(param) if param.from_step() => add(
                    format!("match.from names {name}, which a preflight step fills"),
                    "Compare an input the agent gives",
                ),
                Some(param) if !param.required && param.default.is_none() => add(
                    format!("match.from names {name}, which the change may be sent without"),
                    "Make the input required or give it a default",
                ),
                Some(param) if !matches!(param.kind.as_str(), "string" | "int" | "float") => add(
                    format!(
                        "match.from names {name}, whose type {} cannot be compared",
                        param.kind
                    ),
                    "Compare a string or number input",
                ),
                Some(_) => {}
            }
        }
    }
}
