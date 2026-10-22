#![allow(clippy::unwrap_used)]
use itda_prep::{Described, Description, Options, Page, from_pages, from_pages_and_description};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/").to_owned() + name,
    )
    .unwrap()
}

fn spec_of(draft: &itda_prep::Draft) -> itda_spec::Spec {
    itda_spec::parse(draft.yaml.as_bytes()).unwrap()
}

fn page(url: &str, file: &str) -> Page {
    Page {
        url: url.into(),
        html: fixture(file),
    }
}

#[test]
fn a_board_page_becomes_a_list_a_search_a_post_and_a_feed() {
    let draft = from_pages(
        &[page(
            "https://intranet.example.co.kr/board?page=1",
            "board.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.site, "intranet-example");
    assert_eq!(draft.operations, 5, "{}", draft.yaml);
    assert_eq!(draft.reads, 4);
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://intranet.example.co.kr");

    let board = spec.op("get-board").unwrap();
    let response = board.response.as_ref().unwrap();
    assert_eq!(response.format, "html");
    assert_eq!(response.required_html_fields[0].selector, "h1");
    let html = response.html.as_ref().unwrap();
    assert_eq!(html.items, "table tbody tr");
    assert_eq!(html.fields["title"].selector, "td:nth-child(2)");
    assert!(html.fields["link"].absolute_url);
    assert_eq!(board.query["page"].kind, "int");
    assert_eq!(board.query["page"].default, Some(1.into()));
    let pagination = board.pagination.as_ref().unwrap();
    assert_eq!(
        (pagination.style.as_str(), pagination.next.as_str()),
        ("page", "")
    );
    assert_eq!(pagination.input(), ("page", false));
    assert_eq!(html.fields["author"].selector, "td:nth-child(3)");

    let one = spec.op("get-board-by-id").unwrap();
    assert_eq!(one.path, "/board/{id}");
    assert_eq!(one.desc, "Read one of the items on Notice board");
    assert_eq!(one.params["id"].kind, "int");
    assert!(one.params["id"].required);
    assert!(one.response.is_none());
    assert_eq!(
        one.evidence,
        "the records of GET /board link to /board/{id}"
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.starts_with("GET /board/{id}: reads one record at the list's links")),
        "{:?}",
        draft.notes
    );

    let search = spec.op("search-board").unwrap();
    assert_eq!(search.method, "GET");
    assert_eq!(search.path, "/board/search");
    assert!(search.query["q"].required);
    assert_eq!(search.query["board_id"].default, Some("7".into()));
    assert_eq!(search.query["sort"].kind, "string");

    let write = spec.op("post-write").unwrap();
    assert_eq!(write.auth, "session");
    let body = write.body.as_ref().unwrap();
    assert!(body.form["title"].required);
    assert!(body.form["_csrf"].from_step());
    // The board lists what the form posts, by title: the change names it.
    let confirm = write.confirm.as_ref().unwrap();
    assert_eq!(confirm.read, "get-board");
    assert_eq!(confirm.matching.pointer, "/items/*/title");
    assert_eq!(confirm.matching.from, "body.title");
    assert!(
        write.evidence.contains("GET /board lists it under title"),
        "{}",
        write.evidence
    );
    assert_eq!(write.preflight[0].path, "/board?page=1");
    assert_eq!(
        write.preflight[0].extract["csrf"],
        "css:input[name=\"_csrf\"]@value"
    );

    let feed = spec.op("feed").unwrap();
    assert_eq!(feed.path, "/board/feed.xml");
    assert_eq!(feed.response.as_ref().unwrap().format, "xml");

    assert_eq!(spec.auth[0].kind, "browser");
    assert!(!draft.yaml.contains("recorded-secret-token"));
    assert!(!draft.yaml.contains("name: pw"));
    assert!(draft.notes.iter().any(|n| n.contains("sign-in form")));
}

#[test]
fn an_app_shell_reads_its_embedded_data_and_points_at_recording() {
    let draft = from_pages(
        &[page("https://shop.example/products", "app.html")],
        &Options {
            name: Some("shop".into()),
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(draft.operations, 1);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let products = spec.op("list-products").unwrap();
    let response = products.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "__NEXT_DATA__");
    assert_eq!(response.extract_pointer, "/props/pageProps/products");
    assert_eq!(response.required_pointers, ["/0/name", "/0/id"]);
    assert!(!draft.yaml.contains("private-value"));
    assert!(draft.notes.iter().any(|n| n.contains("__NEXT_DATA__")));
}

#[test]
fn pages_elsewhere_and_duplicates_are_left_out() {
    let draft = from_pages(
        &[
            page("https://intranet.example.co.kr/board", "board.html"),
            page("https://intranet.example.co.kr/board", "board.html"),
            page("https://other.example/board", "board.html"),
        ],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 5);
    assert!(draft.notes.iter().any(|n| n.contains("not under")));
    assert!(draft.notes.iter().any(|n| n.contains("drafted once")));
    assert!(from_pages(&[], &Options::default()).is_err());
    assert!(from_pages(&[page("ftp://x/", "app.html")], &Options::default()).is_err());
}

fn described(server: &str, extra_path: &str) -> String {
    let servers = if server.is_empty() {
        String::new()
    } else {
        format!(r#""servers": [{{"url": "{server}"}}],"#)
    };
    format!(
        r#"{{"openapi": "3.0.3", "info": {{"title": "Pet API", "version": "2.0"}}, {servers}
        "components": {{"securitySchemes": {{"token": {{"type": "http", "scheme": "bearer"}}}}}},
        "paths": {{
          "/pets": {{
            "get": {{"operationId": "listPets", "summary": "List pets",
                     "responses": {{"200": {{"content": {{"application/json": {{"schema": {{"type": "object", "required": ["items"]}}}}}}}}}}}},
            "post": {{"operationId": "createPet", "security": [{{"token": []}}],
                      "requestBody": {{"content": {{"application/json": {{"schema": {{"type": "object", "properties": {{"name": {{"type": "string"}}}}}}}}}}}},
                      "responses": {{"201": {{"description": "made"}}}}}}
          }},
          "/pets/{{petId}}": {{
            "get": {{"operationId": "showPet", "security": [{{"token": []}}],
                     "parameters": [{{"name": "petId", "in": "path", "required": true, "schema": {{"type": "integer"}}}}],
                     "responses": {{"200": {{"content": {{"application/json": {{"schema": {{"type": "object", "required": ["id"]}}}}}}}}}}}},
            "delete": {{"operationId": "removePet", "responses": {{"204": {{"description": "gone"}}}}}}
          }},
          "{extra_path}": {{
            "get": {{"operationId": "extra", "responses": {{"200": {{"description": "ok"}}}}}}
          }},
          "/secure": {{
            "get": {{"operationId": "secure",
                     "parameters": [{{"name": "apikey", "in": "query", "schema": {{"type": "string"}}}}],
                     "responses": {{"200": {{"description": "ok"}}}}}}
          }}
        }}}}"#
    )
}

#[test]
fn a_linked_description_adds_its_reads_and_notes_its_changes() {
    let page = page("https://pets.example/docs", "docs.html");
    let description = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("/api/v2", "/docs").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&description),
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(
        draft.described,
        Some(Described {
            url: "https://pets.example/openapi.json".into(),
            reads: 3,
            changes: 2,
        })
    );
    assert_eq!((draft.operations, draft.reads), (5, 5), "{}", draft.yaml);
    assert_eq!(
        spec_of(&draft).op("get-pet").unwrap().path,
        "/pets/{id}",
        "the table's links share one shape: {}",
        draft.yaml
    );
    assert!(
        draft
            .yaml
            .starts_with("# Drafted by `itda draft page` from pages on pets.example and the API description at https://pets.example/openapi.json."),
        "{}",
        draft.yaml
    );
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.site.base_url, "https://pets.example");
    assert_eq!(spec.site.name, "pets");

    let list = spec.op("list-pets").unwrap();
    assert_eq!(
        list.path, "/api/v2/pets",
        "a relative server is a path prefix"
    );
    assert_eq!(list.desc, "List pets");
    assert_eq!(
        list.evidence,
        "OpenAPI Pet API 2.0 at https://pets.example/openapi.json: GET /pets"
    );
    assert_eq!(
        list.response.as_ref().unwrap().required_pointers,
        ["/items"]
    );
    let show = spec.op("show-pet").unwrap();
    assert_eq!(show.auth, "token");
    assert_eq!(show.params["petId"].kind, "int");
    assert_eq!(spec.op("extra").unwrap().path, "/api/v2/docs");
    assert_eq!(
        spec.auth[0].value_from, "store:pets/token",
        "auth belongs to the page's site, not the document's title"
    );

    assert!(spec.op("create-pet").is_err());
    assert!(spec.op("remove-pet").is_err());
    assert!(
        spec.op("secure").is_err(),
        "a read keyed by an input is not drafted: {}",
        draft.yaml
    );
    assert!(
        draft.notes.iter().any(|n| n
            == "GET /secure: left out; credential input apikey goes in auth, never through the agent."),
        "{:?}",
        draft.notes
    );
    assert!(
        draft.notes.iter().any(|n| n
            .contains("`itda draft openapi https://pets.example/openapi.json` drafts the changes")),
        "{:?}",
        draft.notes
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.starts_with("POST /pets: a change the API description lists")),
        "{:?}",
        draft.notes
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.starts_with("DELETE /pets/{petId}: a change")),
        "{:?}",
        draft.notes
    );
    assert!(
        !draft
            .notes
            .iter()
            .any(|n| n.contains("`itda draft openapi` makes a fuller")),
        "the old hint goes once the description is read: {:?}",
        draft.notes
    );
}

#[test]
fn a_description_without_servers_lives_where_it_was_found_and_dedupes_by_path() {
    let page = page("https://pets.example/docs", "docs.html");
    let description = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("", "/docs").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&description),
        &Options::default(),
    )
    .unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    assert_eq!(spec.op("list-pets").unwrap().path, "/pets");
    assert!(
        spec.op("extra").is_err(),
        "GET /docs is the page itself: {}",
        draft.yaml
    );
    assert_eq!(
        spec.op("list-docs")
            .unwrap()
            .response
            .as_ref()
            .unwrap()
            .format,
        "html"
    );
    assert_eq!(draft.described.as_ref().unwrap().reads, 2);
}

#[test]
fn a_description_elsewhere_or_unreadable_leaves_the_page_draft_alone() {
    let page = page("https://pets.example/docs", "docs.html");
    let plain = from_pages(std::slice::from_ref(&page), &Options::default()).unwrap();
    assert_eq!(plain.operations, 2, "{}", plain.yaml);
    assert!(plain.described.is_none());
    assert!(
        plain.notes.iter().any(|n| n.contains(
            "links to an API description at https://pets.example/openapi.json; `itda draft openapi`"
        )),
        "{:?}",
        plain.notes
    );

    let elsewhere = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: described("https://api.pets.example/v2", "/x").into_bytes(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&elsewhere),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 2);
    assert!(draft.described.is_none());
    assert!(
        draft.notes.iter().any(|n| n.contains(
            "describes an API at https://api.pets.example/v2, not under https://pets.example"
        )),
        "{:?}",
        draft.notes
    );

    let broken = Description {
        url: "https://pets.example/openapi.json".into(),
        bytes: b"<html>not here</html>".to_vec(),
    };
    let draft = from_pages_and_description(
        std::slice::from_ref(&page),
        Some(&broken),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 2);
    assert!(draft.described.is_none());
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("as an API description")),
        "{:?}",
        draft.notes
    );
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
}

#[test]
fn an_ld_json_item_list_becomes_a_typed_read_of_its_articles() {
    let draft = from_pages(
        &[page("https://news.example/top", "ld-itemlist.html")],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let op = spec.op("list-news-articles").unwrap();
    assert_eq!(op.desc, "List the news articles on Top stories");
    let response = op.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "ld+json");
    assert_eq!(response.extract_pointer, "/blocks/0/itemListElement");
    assert_eq!(
        response.required_pointers,
        ["/0/item/headline", "/0/item/url"]
    );
    assert_eq!(op.model, "news-article");
    let model = &spec.models["news-article"];
    assert_eq!(model["headline"].path, "item.headline");
    assert_eq!(model["datePublished"].path, "item.datePublished");
    assert_eq!(model["description"].kind, "string");
    assert_eq!(model["author_name"].path, "item.author.name");
    assert!(!model.contains_key("name"), "{:?}", model.keys());
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("`news-article` model")),
        "{:?}",
        draft.notes
    );
}

#[test]
fn a_next_f_list_gets_a_model_without_its_secrets() {
    let draft = from_pages(
        &[page("https://blog.example/", "next-f.html")],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let op = spec.op("list-items").unwrap();
    assert_eq!(op.desc, "블로그의 항목 목록 읽기");
    let response = op.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "__NEXT_F__");
    assert_eq!(response.extract_pointer, "/data");
    assert_eq!(response.required_pointers, ["/0/title", "/0/id"]);
    let model = &spec.models["item"];
    let fields: Vec<&String> = model.keys().collect();
    assert!(fields.contains(&&"released_at".to_string()), "{fields:?}");
    assert!(fields.contains(&&"likes".to_string()), "{fields:?}");
    assert_eq!(model["likes"].kind, "int");
    assert!(!draft.yaml.contains("api_token"), "{}", draft.yaml);
    assert!(!draft.yaml.contains("secret-1"), "{}", draft.yaml);
}

#[test]
fn an_apollo_cache_is_read_by_typename() {
    let draft = from_pages(
        &[page(
            "https://ask.example/questions?sort=new",
            "apollo.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let op = spec.op("list-questions").unwrap();
    let response = op.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "__APOLLO_STATE__");
    assert_eq!(response.extract_pointer, "/Question");
    assert_eq!(op.query["sort"].default, Some("new".into()));
    let model = &spec.models["question"];
    assert_eq!(model["answerCount"].kind, "int");
    assert!(model.contains_key("createdAt"));
    assert!(!model.contains_key("author"));
}

#[test]
fn a_preloaded_state_lists_what_has_a_title_not_the_navigation() {
    let draft = from_pages(
        &[page("https://shop.example/deals", "preloaded.html")],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let op = spec.op("list-deals").unwrap();
    let response = op.response.as_ref().unwrap();
    assert_eq!(response.ssr_json, "__PRELOADED_STATE__");
    assert_eq!(response.extract_pointer, "/deals/list");
    assert_eq!(response.required_pointers, ["/0/name", "/0/id", "/0/url"]);
    let model = &spec.models["deal"];
    assert_eq!(model["price"].kind, "float");
    assert!(model.contains_key("endsAt"));
    assert!(!draft.yaml.contains("csrf_token"), "{}", draft.yaml);
}

#[test]
fn fields_one_object_down_are_read_and_menus_are_not_lists() {
    let draft = from_pages(
        &[page(
            "https://code.example/rust-lang/rust/issues",
            "nested.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let op = spec.op("list-issues").unwrap();
    assert_eq!(op.desc, "List the issues on Issues");
    let response = op.response.as_ref().unwrap();
    assert_eq!(response.extract_pointer, "/props/pageProps/results");
    assert_eq!(
        response.required_pointers,
        ["/0/issue/title", "/0/id", "/0/issue/url"]
    );
    let model = &spec.models["issue"];
    assert_eq!(model["title"].path, "issue.title");
    assert_eq!(model["issue_id"].path, "issue.id");
    assert_eq!(model["url"].path, "issue.url");
    assert_eq!(model["number"].kind, "int");
    assert!(model.contains_key("createdAt"), "{:?}", model.keys());
    assert!(!model.contains_key("author"));
    assert!(!draft.yaml.contains("mainNavigation"), "{}", draft.yaml);
}

#[test]
fn a_card_grid_without_list_markup_lists_its_projects_and_reads_one() {
    let draft = from_pages(
        &[page("https://forge.example/projects", "cards.html")],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.operations, 2, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let list = spec.op("list-projects").unwrap();
    assert_eq!(list.desc, "List the projects on Projects");
    assert_eq!(list.query["page"].default, None);
    let html = list.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.items, "div.project-card");
    assert_eq!(html.fields["title"].selector, "h3");
    assert_eq!(html.fields["summary"].selector, "p");
    assert_eq!(html.fields["stars"].selector, "span.stars");
    assert_eq!(html.fields["published_at"].attribute, "datetime");
    assert_eq!(html.fields["link"].selector, "a[href]");
    let value = html
        .extract(&fixture("cards.html"), "https://forge.example/projects")
        .unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), 5);
    assert_eq!(value["items"][1]["stars"], "88");
    assert_eq!(
        value["items"][1]["link"],
        "https://forge.example/projects/beta"
    );
    assert!(!draft.yaml.contains("Terms"), "{}", draft.yaml);

    let one = spec.op("get-project").unwrap();
    assert_eq!(one.path, "/projects/{id}");
    assert_eq!(one.params["id"].kind, "string");
    assert_eq!(one.desc, "Read one of the projects on Projects");
}

#[test]
fn rows_with_a_named_title_but_no_links_are_records_with_a_date_and_an_offset() {
    let draft = from_pages(
        &[page("https://market.example/search?q=bicycle", "rows.html")],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let list = spec.op("get-search").unwrap();
    assert_eq!(list.desc, "List the results on Search results");
    assert_eq!(list.query["offset"].default, None);
    assert_eq!(list.query["q"].default, Some("bicycle".into()));
    let html = list.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.items, "div.results > div.result");
    assert_eq!(html.fields["title"].selector, "span.result__title");
    assert_eq!(html.fields["price"].selector, "span.result__price");
    assert_eq!(html.fields["published_at"].selector, "span.result__date");
    assert!(!html.fields.contains_key("link"));
}

#[test]
fn an_article_is_read_as_one_page_with_its_headline_body_and_byline() {
    let draft = from_pages(
        &[page(
            "https://blog.example/posts/why-the-gate-waits",
            "article.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let post = spec.op("get-why-the-gate-waits").unwrap();
    assert_eq!(post.desc, "Read Why the gate waits");
    let response = post.response.as_ref().unwrap();
    assert_eq!(response.required_html_fields[0].selector, "h1.post-title");
    let html = response.html.as_ref().unwrap();
    assert!(html.items.is_empty());
    assert!(html.page["headline"].required);
    assert_eq!(html.page["body"].selector, "div.post-content");
    assert_eq!(html.page["byline"].selector, "p.post-byline");
    assert_eq!(html.page["published_at"].attribute, "datetime");
    assert_eq!(html.page["author"].attribute, "content");
    assert_eq!(html.page["description"].attribute, "content");
    let value = html
        .extract(
            &fixture("article.html"),
            "https://blog.example/posts/why-the-gate-waits",
        )
        .unwrap();
    assert_eq!(value["headline"], "Why the gate waits");
    assert_eq!(value["author"], "Mina Park");
    assert_eq!(value["published_at"], "2026-03-01T09:00:00Z");
    assert!(
        value["body"]
            .as_str()
            .unwrap()
            .starts_with("Every request to a site")
    );
    assert!(
        post.evidence
            .starts_with("GET /posts/why-the-gate-waits is one page with a headline, a body"),
        "{}",
        post.evidence
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("reads the page's own content as")),
        "{:?}",
        draft.notes
    );
}

#[test]
fn a_product_page_reads_its_labelled_values_but_never_a_secret_one() {
    let draft = from_pages(
        &[page(
            "https://shop.example/items/widget-pro",
            "product.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let item = spec.op("get-widget-pro").unwrap();
    let html = item.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.page["headline"].selector, "h1.product-name");
    assert_eq!(html.page["price"].selector, "p.product-price");
    assert_eq!(
        html.page["weight"].selector,
        "table.specs tr:nth-child(1) > td"
    );
    assert_eq!(
        html.page["warranty"].selector,
        "table.specs tr:nth-child(3) > td"
    );
    assert_eq!(html.page["sku"].selector, "dl.meta > dd:nth-of-type(1)");
    assert_eq!(html.page["color"].selector, "dl.meta > dd:nth-of-type(2)");
    assert!(!html.page.contains_key("api-key"), "{:?}", html.page.keys());
    assert!(!html.page.contains_key("body"));
    let value = html
        .extract(
            &fixture("product.html"),
            "https://shop.example/items/widget-pro",
        )
        .unwrap();
    assert_eq!(value["weight"], "1.2 kg");
    assert_eq!(value["color"], "Blue");
    assert_eq!(value["price"], "99.00");
    assert_eq!(
        item.evidence,
        "GET /items/widget-pro is one page with a headline, 1 more fields, 5 labelled values"
    );
}

#[test]
fn links_that_differ_by_a_query_key_become_a_read_by_that_key() {
    let draft = from_pages(
        &[page(
            "https://board.example/list.php?board=free",
            "ids.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    assert_eq!(draft.operations, 2, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let list = spec.op("get-list").unwrap();
    assert_eq!(list.desc, "List the posts on Free board");
    assert_eq!(list.query["offset"].default, None);
    let html = list.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.fields["writer"].selector, "span.writer");
    let one = spec.op("get-post").unwrap();
    assert_eq!(one.path, "/view.php");
    assert_eq!(one.query["id"].kind, "int");
    assert!(one.query["id"].required);
    assert_eq!(one.query["board"].default, Some("free".into()));
    assert_eq!(
        one.evidence,
        "the records of GET /list.php link to /view.php?id="
    );
}

#[test]
fn a_page_with_nothing_to_read_is_still_only_a_title_check() {
    let draft = from_pages(
        &[page(
            "https://pypi.example/project/requests/",
            "challenge.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert_eq!(draft.operations, 1, "{}", draft.yaml);
    let spec = spec_of(&draft);
    let op = &spec.operations[0];
    let html = op.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.page.len(), 1);
    assert_eq!(html.page["title"].selector, "title");
    assert!(html.items.is_empty());
    assert!(!draft.yaml.contains("noscript"), "{}", draft.yaml);
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("scripts draw the page") || n.contains("checks only the title")),
        "{:?}",
        draft.notes
    );
}

#[test]
fn a_page_with_page_links_but_no_records_does_not_turn_pages() {
    let html = r#"<html><head><title>About</title><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Example","blocks":[{"a":1}]}</script></head>
<body><h1>About us</h1><p>Text.</p><a href="/about?page=2">2</a><a href="/about?page=3">3</a></body></html>"#;
    let draft = from_pages(
        &[Page {
            url: "https://example.org/about".into(),
            html: html.into(),
        }],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    for op in &spec.operations {
        assert!(op.pagination.is_none(), "{}: {}", op.name, draft.yaml);
    }
}

#[test]
fn links_that_count_records_declare_an_offset() {
    let html = r#"<html><head><title>Forum</title></head><body><h1>Topics</h1>
<table><thead><tr><th>Title</th><th>Replies</th></tr></thead><tbody>
<tr><td><a href="/t/1">First</a></td><td>3</td></tr>
<tr><td><a href="/t/2">Second</a></td><td>0</td></tr>
<tr><td><a href="/t/3">Third</a></td><td>7</td></tr>
</tbody></table>
<a href="/viewforum?start=25">2</a><a href="/viewforum?start=50">3</a></body></html>"#;
    let draft = from_pages(
        &[Page {
            url: "https://forum.example/viewforum".into(),
            html: html.into(),
        }],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let list = spec
        .operations
        .iter()
        .find(|op| op.pagination.is_some())
        .unwrap_or_else(|| panic!("{}", draft.yaml));
    let pagination = list.pagination.as_ref().unwrap();
    assert_eq!(
        (pagination.style.as_str(), pagination.next.as_str()),
        ("offset", "")
    );
    assert_eq!(pagination.input(), ("start", false));
    assert_eq!(list.query["start"].default, None);
}

#[test]
fn a_list_page_with_page_links_asks_for_the_drafted_address_until_told_a_page() {
    let draft = from_pages(
        &[page("https://intranet.example.co.kr/board", "board.html")],
        &Options::default(),
    )
    .unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    let board = spec.op("get-board").unwrap();
    assert_eq!(board.query["page"].kind, "int");
    assert_eq!(board.query["page"].default, None);
    assert!(!board.query["page"].required);
    let (url, _) = spec.build_request(board, &Default::default()).unwrap();
    assert_eq!(url, "https://intranet.example.co.kr/board");
    let second = [("page".to_owned(), "2".to_owned())].into_iter().collect();
    let (url, _) = spec.build_request(board, &second).unwrap();
    assert_eq!(url, "https://intranet.example.co.kr/board?page=2");
}

#[test]
fn pages_read_signed_in_keep_the_sign_in_and_need_the_session_everywhere() {
    let draft = from_pages(
        &[page(
            "https://intranet.example.co.kr/board?page=1",
            "board.html",
        )],
        &Options {
            name: Some("board".into()),
            session: Some(itda_prep::SessionAuth {
                cookie: "JSESSIONID".into(),
                query: None,
                login: itda_spec::LoginForm {
                    path: "/login".into(),
                    user: "user_id".into(),
                    password: "pw".into(),
                },
            }),
            ..Options::default()
        },
    )
    .unwrap();
    let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
    itda_policy::validate_spec(&spec).unwrap();
    let session = spec.find_auth("session").unwrap();
    assert_eq!(session.kind, "browser");
    assert_eq!(session.value_from, "cookie:JSESSIONID");
    let login = session.login.as_ref().unwrap();
    assert_eq!(
        (login.path.as_str(), login.user.as_str()),
        ("/login", "user_id")
    );
    assert!(
        spec.operations.iter().all(|op| op.auth == "session"),
        "{}",
        draft.yaml
    );
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("`itda login board` signs in"))
    );
    assert!(!draft.notes.iter().any(|n| n.contains("--browser")));
}

#[test]
fn a_portal_shell_drafts_its_frame_pages_under_their_own_addresses() {
    let shell = page("https://intranet.example.co.kr/portal/", "portal.html");
    let board = page(
        "https://intranet.example.co.kr/frame/board.do?bbs=1",
        "frame-board.html",
    );
    // The caller fetched the same-origin frame and passes it along; each
    // page is drafted under the address a request to the server would use.
    let draft = from_pages(&[shell.clone(), board], &Options::default()).unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = spec_of(&draft);
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        ["get-portal", "get-board", "get-view", "post-write"],
        "{}",
        draft.yaml
    );
    let list = spec.op("get-board").unwrap();
    assert_eq!(list.path, "/frame/board.do");
    assert_eq!(list.query["bbs"].default, Some(1.into()));
    let html = list.response.as_ref().unwrap().html.as_ref().unwrap();
    assert_eq!(html.items, "table tbody tr");
    assert_eq!(html.fields["title"].selector, "td:nth-child(2)");
    let view = spec.op("get-view").unwrap();
    assert_eq!(view.path, "/frame/view.do");
    assert!(view.query["id"].required);
    let write = spec.op("post-write").unwrap();
    assert_eq!(write.path, "/frame/write.do");
    assert_eq!(write.preflight[0].path, "/frame/board.do?bbs=1");
    assert!(!draft.yaml.contains("frame-token-9"));
    assert_eq!(
        draft.thin_reads, 1,
        "only the shell checks nothing but a title"
    );
    assert!(
        draft.notes.iter().any(|n| n
            == "/portal/: shows https://cdn.example.net/widget in a frame on another site; left out."),
        "{:?}",
        draft.notes
    );
    assert!(
        !draft.notes.iter().any(|n| n.contains("was not read")),
        "{:?}",
        draft.notes
    );

    // The shell alone is a title check, and the draft says what it skipped.
    let alone = from_pages(std::slice::from_ref(&shell), &Options::default()).unwrap();
    assert_eq!(
        (alone.operations, alone.thin_reads),
        (1, 1),
        "{}",
        alone.yaml
    );
    assert!(
        alone.notes.iter().any(|n| n.starts_with(
            "/portal/: shows /frame/board.do in a frame that was not read; `itda draft page` and `itda setup` follow"
        )),
        "{:?}",
        alone.notes
    );

    // A frame page that is itself a frameset names its own frames the same way.
    let nested = from_pages(
        &[page(
            "https://intranet.example.co.kr/frame/main.do",
            "frame-nested.html",
        )],
        &Options::default(),
    )
    .unwrap();
    let noted: Vec<&String> = nested
        .notes
        .iter()
        .filter(|n| n.contains("in a frame that was not read"))
        .collect();
    assert_eq!(noted.len(), 2, "{:?}", nested.notes);
    assert!(noted[0].contains("/frame/menu.do"), "{:?}", noted);
    assert!(noted[1].contains("/frame/board.do"), "{:?}", noted);
}

#[test]
fn pages_read_with_a_query_session_name_the_query_key_instead_of_a_cookie() {
    let draft = from_pages(
        &[page("https://intranet.example.co.kr/board", "board.html")],
        &Options {
            name: Some("board".into()),
            session: Some(itda_prep::SessionAuth {
                cookie: "lang".into(),
                query: Some("sid".into()),
                login: itda_spec::LoginForm {
                    path: "/login".into(),
                    user: "user_id".into(),
                    password: "pw".into(),
                },
            }),
            ..Options::default()
        },
    )
    .unwrap();
    let spec = spec_of(&draft);
    itda_policy::validate_spec(&spec).unwrap();
    let session = spec.find_auth("session").unwrap();
    assert_eq!(session.value_from, "query:sid");
    assert!(session.login.is_some());
    assert!(spec.operations.iter().all(|op| op.auth == "session"));
}

#[test]
fn attachments_become_file_reads_and_an_upload_form_a_multipart_change() {
    let draft = from_pages(
        &[page(
            "https://intranet.example.co.kr/board/41",
            "attachments.html",
        )],
        &Options::default(),
    )
    .unwrap();
    assert!(draft.problems.is_empty(), "{:?}", draft.problems);
    let spec = spec_of(&draft);
    itda_policy::validate_spec(&spec).unwrap();
    let names: Vec<&str> = spec.operations.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"download-file"), "{names:?}");
    assert!(names.contains(&"download-download"), "{names:?}");
    assert!(names.contains(&"post-attach"), "{names:?}");
    assert_eq!(draft.reads, draft.operations - 1);

    let by_path = spec.op("download-file").unwrap();
    assert_eq!(by_path.path, "/board/files/{file_id}");
    assert_eq!(by_path.desc, "Download an attachment from Minutes");
    assert!(by_path.params["file_id"].required);
    let response = by_path.response.as_ref().unwrap();
    assert_eq!(response.format, "file");
    // The links name no kind of file, so the read accepts the usual documents.
    assert!(
        response.media_types.contains(&"application/pdf".to_owned()),
        "{:?}",
        response.media_types
    );
    assert!(
        response
            .media_types
            .contains(&"application/octet-stream".to_owned())
    );
    assert_eq!(
        by_path.evidence,
        "/board/41 links to GET /board/files/{file_id} as an attachment (2 links)"
    );
    assert_eq!(
        itda_policy::operation_effect(by_path, &Default::default()),
        itda_policy::Effect::Read
    );

    let by_query = spec.op("download-download").unwrap();
    assert_eq!(by_query.path, "/download.php");
    assert_eq!(
        (
            by_query.query["id"].kind.as_str(),
            by_query.query["id"].required
        ),
        ("int", true)
    );
    assert_eq!(by_query.query["type"].default, Some("board".into()));

    let upload = spec.op("post-attach").unwrap();
    assert_eq!(upload.desc, "Upload a file to Minutes");
    assert_eq!(upload.auth, "session");
    let multipart = upload.body.as_ref().unwrap().multipart.as_ref().unwrap();
    assert_eq!(
        multipart.file,
        Some(itda_spec::FilePart {
            name: "attachment".into(),
            from: "attachment".into()
        })
    );
    assert_eq!(multipart.fields["memo"].kind, "string");
    assert!(multipart.fields["_csrf"].from_step());
    assert_eq!(upload.preflight[0].path, "/board/41");
    assert_eq!(upload.file_input().map(String::as_str), Some("attachment"));
    assert!(!draft.yaml.contains("recorded-secret-token"));
    assert!(
        draft
            .notes
            .iter()
            .any(|n| n.contains("you approve the file by name, size and hash")),
        "{:?}",
        draft.notes
    );
    assert!(
        !draft
            .notes
            .iter()
            .any(|n| n.contains("do not cover file uploads")),
        "{:?}",
        draft.notes
    );
    assert!(
        !spec
            .operations
            .iter()
            .any(|o| o.path == "/board/files/101.pdf"),
        "{names:?}"
    );
}
