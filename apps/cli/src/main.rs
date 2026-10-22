//! The `itda` command. This is the composition root: it picks the data
//! directory, the secret store and the network settings, and hands them to
//! the runtime. Commands that need no network never start an async runtime.
mod feedback;
mod install;
mod output;
mod refresh;
mod setup;
mod text;

use clap::{CommandFactory, Parser, Subcommand};
use itda_net::{Gate, Network};
use itda_policy::{Approvals, Cursors, operation_effect};
use itda_runtime::{
    Call, Runtime,
    audit::{AuditLog, Event},
};
use itda_setup::{check, sites};
use itda_spec::{Operation, Spec};
use itda_store::Store;
use itda_types::error::{AppError, AppResult, usage};
use itda_vault::{EnvStore, KeyringStore, SealedStore, Secret, SecretStore, Vault};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

#[derive(Parser)]
#[command(
    name = "itda",
    version,
    about = "Verified website actions for AI agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List installed sites and their actions
    Ls {
        /// Only this site
        site: Option<String>,
        /// Print JSON for scripts and agents
        #[arg(long)]
        json: bool,
    },
    /// Show a site's actions with their inputs and an example command each:
    /// itda describe <site> or itda describe <site>.<action>
    Describe {
        /// <site> or <site>.<action>
        target: String,
        /// Print JSON for scripts and agents
        #[arg(long)]
        json: bool,
    },
    /// Install a site definition after showing where it sends requests and
    /// which actions change the site
    Add {
        /// A file, an https URL, - for stdin, or a ready-made site: github,
        /// npm, crates-io or pypi
        source: String,
        /// Replace the site if it is already installed
        #[arg(long)]
        force: bool,
    },
    /// Remove an installed site definition
    Remove { site: String },
    /// Call an action: itda call <site>.<action> [--input value]... [--account id] [--json]
    Call {
        /// <site>.<action>
        target: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },
    /// Sign a site account in: through the site's own sign-in form with your
    /// user name and password (the password read from stdin), by storing an
    /// API key read from stdin, or with --browser in your own browser
    Login {
        site: String,
        #[arg(long, default_value = itda_runtime::DEFAULT_ACCOUNT)]
        account: String,
        /// Sign in in a browser window instead of typing a password here
        #[arg(long)]
        browser: bool,
        /// The page to open for --browser; defaults to the site's address
        #[arg(long, requires = "browser")]
        url: Option<String>,
        /// The user name for the site's sign-in form; asked for when left out
        #[arg(long, conflicts_with_all = ["browser", "secret"])]
        user: Option<String>,
        /// Store an API key under this secret name instead (store:<site>/<name>);
        /// api-key when the site has no sign-in form
        #[arg(long)]
        secret: Option<String>,
        /// A label to recognise the account by
        #[arg(long, default_value = "")]
        label: String,
    },
    /// List signed-in accounts and the names of their secrets, never the values
    Accounts {
        /// Only this site
        site: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Remove an account and every secret saved for it
    Logout {
        site: String,
        #[arg(long, default_value = itda_runtime::DEFAULT_ACCOUNT)]
        account: String,
    },
    /// Resume requests paused after a block or an expired sign-in
    Resume {
        site: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// List changes waiting for your approval
    Approvals {
        #[arg(long)]
        json: bool,
    },
    /// Review a change an agent asked for and approve it. Needs a terminal
    Approve { id: String },
    /// Refuse a change an agent asked for
    Reject { id: String },
    /// Draft a site definition from an API description or a browser recording
    Draft {
        #[command(subcommand)]
        source: DraftSource,
    },
    /// Set a site up in one go: look at the page (or record you using it),
    /// name what it offers, try every read, keep what works, and connect an
    /// agent. Asks before each step; flags answer for scripts
    Setup {
        /// The page to start from
        url: String,
        /// Site id; taken from the host name when left out
        #[arg(long)]
        name: Option<String>,
        /// Record in your browser instead of fetching the page
        #[arg(long, conflicts_with = "no_record")]
        record: bool,
        /// Never open the browser; fail if the page cannot be read
        #[arg(long)]
        no_record: bool,
        /// Use the template for this software whatever the page says:
        /// gitlab, jira, confluence, redmine, discourse, mediawiki, wordpress
        /// or gitea (also Forgejo)
        #[arg(long = "as", value_name = "SOFTWARE", conflicts_with_all = ["no_template", "record"])]
        software: Option<String>,
        /// Read the page like any other, even when it runs known software
        #[arg(long)]
        no_template: bool,
        /// Only watch the page load in your browser when it shows itda
        /// nothing; never ask you to record
        #[arg(long, conflicts_with_all = ["record", "no_record"])]
        watch_only: bool,
        /// The agent to connect: claude-desktop, claude-code, cursor, vscode,
        /// codex or none
        #[arg(long)]
        client: Option<String>,
        /// Answer yes to every question, including replacing an installed site
        #[arg(long, short = 'y')]
        yes: bool,
        /// Sign in with this user name when the page needs one; the password
        /// is asked for, or read from stdin without a terminal
        #[arg(long)]
        user: Option<String>,
    },
    /// Use a site in your own browser while itda records the requests, then
    /// draft a site definition from them. Nothing is written to disk unless
    /// --save asks for it
    Record {
        /// The page to open
        url: String,
        /// Site id; taken from the host name when left out
        #[arg(long)]
        name: Option<String>,
        /// Only calls under this address (default: the most used one)
        #[arg(long)]
        base_url: Option<String>,
        /// Keep only calls whose path contains this
        #[arg(long)]
        include: Option<String>,
        /// Install the draft into the data directory instead of printing it
        #[arg(long)]
        install: bool,
        /// Replace an installed definition with the same name
        #[arg(long, requires = "install")]
        force: bool,
        /// Also save the recording as a HAR file. It holds your cookies and
        /// tokens; delete it when you are done
        #[arg(long)]
        save: Option<PathBuf>,
    },
    /// Show what agents did, newest last; --verify checks the log was not changed
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        json: bool,
    },
    /// Check the data directory, the keyring and the installed sites
    Doctor,
    /// Report a problem: print a link to a new GitHub issue with the version,
    /// the platform and the doctor checks filled in, and open it. Sends nothing
    Feedback {
        /// Only print the link
        #[arg(long)]
        no_open: bool,
    },
    /// Run every read that needs no input and report whether the site still
    /// answers the way its definition says. Changes are never sent
    Check {
        /// Only this site
        site: Option<String>,
        /// Every installed site, one verdict line each
        #[arg(long, conflicts_with = "site")]
        all: bool,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Draft a site again from the sources it was set up from, show what
    /// would change, and keep what you decided: the sign-in, the defaults,
    /// the name and every action you edited by hand. Then checks the site
    Refresh {
        site: String,
        /// Write the changes without asking
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Serve installed actions to an AI agent over MCP (stdio)
    Mcp {
        #[command(subcommand)]
        command: Option<McpCommand>,
        /// direct: one tool per action. catalog: search, describe and call.
        /// Chosen from the number of actions when left out
        #[arg(long, value_parser = ["direct", "catalog"])]
        mode: Option<String>,
        /// Ask you to approve changes in the MCP client's own form, for clients
        /// that show elicitation requests to you (also ITDA_APPROVE_IN_CLIENT=1)
        #[arg(long)]
        approve_in_client: bool,
        /// Serve this one site only: its actions are the only tools, and
        /// itda_setup, itda_login, itda_check and itda_manage stay inside it
        #[arg(long)]
        site: Option<String>,
    },
}

#[derive(Subcommand)]
enum DraftSource {
    /// From an OpenAPI 3 or Swagger 2.0 file (JSON or YAML)
    Openapi {
        file: PathBuf,
        /// Site id; taken from the document's title when left out
        #[arg(long)]
        name: Option<String>,
        /// Use this address instead of the document's server URL
        #[arg(long)]
        base_url: Option<String>,
        /// Keep only operations whose path, operationId or tag contains this
        #[arg(long)]
        include: Option<String>,
        /// Install the draft into the data directory instead of printing it
        #[arg(long)]
        install: bool,
        /// Replace an installed definition with the same name
        #[arg(long, requires = "install")]
        force: bool,
    },
    /// From pages you use: fetches each address and drafts actions from its
    /// tables, lists, embedded data, forms and feeds
    Page {
        /// One or more page addresses on the same site
        #[arg(required = true)]
        urls: Vec<String>,
        /// Site id; taken from the host name when left out
        #[arg(long)]
        name: Option<String>,
        /// The site's address; defaults to the first page's origin
        #[arg(long)]
        base_url: Option<String>,
        /// Keep only pages and forms whose path contains this
        #[arg(long)]
        include: Option<String>,
        /// Install the draft into the data directory instead of printing it
        #[arg(long)]
        install: bool,
        /// Replace an installed definition with the same name
        #[arg(long, requires = "install")]
        force: bool,
    },
    /// From a browser recording (HAR) of you using the site; keeps only the
    /// shape of each call, never the recorded values
    Har {
        file: PathBuf,
        /// Site id; taken from the host name when left out
        #[arg(long)]
        name: Option<String>,
        /// Only calls under this address (default: the most used one)
        #[arg(long)]
        base_url: Option<String>,
        /// Keep only calls whose path contains this
        #[arg(long)]
        include: Option<String>,
        /// Install the draft into the data directory instead of printing it
        #[arg(long)]
        install: bool,
        /// Replace an installed definition with the same name
        #[arg(long, requires = "install")]
        force: bool,
    },
}

#[derive(Subcommand)]
enum McpCommand {
    /// Add itda to an MCP client: claude-desktop, claude-code, cursor, vscode or codex
    Install {
        client: String,
        /// Show the change without writing it
        #[arg(long)]
        dry_run: bool,
        /// Approve changes in the client instead of with itda approve
        #[arg(long)]
        approve_in_client: bool,
    },
}

fn main() -> ExitCode {
    let first = match shortcut(std::env::args_os().collect()) {
        Shortcut::Args(args) => Cli::parse_from(args).command.map(First::Run),
        Shortcut::Bundled(name) => Some(First::Bundled(name)),
    };
    let command = match first.or_else(welcome) {
        Some(First::Run(command)) => command,
        Some(First::Bundled(name)) => {
            return match home().and_then(|home| {
                block_on(add(&home, &name, false))?;
                setup::connect_detected()?;
                Ok(ExitCode::SUCCESS)
            }) {
                Ok(code) => code,
                Err(error) => output::fail(&error, false),
            };
        }
        None => {
            Cli::command().print_help().ok();
            return ExitCode::SUCCESS;
        }
    };
    let as_json = matches!(&command, Command::Ls { json: true, .. })
        || matches!(&command, Command::Describe { json: true, .. })
        || matches!(&command, Command::Approvals { json: true })
        || matches!(&command, Command::Accounts { json: true, .. })
        || matches!(&command, Command::Audit { json: true, .. })
        || matches!(&command, Command::Check { json: true, .. })
        || matches!(&command, Command::Call { rest, .. } if rest.iter().any(|a| a == "--json"));
    let result = home().and_then(|home| match command {
        Command::Ls { site, json } => ls(&home, site.as_deref(), json),
        Command::Describe { target, json } => describe(&home, &target, json),
        Command::Add { source, force } => block_on(add(&home, &source, force)),
        Command::Remove { site } => remove(&home, &site),
        Command::Call { target, rest } => block_on(call(&home, &target, rest)),
        Command::Login {
            site,
            account,
            browser: true,
            url,
            label,
            ..
        } => block_on(login_browser(&home, &site, &account, url, &label)),
        Command::Login {
            site,
            account,
            user,
            secret,
            label,
            ..
        } => block_on(login(
            &home,
            &site,
            &account,
            user.as_deref(),
            secret.as_deref(),
            &label,
        )),
        Command::Accounts { site, json } => block_on(accounts(&home, site.as_deref(), json)),
        Command::Logout { site, account } => block_on(logout(&home, &site, &account)),
        Command::Resume { site, account } => block_on(resume(&home, &site, account.as_deref())),
        Command::Approvals { json } => block_on(approvals(&home, json)),
        Command::Approve { id } => block_on(approve(&home, &id)),
        Command::Reject { id } => block_on(reject(&home, &id)),
        Command::Draft { source } => match source {
            DraftSource::Openapi {
                file,
                name,
                base_url,
                include,
                install,
                force,
            } => draft(
                &home,
                itda_prep::from_openapi,
                &file,
                itda_prep::Options {
                    name,
                    base_url,
                    include,
                    lang: lang(),
                    schemas: Vec::new(),
                    session: None,
                    frames_to_sign_in: Vec::new(),
                },
                install,
                force,
            ),
            DraftSource::Har {
                file,
                name,
                base_url,
                include,
                install,
                force,
            } => block_on(draft_har(
                &home,
                &file,
                itda_prep::Options {
                    name,
                    base_url,
                    include,
                    lang: lang(),
                    schemas: Vec::new(),
                    session: None,
                    frames_to_sign_in: Vec::new(),
                },
                install,
                force,
            )),
            DraftSource::Page {
                urls,
                name,
                base_url,
                include,
                install,
                force,
            } => block_on(draft_page(
                &home,
                &urls,
                itda_prep::Options {
                    name,
                    base_url,
                    include,
                    lang: lang(),
                    schemas: Vec::new(),
                    session: None,
                    frames_to_sign_in: Vec::new(),
                },
                install,
                force,
            )),
        },
        Command::Setup {
            url,
            name,
            record,
            no_record,
            software,
            no_template,
            watch_only,
            client,
            yes,
            user,
        } => block_on(setup::run(
            &home,
            setup::Flags {
                url,
                name,
                user,
                record: if record {
                    Some(true)
                } else if no_record {
                    Some(false)
                } else {
                    None
                },
                template: match software {
                    Some(name) => itda_setup::Template::Named(name),
                    None if no_template => itda_setup::Template::Off,
                    None => itda_setup::Template::Auto,
                },
                watch_only,
                client,
                yes,
            },
        )),
        Command::Record {
            url,
            name,
            base_url,
            include,
            install,
            force,
            save,
        } => block_on(record(
            &home,
            &url,
            itda_prep::Options {
                name,
                base_url,
                include,
                lang: lang(),
                schemas: Vec::new(),
                session: None,
                frames_to_sign_in: Vec::new(),
            },
            install,
            force,
            save.as_deref(),
        )),
        Command::Audit {
            limit,
            verify,
            json,
        } => audit(&home, limit, verify, json),
        Command::Doctor => Ok(doctor(&home)),
        Command::Feedback { no_open } => Ok(feedback(&home, no_open)),
        Command::Check {
            site,
            all,
            account,
            json,
        } => block_on(check(&home, site.as_deref(), all, account.as_deref(), json)),
        Command::Refresh { site, yes } => block_on(refresh::run(&home, &site, yes)),
        Command::Mcp {
            command:
                Some(McpCommand::Install {
                    client,
                    dry_run,
                    approve_in_client,
                }),
            ..
        } => mcp_install(&client, dry_run, approve_in_client),
        Command::Mcp {
            command: None,
            mode,
            approve_in_client,
            site,
        } => block_on(mcp(
            &home,
            mode.as_deref(),
            approve_in_client || std::env::var("ITDA_APPROVE_IN_CLIENT").is_ok_and(|v| v == "1"),
            site.as_deref(),
        )),
    });
    match result {
        Ok(code) => code,
        Err(error) => output::fail(&error, as_json),
    }
}

/// `itda <address>` is `itda setup <address>`, and `itda github` adds the
/// ready-made site, so the one line people read is the one line they type.
enum Shortcut {
    Args(Vec<std::ffi::OsString>),
    Bundled(String),
}

fn shortcut(args: Vec<std::ffi::OsString>) -> Shortcut {
    let Some(first) = args.get(1).and_then(|a| a.to_str()) else {
        return Shortcut::Args(args);
    };
    let known = first.starts_with('-')
        || first == "help"
        || Cli::command()
            .get_subcommands()
            .any(|c| c.get_name() == first || c.get_all_aliases().any(|a| a == first));
    if known {
        return Shortcut::Args(args);
    }
    if args.len() == 2 && sites::bundled(first).is_some() {
        return Shortcut::Bundled(first.to_owned());
    }
    let address = first.contains("://") || first.contains('.') || first.starts_with("localhost");
    if !address {
        return Shortcut::Args(args);
    }
    let url = if first.contains("://") {
        first.to_owned()
    } else {
        format!("https://{first}")
    };
    let mut out = args.clone();
    out[1] = url.into();
    out.insert(1, "setup".into());
    Shortcut::Args(out)
}

/// What a bare `itda` turns into.
enum First {
    Run(Command),
    /// A ready-made site's name was typed: add it, then connect the agents.
    Bundled(String),
}

/// A bare `itda` in a terminal: say what it is and ask for one address.
/// `None` when nothing was typed, so the command list shows instead.
fn welcome() -> Option<First> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return None;
    }
    eprintln!("{}", text::t("welcome"));
    eprint!("{}", text::t("ask_url"));
    std::io::stderr().flush().ok();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).ok()?;
    first_command(&line)
}

fn first_command(typed: &str) -> Option<First> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if sites::bundled(typed).is_some() {
        return Some(First::Bundled(typed.to_owned()));
    }
    let url = if typed.contains("://") {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    };
    Some(First::Run(Command::Setup {
        url,
        name: None,
        record: false,
        no_record: false,
        software: None,
        no_template: false,
        watch_only: false,
        client: None,
        user: None,
        yes: false,
    }))
}

fn home() -> AppResult<PathBuf> {
    itda_types::paths::home().ok_or_else(|| {
        usage(
            "itda found no data directory",
            "Set ITDA_HOME to a directory itda may use.",
        )
    })
}

fn block_on<F: Future<Output = AppResult<ExitCode>>>(future: F) -> AppResult<ExitCode> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

/// `ITDA_LANG`, else the system `LANG`: the language of what people read.
fn lang() -> String {
    std::env::var("ITDA_LANG")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default()
}

/// Private and local addresses are refused unless `ITDA_ALLOW_LOCAL=1`, which
/// intranet and sandbox sites need.
fn allow_local() -> bool {
    std::env::var("ITDA_ALLOW_LOCAL").is_ok_and(|v| v == "1")
}

/// `ITDA_SECRET_STORE=env` reads `ITDA_SECRET__` variables instead of the
/// keyring. Otherwise short secrets go to the keyring and long ones (browser
/// sessions) into `vault.sealed` under the data directory, sealed with a key
/// that lives in the keyring.
fn secret_store(home: &Path) -> Arc<dyn SecretStore> {
    if std::env::var("ITDA_SECRET_STORE").is_ok_and(|v| v == "env") {
        Arc::new(EnvStore::from_env())
    } else {
        Arc::new(SealedStore::new(home, Arc::new(KeyringStore::new())))
    }
}

struct Parts {
    net: Arc<Network>,
    vault: Arc<Vault>,
    approvals: Arc<Approvals>,
    cursors: Arc<Cursors>,
    audit: Arc<AuditLog>,
}

impl Parts {
    /// Reads run directly; changes wait for the user's approval. Everything
    /// is recorded in the audit log, and reads that turn pages keep their
    /// markers in the local database.
    fn runtime(self) -> Runtime {
        Runtime::new(self.net, self.vault)
            .with_approvals(self.approvals)
            .with_pagination(self.cursors)
            .with_audit(self.audit)
    }

    /// The same parts, with a runtime built on them, for the setup steps.
    fn context(self, home: &Path) -> itda_setup::Context {
        let (net, vault, approvals, audit) = (
            self.net.clone(),
            self.vault.clone(),
            self.approvals.clone(),
            self.audit.clone(),
        );
        itda_setup::Context {
            home: home.to_path_buf(),
            runtime: Arc::new(self.runtime()),
            net,
            vault,
            approvals,
            audit,
            lang: lang(),
        }
    }
}

fn audit_log(home: &Path) -> AuditLog {
    AuditLog::new(home.join("audit.jsonl"))
}

fn open(home: &Path) -> AppResult<Parts> {
    let store = Store::open(&home.join("itda.db"))?;
    let gate = Arc::new(Gate::open(store.clone())?);
    Ok(Parts {
        net: Arc::new(Network::new(gate, allow_local())),
        approvals: Arc::new(Approvals::open(store.clone())?),
        cursors: Arc::new(Cursors::open(store.clone())?),
        audit: Arc::new(audit_log(home)),
        vault: Arc::new(Vault::open(store, secret_store(home))?),
    })
}

fn ls(home: &Path, only: Option<&str>, as_json: bool) -> AppResult<ExitCode> {
    let installed: Vec<_> = sites::list(home)
        .into_iter()
        .filter(|site| only.is_none_or(|name| name == site.name))
        .collect();
    if let Some(name) = only
        && installed.is_empty()
    {
        sites::load(home, name)?;
    }
    if as_json {
        let rows: Vec<Value> = installed
            .iter()
            .map(|site| match &site.spec {
                Ok(spec) => json!({
                    "site": site.name,
                    "title": spec.site.title,
                    "base_url": spec.site.base_url,
                    "actions": spec.operations.iter().map(|op| json!({
                        "name": op.name,
                        "effect": operation_effect(op, &BTreeMap::new()).as_str(),
                        "desc": op.desc,
                        "example": example_command(&spec.site.name, op),
                    })).collect::<Vec<_>>(),
                }),
                Err(error) => json!({"site": site.name, "path": site.path, "error": error.msg}),
            })
            .collect();
        println!("{}", Value::Array(rows));
        return Ok(ExitCode::SUCCESS);
    }
    if installed.is_empty() {
        println!(
            "No sites installed. Put a site definition at {}",
            sites::specs_dir(home).join("<site>/spec.yaml").display()
        );
    }
    for site in &installed {
        match &site.spec {
            Ok(spec) => {
                println!("{}  {}", site.name, spec.site.base_url);
                for op in &spec.operations {
                    let effect = operation_effect(op, &BTreeMap::new());
                    println!("  {:<6} {:<28} {}", effect.as_str(), op.name, op.desc);
                }
            }
            Err(error) => println!(
                "{}  invalid: {} ({})",
                site.name,
                error.msg,
                site.path.display()
            ),
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Every action of a site, or one of them, with its inputs and a command
/// to copy. The example names the inputs the action cannot run without.
fn describe(home: &Path, target: &str, as_json: bool) -> AppResult<ExitCode> {
    let (site, action) = match target.split_once('.') {
        Some((site, action)) => (site, Some(action)),
        None => (target, None),
    };
    let spec = sites::load(home, site)?;
    let ops: Vec<&Operation> = match action {
        Some(action) => vec![spec.op(action)?],
        None => spec.operations.iter().collect(),
    };
    if as_json {
        let rows: Vec<Value> = ops.iter().map(|op| describe_json(&spec, op)).collect();
        println!("{}", Value::Array(rows));
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}  {}", spec.site.name, spec.site.base_url);
    for op in ops {
        let effect = operation_effect(op, &BTreeMap::new());
        let word = text::t(if effect == itda_policy::Effect::Read {
            "read"
        } else {
            "change"
        });
        println!("  {word} {:<28} {}", op.name, op.desc);
        let inputs: Vec<String> = declared_inputs(op)
            .iter()
            .map(|(name, kind, required, default)| {
                let mut notes = vec![kind.clone()];
                if *required {
                    notes.push(text::t("required").to_owned());
                }
                if let Some(default) = default {
                    notes.push(text::tf(
                        "default",
                        &[("value", &itda_types::util::plain(default))],
                    ));
                }
                format!("{name} ({})", notes.join(", "))
            })
            .collect();
        if !inputs.is_empty() {
            println!(
                "         {}",
                text::tf("inputs", &[("list", &inputs.join(", "))])
            );
        }
        println!(
            "         {}",
            text::tf(
                "example",
                &[("command", &example_command(&spec.site.name, op))]
            )
        );
        if effect != itda_policy::Effect::Read {
            println!("         {}", text::t("approve_note"));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn describe_json(spec: &Spec, op: &Operation) -> Value {
    json!({
        "site": spec.site.name,
        "action": op.name,
        "effect": operation_effect(op, &BTreeMap::new()).as_str(),
        "desc": op.desc,
        "inputs": declared_inputs(op).iter().map(|(name, kind, required, default)| json!({
            "name": name,
            "type": kind,
            "required": required,
            "default": default,
        })).collect::<Vec<_>>(),
        "example": example_command(&spec.site.name, op),
    })
}

/// The inputs an agent may give: name, type, whether the action cannot run
/// without it, and its default. Values a preflight step fills are left out.
fn declared_inputs(op: &Operation) -> Vec<(String, String, bool, Option<Value>)> {
    let needed = check::needed_inputs(op);
    op.params
        .iter()
        .chain(op.query.iter())
        .chain(op.body.iter().flat_map(|body| body.inputs()))
        .filter(|(_, param)| !param.from_step())
        .map(|(name, param)| {
            (
                name.clone(),
                param.kind.clone(),
                needed.contains(name),
                param.default.clone(),
            )
        })
        .collect()
}

/// A command to copy: the action with a placeholder for each input it
/// cannot run without.
fn example_command(site: &str, op: &Operation) -> String {
    let mut command = format!("itda call {site}.{}", op.name);
    for (name, kind, required, _) in declared_inputs(op) {
        if required {
            command.push_str(&format!(" --{name} <{kind}>"));
        }
    }
    command.push_str(" --json");
    command
}

struct CallFlags {
    args: BTreeMap<String, String>,
    account: Option<String>,
    approval: Option<String>,
    next: Option<String>,
    as_json: bool,
}

/// `--name value` or `--name=value` inputs, plus `--account`, `--approval`,
/// `--next` and `--json`.
fn parse_call(rest: Vec<String>) -> AppResult<CallFlags> {
    let mut args = BTreeMap::new();
    let (mut account, mut approval, mut next, mut as_json) = (None, None, None, false);
    let mut words = rest.into_iter();
    while let Some(word) = words.next() {
        let Some(flag) = word.strip_prefix("--") else {
            return Err(usage(
                format!("Unexpected argument: {word}"),
                "Pass inputs as --name value.",
            ));
        };
        if flag == "json" {
            as_json = true;
            continue;
        }
        let (name, value) = match flag.split_once('=') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => {
                let value = words.next().ok_or_else(|| {
                    usage(
                        format!("--{flag} needs a value"),
                        "Pass inputs as --name value.",
                    )
                })?;
                (flag.to_owned(), value)
            }
        };
        match name.as_str() {
            "account" => account = Some(value),
            "approval" => approval = Some(value),
            "next" => next = Some(value),
            _ => {
                args.insert(name, value);
            }
        }
    }
    Ok(CallFlags {
        args,
        account,
        approval,
        next,
        as_json,
    })
}

async fn call(home: &Path, target: &str, rest: Vec<String>) -> AppResult<ExitCode> {
    let (site, action) = target.split_once('.').ok_or_else(|| {
        usage(
            format!("{target} is not <site>.<action>"),
            "Run `itda ls` to see installed actions.",
        )
    })?;
    let CallFlags {
        args,
        account,
        approval,
        next,
        as_json,
    } = parse_call(rest)?;
    let spec = sites::load(home, site)?;
    let envelope = open(home)?
        .runtime()
        .call(
            &spec,
            Call {
                action: action.to_owned(),
                args,
                account,
                approval,
                next,
                discovery: false,
            },
        )
        .await?;
    if as_json {
        let mut value = serde_json::to_value(&envelope)?;
        if let Value::Object(object) = &mut value {
            object.insert("status".into(), "ok".into());
            // Site content is data, never instructions for the agent.
            object.insert("untrusted".into(), true.into());
        }
        println!("{value}");
    } else {
        println!("{}", serde_json::to_string_pretty(&envelope.data)?);
        if let Some(handle) = &envelope.meta.next {
            eprintln!("{}", text::t("more_pages").replace("{handle}", handle));
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn check(
    home: &Path,
    only: Option<&str>,
    all: bool,
    account: Option<&str>,
    as_json: bool,
) -> AppResult<ExitCode> {
    let specs = match only {
        Some(site) => vec![sites::load(home, site)?],
        None => sites::list(home)
            .into_iter()
            .filter_map(|site| site.spec.ok())
            .collect(),
    };
    let outcomes = check::run(&open(home)?.runtime(), &specs, account).await;
    let verdicts = check::verdicts(&outcomes);
    check::remember(home, &verdicts)?;
    let status = check::exit_status(&outcomes);
    if as_json {
        println!(
            "{}",
            json!({
                "status": status,
                "verdict": check::Verdict::of(&outcomes),
                "sites": verdicts,
                "results": outcomes,
            })
        );
    } else if all {
        let width = verdicts.iter().map(|v| v.site.len()).max().unwrap_or(0);
        for site in &verdicts {
            println!(
                "{:<width$}  {}: {}",
                site.site,
                site.verdict.as_str(),
                site.hint
            );
        }
    } else {
        say_check(&outcomes, &verdicts);
    }
    Ok(ExitCode::from(output::exit_code(status)))
}

/// What `itda check` prints: one line per action, a verdict per site and a
/// count at the end.
pub(crate) fn say_check(outcomes: &[check::Outcome], verdicts: &[check::SiteVerdict]) {
    {
        let width = outcomes
            .iter()
            .map(|o| o.site.len() + o.action.len() + 1)
            .max()
            .unwrap_or(0);
        for site in verdicts {
            for o in outcomes.iter().filter(|o| o.site == site.site) {
                let time = o
                    .duration_ms
                    .map(|ms| format!(" {ms} ms"))
                    .unwrap_or_default();
                println!(
                    "{:<width$}  {:<18} {}{time}",
                    format!("{}.{}", o.site, o.action),
                    o.status,
                    o.detail
                );
            }
            println!("verdict  {}: {}", site.verdict.as_str(), site.hint);
        }
        let count = |status: &str| outcomes.iter().filter(|o| o.status == status).count();
        let failed = outcomes.iter().filter(|o| o.failed()).count();
        println!(
            "\n{} ok, {failed} failed, {} skipped.",
            count("ok"),
            count("skipped")
        );
    }
}

async fn login(
    home: &Path,
    site: &str,
    account: &str,
    user: Option<&str>,
    secret: Option<&str>,
    label: &str,
) -> AppResult<ExitCode> {
    let spec = sites::load(home, site)?;
    let form = spec
        .auth
        .iter()
        .find(|auth| auth.kind == "browser" && auth.login.is_some())
        .and_then(|auth| {
            auth.login
                .clone()
                .map(|login| (auth.value_from.clone(), login))
        });
    if let (None, Some((value_from, login))) = (secret, form) {
        return login_form(home, &spec, account, user, label, &value_from, &login).await;
    }
    let secret = secret.unwrap_or("api-key");
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprintln!("Paste the {secret} for {site}/{account} and press Enter:");
    }
    let mut value = String::new();
    stdin.lock().read_line(&mut value)?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err(usage(
            "No secret was given",
            "Pipe the secret on stdin, for example from your password manager.",
        ));
    }
    let parts = open(home)?;
    parts.vault.add_account(site, account, label).await?;
    parts
        .vault
        .set_secret(site, account, secret, Secret::new(value))
        .await?;
    // Approvals were bound to the credential the user saw; a new one needs
    // a new look.
    parts
        .approvals
        .invalidate(Some(site), Some(account))
        .await?;
    eprintln!("Saved {secret} for {site}/{account}.");
    Ok(ExitCode::SUCCESS)
}

/// Sign in through the site's own form: the user name from `--user` or a
/// question, the password from the next line of stdin, both sent once to
/// the form as itda. The session is kept in the system keyring.
async fn login_form(
    home: &Path,
    spec: &itda_spec::Spec,
    account: &str,
    user: Option<&str>,
    label: &str,
    value_from: &str,
    login: &itda_spec::LoginForm,
) -> AppResult<ExitCode> {
    use text::{t, tf};
    let site = &spec.site.name;
    itda_setup::credentials::writable()?;
    let credentials = setup::ask_credentials(site, user)?;
    let scope = itda_types::url::validate_url(spec.site.source_url())?;
    let page = format!("{}{}", scope.origin().ascii_serialization(), login.path);
    let ctx = open(home)?.context(home);
    let signed = itda_setup::login::form(
        &ctx.net,
        &scope,
        &page,
        Some((&login.user, &login.password)),
        &credentials,
        spec.defaults.interval(),
    )
    .await?;
    if let Some(required) = value_from.strip_prefix("cookie:")
        && !signed
            .session
            .cookies
            .split(';')
            .any(|pair| pair.trim().starts_with(&format!("{required}=")))
    {
        return Err(usage(
            tf("cookie_missing", &[("name", required), ("site", site)]),
            t("cookie_missing_fix"),
        ));
    }
    // A session the site carries in the query: the sign-in must have handed
    // out the key the definition names, in its redirect or on the first page.
    let query = value_from.strip_prefix("query:");
    if let Some(required) = query
        && signed
            .session
            .query
            .as_ref()
            .is_none_or(|(key, _)| key != required)
    {
        return Err(usage(
            tf("query_missing", &[("name", required), ("site", site)]),
            t("query_missing_fix"),
        ));
    }
    let count = signed.session.count;
    itda_setup::login::keep(&ctx, site, account, label, signed.session, value_from).await?;
    eprintln!(
        "{}",
        match query {
            Some(key) => tf(
                "session_saved_query",
                &[("site", site), ("account", account), ("name", key)]
            ),
            None => tf(
                "session_saved",
                &[
                    ("site", site),
                    ("account", account),
                    ("n", &count.to_string())
                ]
            ),
        }
    );
    Ok(ExitCode::SUCCESS)
}

async fn login_browser(
    home: &Path,
    site: &str,
    account: &str,
    url: Option<String>,
    label: &str,
) -> AppResult<ExitCode> {
    let spec = sites::load(home, site)?;
    // Check where the session will be kept before the person signs in.
    itda_setup::credentials::writable()?;
    let ctx = open(home)?.context(home);
    let session = tokio::task::spawn_blocking(move || {
        let done = setup::enter_pressed();
        itda_setup::login::browser(&spec, url.as_deref(), Some(&done), &mut say_login)
    })
    .await
    .map_err(|_| AppError::api("internal", 500))??;
    let count = session.count;
    itda_setup::login::keep(&ctx, site, account, label, session, "").await?;
    eprintln!(
        "Saved the session for {site}/{account} ({count} cookies). It is sealed under the data directory with a key in the system keyring. Agents only see the account id."
    );
    Ok(ExitCode::SUCCESS)
}

fn say_login(progress: itda_setup::login::Progress) {
    match progress {
        itda_setup::login::Progress::Opened { site } => {
            eprintln!("Sign in to {site} in the browser window that just opened.");
        }
        itda_setup::login::Progress::WaitsFor { cookie: Some(name) } => eprintln!(
            "itda continues when the {name} cookie appears, or press Enter once you are signed in."
        ),
        itda_setup::login::Progress::WaitsFor { cookie: None } => {
            eprintln!("Press Enter here once you are signed in.");
        }
    }
}

async fn resume(home: &Path, site: &str, account: Option<&str>) -> AppResult<ExitCode> {
    itda_setup::resume(&open(home)?.context(home), site, account).await?;
    eprintln!("Resumed {site}.");
    Ok(ExitCode::SUCCESS)
}

async fn add(home: &Path, source: &str, force: bool) -> AppResult<ExitCode> {
    const LIMIT: usize = 4 * 1024 * 1024;
    let bytes = if source == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)?;
        bytes
    } else if source.starts_with("https://") || source.starts_with("http://") {
        let request = itda_net::Request {
            limit: LIMIT,
            ..itda_net::Request::read(source)
        };
        let response = open(home)?.net.fetch(request).await?;
        if !response.successful() {
            return Err(AppError::new(
                itda_types::error::Code::Runtime,
                format!("{source} answered HTTP {}", response.status),
                "Check the address points at the raw definition file.",
            ));
        }
        response.bytes
    } else if !Path::new(source).exists() && !source.contains(['/', '\\', '.']) {
        let Some(text) = sites::bundled(source) else {
            let names: Vec<_> = sites::BUNDLED.iter().map(|(name, _)| *name).collect();
            return Err(usage(
                format!("{source} is not a file or a ready-made site"),
                format!("Ready-made sites: {}.", names.join(", ")),
            ));
        };
        text.as_bytes().to_vec()
    } else {
        itda_types::util::read_bounded(Path::new(source), LIMIT)?
    };
    let (spec, path) = sites::install(home, &bytes, force)?;
    if force {
        approvals_store(home)?
            .invalidate(Some(&spec.site.name), None)
            .await?;
    }
    eprintln!("Installed {} at {}.", spec.site.name, path.display());
    for line in sites::review(&spec) {
        eprintln!("  {line}");
    }
    eprintln!("Try `itda ls {}`.", spec.site.name);
    Ok(ExitCode::SUCCESS)
}

fn remove(home: &Path, site: &str) -> AppResult<ExitCode> {
    sites::remove(home, site)?;
    block_on(async {
        approvals_store(home)?.invalidate(Some(site), None).await?;
        Ok(ExitCode::SUCCESS)
    })?;
    eprintln!("Removed {site}. Accounts signed in to it stay until `itda logout {site}`.");
    Ok(ExitCode::SUCCESS)
}

type Drafter = fn(&[u8], &itda_prep::Options) -> AppResult<itda_prep::Draft>;

/// A saved recording holds the pages the person saw as well as the calls
/// those pages made; both become actions. A GraphQL endpoint the pages
/// called is asked once what else it offers.
async fn draft_har(
    home: &Path,
    file: &Path,
    mut options: itda_prep::Options,
    install: bool,
    force: bool,
) -> AppResult<ExitCode> {
    let text = std::fs::read(file)?;
    let endpoints = itda_prep::graphql::endpoints_in_recording(&text);
    if !endpoints.is_empty() {
        options.schemas = itda_setup::introspect::schemas(&open(home)?.net, &endpoints).await;
        say_schemas(&options.schemas);
    }
    let draft = itda_prep::from_recording(&text, &options, "`itda draft har`")?;
    let sources = itda_setup::refresh::Sources::of_pages(&[], &draft, &options);
    publish(
        home,
        draft,
        install,
        force,
        Origin {
            sources,
            recording: Some(text),
        },
    )
}

/// The draft goes to stdout (or the data directory); what to review goes to
/// stderr, so `itda draft openapi api.json > spec.yaml` keeps them apart.
fn draft(
    home: &Path,
    from: Drafter,
    file: &Path,
    options: itda_prep::Options,
    install: bool,
    force: bool,
) -> AppResult<ExitCode> {
    let text = std::fs::read(file)?;
    let draft = from(&text, &options)?;
    let sources = itda_setup::refresh::Sources {
        description: Some(file.display().to_string()),
        ..itda_setup::refresh::Sources::of_pages(&[], &draft, &options)
    };
    publish(
        home,
        draft,
        install,
        force,
        Origin {
            sources,
            recording: None,
        },
    )
}

/// What the gate already knows about a page that refused itda, as the error
/// `itda call` would give (`blocked_by_site`, `site_rate_limited`) with the
/// block named in the person's language. `None` when the answer was not a
/// block.
fn site_refusal(url: &str, response: &itda_net::Response) -> Option<AppError> {
    itda_setup::refusal(url, response).map(|refused| refusal_words(&refused))
}

/// The refusal in the person's language: Cloudflare, a bot manager, a
/// captcha, a rate limit with the wait it asked for, or a sign-in redirect.
fn refusal_words(refused: &itda_setup::Refused) -> AppError {
    use itda_setup::Refusal;
    use text::{t, tf};
    let host = refused.host.clone();
    let (msg, fix) = match &refused.kind {
        Refusal::SignInRedirect { path, .. } => (
            tf("redirect_signin", &[("host", &host), ("path", path)]),
            t("redirect_signin_fix"),
        ),
        Refusal::Cloudflare => (
            tf("blocked_cloudflare", &[("host", &host)]),
            t("blocked_fix"),
        ),
        Refusal::BotManager => (
            tf("blocked_bot_manager", &[("host", &host)]),
            t("blocked_fix"),
        ),
        Refusal::Captcha => (tf("blocked_captcha", &[("host", &host)]), t("blocked_fix")),
        Refusal::RateLimited { wait: Some(wait) } => (
            tf(
                "rate_limited_wait",
                &[("host", &host), ("n", &wait.to_string())],
            ),
            t("rate_limited_fix"),
        ),
        Refusal::RateLimited { wait: None } => (
            tf("rate_limited", &[("host", &host)]),
            t("rate_limited_fix"),
        ),
        Refusal::Other { status } => (
            tf(
                "blocked_other",
                &[("host", &host), ("status", &status.to_string())],
            ),
            t("blocked_fix"),
        ),
    };
    let mut error = AppError::api(&refused.error.public_code, refused.error.http_status);
    error.extra = refused.error.extra.clone();
    error.msg = msg;
    error.remedy = fix.to_owned();
    error
}

/// Fetch each page with the honest client, then draft from what they show.
/// A page that refuses or redirects to a sign-in is reported, never worked
/// around; `itda record` covers sites that need an account.
async fn draft_page(
    home: &Path,
    urls: &[String],
    mut options: itda_prep::Options,
    install: bool,
    force: bool,
) -> AppResult<ExitCode> {
    const LIMIT: usize = 8 * 1024 * 1024;
    let net = open(home)?.net;
    let mut pages = Vec::new();
    for url in urls {
        let request = itda_net::Request {
            limit: LIMIT,
            ..itda_net::Request::read(url.as_str())
        };
        let response = net.fetch(request).await?;
        if let Some(error) = site_refusal(url, &response) {
            return Err(error);
        }
        if !response.successful() {
            return Err(AppError::new(
                itda_types::error::Code::Runtime,
                format!("{url} answered HTTP {}", response.status),
                "Open the address in a browser to check it. Pages behind a sign-in: record yourself using them with `itda record`.",
            ));
        }
        let content_type = response.header("content-type").to_owned();
        if !content_type.contains("html") {
            return Err(usage(
                format!("{url} is not an HTML page ({content_type})"),
                "Pass the address of a page. For an API description, use `itda draft openapi`.",
            ));
        }
        eprintln!(
            "Fetched {} ({} KB).",
            response.url,
            response.bytes.len().div_ceil(1024)
        );
        let page = itda_prep::Page {
            url: response.url.clone(),
            html: String::from_utf8_lossy(&response.bytes).into_owned(),
        };
        // A shell whose content is in frames on its own origin is read
        // through them, each frame page under its own address.
        let mut framed = itda_setup::frames::follow(&net, &page, None).await?;
        if let Some(line) = text::followed_frames(&framed.followed) {
            eprintln!("{line}");
        }
        // A frame that answers with a sign-in is noted as leading to one,
        // not only as unread; `itda setup` can sign in and read it.
        options.frames_to_sign_in.extend(framed.sign_in_paths());
        pages.push(page);
        pages.append(&mut framed.pages);
    }
    let endpoints = itda_setup::introspect::endpoints_in_pages(&pages);
    if !endpoints.is_empty() {
        options.schemas = itda_setup::introspect::schemas(&net, &endpoints).await;
        say_schemas(&options.schemas);
    }
    let description = itda_setup::description::find(&net, &pages).await;
    let draft = itda_prep::from_pages_and_description(&pages, description.as_ref(), &options)?;
    if let Some(described) = &draft.described {
        eprintln!("{}", text::described(described));
    }
    let sources = itda_setup::refresh::Sources::of_pages(urls, &draft, &options);
    publish(
        home,
        draft,
        install,
        force,
        Origin {
            sources,
            recording: None,
        },
    )
}

/// Record the site in the person's own browser and draft from the recording,
/// which stays in memory unless `save` names a file for it. With `--install`,
/// the token or session the recording used can go straight to the keyring,
/// each after a yes in the terminal.
async fn record(
    home: &Path,
    url: &str,
    mut options: itda_prep::Options,
    install: bool,
    force: bool,
    save: Option<&Path>,
) -> AppResult<ExitCode> {
    let url = url.to_owned();
    let recording = tokio::task::spawn_blocking(move || {
        let done = setup::enter_pressed();
        itda_setup::record::browser(&url, Some(&done), &mut |progress| say_recording(&progress))
    })
    .await
    .map_err(|_| AppError::api("internal", 500))??;
    let har = serde_json::to_vec(&recording.har)?;
    if let Some(path) = save {
        itda_types::util::atomic_file(path, &har)?;
        eprintln!(
            "Saved the recording at {}. It holds your cookies and tokens; delete it when you are done.",
            path.display()
        );
    }
    let endpoints = itda_prep::graphql::endpoints_in_recording(&har);
    if !endpoints.is_empty() {
        options.schemas = itda_setup::introspect::schemas(&open(home)?.net, &endpoints).await;
        say_schemas(&options.schemas);
    }
    let draft = itda_prep::from_recording(&har, &options, "`itda record`")?;
    let site = draft.site.clone();
    // Without --save the recording is gone once this returns, so a refresh
    // reads the pages it held again; with it, the copy under the site is
    // what a refresh drafts from.
    let base_url = itda_spec::parse(draft.yaml.as_bytes())
        .map(|spec| spec.site.base_url)
        .unwrap_or_default();
    let pages = itda_prep::pages_in_recording(&har, &base_url);
    let sources = itda_setup::refresh::Sources::of_pages(&pages, &draft, &options);
    let origin = Origin {
        sources,
        recording: save.map(|_| har.clone()),
    };
    let code = publish(home, draft, install, force, origin)?;
    if !install {
        return Ok(code);
    }
    let spec = sites::load(home, &site)?;
    let kept = keep_credentials(home, &spec, &recording.har, &recording.cookies).await?;
    if !kept {
        eprintln!(
            "Sign in once with `itda login {site} --browser` if its actions need an account."
        );
    }
    Ok(code)
}

/// Offer to keep what the recording signed in with: a header token for each
/// header `auth` entry, and the browser session for a `browser` one. Values
/// are never shown; nothing is kept without a yes in a terminal.
async fn keep_credentials(
    home: &Path,
    spec: &itda_spec::Spec,
    har: &Value,
    cookies: &[itda_browser::Cookie],
) -> AppResult<bool> {
    if !std::io::stdin().is_terminal() || itda_setup::credentials::store_is_env() {
        return Ok(false);
    }
    let offers = itda_setup::credentials::offers(spec, har, cookies);
    if offers.is_empty() {
        return Ok(false);
    }
    KeyringStore::available()?;
    let site = &spec.site.name;
    let account = itda_runtime::DEFAULT_ACCOUNT;
    let ctx = open(home)?.context(home);
    let mut kept = false;
    for offer in offers {
        if !setup::ask_keep(site, &offer.what)? {
            continue;
        }
        itda_setup::credentials::keep(&ctx, site, account, &offer.name, offer.secret).await?;
        say_credential_kept(&offer.name, site, account);
        kept = true;
    }
    Ok(kept)
}

fn say_credential_kept(name: &str, site: &str, account: &str) {
    if name == itda_runtime::SESSION_SECRET {
        eprintln!(
            "{}",
            text::tf("session_kept", &[("site", site), ("account", account)])
        );
        return;
    }
    eprintln!(
        "Saved {name} for {site}/{account}. Tokens expire; record again or use `itda login {site}` when calls answer auth_required."
    );
}

fn say_recording(progress: &itda_setup::record::Progress) {
    match progress {
        itda_setup::record::Progress::Opened { host } => {
            eprintln!(
                "Recording {host}. Use the site in the window that just opened: sign in, search, open things, post a form."
            );
            eprintln!("Press Enter here, or close the window, when you are done.");
        }
        itda_setup::record::Progress::Calls(n) => eprintln!("  {n} calls so far"),
        itda_setup::record::Progress::Finished(n) => eprintln!("Recorded {n} calls."),
    }
}

/// One line per schema that was read, in the person's language.
fn say_schemas(schemas: &[itda_prep::graphql::Schema]) {
    for schema in schemas {
        eprintln!("{}", text::tf("asked_schema", &[("url", &schema.url)]));
    }
}

/// Print the draft, or install it, and list what to review on stderr.
/// Where a draft came from, kept next to it when it is installed so `itda
/// refresh` can draft it again; the recording, when the person kept one,
/// goes under the site.
struct Origin {
    sources: itda_setup::refresh::Sources,
    recording: Option<Vec<u8>>,
}

fn publish(
    home: &Path,
    draft: itda_prep::Draft,
    install: bool,
    force: bool,
    origin: Origin,
) -> AppResult<ExitCode> {
    for note in &draft.notes {
        eprintln!("note: {note}");
    }
    for problem in &draft.problems {
        eprintln!("problem: {problem}");
    }
    eprintln!(
        "Drafted {}: {} actions ({} reads, {} changes).",
        draft.site,
        draft.operations,
        draft.reads,
        draft.operations - draft.reads
    );
    if !install {
        print!("{}", draft.yaml);
        return Ok(ExitCode::SUCCESS);
    }
    let path = sites::specs_dir(home).join(&draft.site).join("spec.yaml");
    if path.exists() && !force {
        return Err(usage(
            format!("{} is already installed", draft.site),
            "Pass --force to replace it, or --name to install under another name.",
        ));
    }
    let replaced = path.exists();
    itda_types::util::atomic_file(&path, draft.yaml.as_bytes())?;
    let Origin {
        mut sources,
        recording,
    } = origin;
    if let Some(har) = recording {
        sources.recording = Some(itda_setup::refresh::keep_recording(
            home,
            &draft.site,
            &har,
        )?);
    }
    itda_setup::refresh::remember(home, &draft.site, sources)?;
    if replaced {
        let site = draft.site.clone();
        block_on(async {
            approvals_store(home)?.invalidate(Some(&site), None).await?;
            Ok(ExitCode::SUCCESS)
        })?;
    }
    eprintln!(
        "Installed at {}. Review it, then try `itda ls {}`.",
        path.display(),
        draft.site
    );
    Ok(ExitCode::SUCCESS)
}

fn approvals_store(home: &Path) -> AppResult<Approvals> {
    Approvals::open(Store::open(&home.join("itda.db"))?)
}

async fn accounts(home: &Path, site: Option<&str>, as_json: bool) -> AppResult<ExitCode> {
    let accounts = open(home)?.vault.accounts(site).await?;
    if as_json {
        println!("{}", serde_json::to_value(&accounts)?);
        return Ok(ExitCode::SUCCESS);
    }
    if accounts.is_empty() {
        println!("No accounts are signed in. Add one with `itda login <site>`.");
    }
    for account in &accounts {
        let label = if account.label.is_empty() {
            String::new()
        } else {
            format!(" ({})", account.label)
        };
        println!(
            "{}/{}{label}  {}",
            account.site,
            account.id,
            account.secrets.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn logout(home: &Path, site: &str, account: &str) -> AppResult<ExitCode> {
    let vault = open(home)?.vault;
    if !vault
        .accounts(Some(site))
        .await?
        .iter()
        .any(|a| a.id == account)
    {
        return Err(usage(
            format!("{site}/{account} is not signed in"),
            "Run `itda accounts` to see signed-in accounts.",
        ));
    }
    vault.remove_account(site, account).await?;
    approvals_store(home)?
        .invalidate(Some(site), Some(account))
        .await?;
    eprintln!("Removed {site}/{account} and its saved secrets.");
    Ok(ExitCode::SUCCESS)
}

async fn approvals(home: &Path, as_json: bool) -> AppResult<ExitCode> {
    let pending = approvals_store(home)?.pending().await?;
    if as_json {
        println!("{}", serde_json::to_value(&pending)?);
        return Ok(ExitCode::SUCCESS);
    }
    if pending.is_empty() {
        println!("No changes are waiting for approval.");
    }
    for approval in &pending {
        println!(
            "{}  {}.{}  account {}  until {}",
            approval.id,
            approval.site_id,
            approval.action_id,
            approval.account_id,
            approval.expires_at
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Show exactly what will be sent and ask in the terminal. An agent working
/// through pipes cannot answer, which keeps the decision with the person.
async fn approve(home: &Path, id: &str) -> AppResult<ExitCode> {
    let store = approvals_store(home)?;
    let review = store.review(id).await?;
    println!(
        "{} wants to change {} ({}) as account {}:",
        review.action_title, review.site_title, review.site_id, review.account_id
    );
    for input in &review.inputs {
        println!("  {:<12} {}", input.label, input.value);
    }
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        return Err(usage(
            "Approving needs a terminal",
            "Run itda approve yourself in a terminal; agents cannot approve changes.",
        ));
    }
    eprint!("Approve this change? [y/N] ");
    let mut answer = String::new();
    stdin.lock().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        store.reject(id).await?;
        record_decision(home, &review, "rejected")?;
        println!("Rejected.");
        return Ok(ExitCode::SUCCESS);
    }
    store.claim(id).await?;
    record_decision(home, &review, "approved")?;
    println!("Approved. The agent can now run it once, with exactly these inputs.");
    Ok(ExitCode::SUCCESS)
}

async fn reject(home: &Path, id: &str) -> AppResult<ExitCode> {
    let store = approvals_store(home)?;
    let review = store.review(id).await?;
    store.reject(id).await?;
    record_decision(home, &review, "rejected")?;
    println!("Rejected.");
    Ok(ExitCode::SUCCESS)
}

fn record_decision(
    home: &Path,
    review: &itda_policy::Approval,
    kind: &'static str,
) -> AppResult<()> {
    audit_log(home).append(Event {
        kind,
        site: review.site_id.clone(),
        action: review.action_id.clone(),
        account: review.account_id.clone(),
        approval: review.id.clone(),
        inputs: review
            .request
            .as_ref()
            .map(|r| r.inputs.clone())
            .unwrap_or_default(),
        outcome: "ok".into(),
        ..Event::default()
    })?;
    Ok(())
}

fn audit(home: &Path, limit: usize, verify: bool, as_json: bool) -> AppResult<ExitCode> {
    let log = audit_log(home);
    if verify {
        let count = log.verify()?;
        if as_json {
            println!("{}", json!({"status": "ok", "records": count}));
        } else {
            println!(
                "The audit log is intact: {count} records in {}.",
                log.path().display()
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    let records = log.records()?;
    let recent = &records[records.len().saturating_sub(limit)..];
    if as_json {
        println!("{}", serde_json::to_value(recent)?);
        return Ok(ExitCode::SUCCESS);
    }
    if recent.is_empty() {
        println!("Nothing recorded yet.");
    }
    for r in recent {
        println!(
            "{:>5}  {}  {:<15} {}.{}  {}{}",
            r.seq,
            r.at,
            r.kind,
            r.site,
            r.action,
            r.outcome,
            if r.approval.is_empty() {
                String::new()
            } else {
                format!("  approval {}", r.approval)
            }
        );
    }
    let unfinished = log.unfinished_changes()?;
    if !unfinished.is_empty() {
        println!("\n{}", unfinished_words(&unfinished));
    }
    Ok(ExitCode::SUCCESS)
}

/// Changes that were about to be sent when the process stopped: nobody
/// knows whether the site applied them.
fn unfinished_words(unfinished: &[itda_runtime::audit::Record]) -> String {
    let list: Vec<String> = unfinished
        .iter()
        .map(|r| {
            format!(
                "{}.{} (record {}, approval {})",
                r.site, r.action, r.seq, r.approval
            )
        })
        .collect();
    format!(
        "{} change{} with an unknown outcome: the process stopped after the change was sent. Check the site before asking for {} again: {}",
        unfinished.len(),
        if unfinished.len() == 1 { "" } else { "s" },
        if unfinished.len() == 1 { "it" } else { "them" },
        list.join(", ")
    )
}

/// Serve MCP on stdin and stdout. Stdout belongs to the protocol, so nothing
/// else is printed there; invalid sites are reported on stderr and skipped.
async fn mcp(
    home: &Path,
    mode: Option<&str>,
    approve_in_client: bool,
    only: Option<&str>,
) -> AppResult<ExitCode> {
    if let Some(site) = only {
        sites::load(home, site)?;
    }
    let mut specs = Vec::new();
    for site in sites::list(home) {
        if only.is_some_and(|only| only != site.name) {
            continue;
        }
        match site.spec {
            Ok(spec) => specs.push(spec),
            Err(error) => eprintln!("itda: skipping {}: {}", site.name, error.msg),
        }
    }
    let mode = match mode {
        Some("direct") => Some(itda_mcp::Mode::Direct),
        Some("catalog") => Some(itda_mcp::Mode::Catalog),
        _ => None,
    };
    let parts = open(home)?;
    let (approvals, audit) = (parts.approvals.clone(), parts.audit.clone());
    let ctx = Arc::new(parts.context(home));
    let mut server =
        itda_mcp::Server::new(ctx.runtime.clone(), itda_mcp::Catalog::new(specs), mode)
            .with_setup(ctx);
    if approve_in_client {
        server = server.with_client_approval(approvals, audit);
    }
    if let Some(site) = only {
        server = server.with_site(site);
    }
    server
        .serve_stdio()
        .await
        .map_err(|e| usage(format!("MCP session ended: {e}"), "Restart the MCP client."))?;
    Ok(ExitCode::SUCCESS)
}

fn mcp_install(client: &str, dry_run: bool, approve_in_client: bool) -> AppResult<ExitCode> {
    let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| usage("itda found no home directory", "Set HOME and try again."))?;
    let exe = std::env::current_exe()?;
    // Clients start itda without this shell's environment, so the settings
    // that change where data lives or what may be reached are passed along.
    let mut env: Vec<(String, String)> = [
        "ITDA_HOME",
        "ITDA_ALLOW_LOCAL",
        "ITDA_SECRET_STORE",
        "ITDA_APPROVE_IN_CLIENT",
    ]
    .into_iter()
    .filter_map(|name| Some((name.to_owned(), std::env::var(name).ok()?)))
    .collect();
    if approve_in_client {
        env.retain(|(name, _)| name != "ITDA_APPROVE_IN_CLIENT");
        env.push(("ITDA_APPROVE_IN_CLIENT".into(), "1".into()));
    }
    let launch = install::Launch {
        command: exe.display().to_string(),
        env,
    };
    match install::plan(client, &user_home, &std::env::current_dir()?, &launch)? {
        install::Plan::Run(command) => {
            let label = text::client_label(client);
            if !dry_run && std::io::stdin().is_terminal() && client_binary(client) {
                println!("{}", text::tf("running", &[("command", &command)]));
                let status = shell(&command)?;
                if !status.success() {
                    return Err(usage(
                        format!("{label} did not accept the command"),
                        format!("Run it yourself:\n\n  {command}"),
                    ));
                }
            } else {
                println!(
                    "{}\n\n  {command}",
                    text::tf("run_yourself", &[("client", label)])
                );
            }
        }
        install::Plan::Write { path, content } if dry_run => {
            println!("Would write {}:\n\n{content}", path.display());
        }
        install::Plan::Write { path, content } => {
            let shown = tilde(&path, &user_home);
            let fields = [
                ("client", text::client_label(client)),
                ("path", shown.as_str()),
            ];
            let key = if install::apply(&path, &content)? {
                "added_client"
            } else {
                "already_client"
            };
            println!("{}", text::tf(key, &fields));
            let handled = client == "claude-desktop" && relaunch_claude_desktop()?;
            if !handled {
                println!("{}", text::tf("restart_hint", &[("client", fields[0].1)]));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// On macOS, where the app can be told to quit and reopened: restart Claude
/// Desktop after a yes when it is open, or say that opening it is enough.
/// `false` when nothing was said, so the caller gives the restart hint.
fn relaunch_claude_desktop() -> AppResult<bool> {
    if !cfg!(target_os = "macos") || !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    let label = text::client_label("claude-desktop");
    let open = std::process::Command::new("pgrep")
        .args(["-x", "Claude"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !open {
        println!("{}", text::tf("open_app", &[("client", label)]));
        return Ok(true);
    }
    if !setup::ask(&text::tf("restart_ask", &[("client", label)]), true)? {
        return Ok(false);
    }
    std::process::Command::new("osascript")
        .args(["-e", "tell application \"Claude\" to quit"])
        .status()?;
    for _ in 0..50 {
        let still = std::process::Command::new("pgrep")
            .args(["-x", "Claude"])
            .output()
            .is_ok_and(|o| o.status.success());
        if !still {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    std::process::Command::new("open")
        .args(["-a", "Claude"])
        .status()?;
    println!("{}", text::tf("restarted", &[("client", label)]));
    Ok(true)
}

/// Whether the client's own command-line tool is on the PATH.
fn client_binary(client: &str) -> bool {
    let name = match client {
        "claude-code" => "claude",
        _ => return false,
    };
    let exe = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(exe)
        .arg(name)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn shell(command: &str) -> AppResult<std::process::ExitStatus> {
    let status = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", command])
            .status()?
    } else {
        std::process::Command::new("sh")
            .args(["-c", command])
            .status()?
    };
    Ok(status)
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn doctor(home: &Path) -> ExitCode {
    println!("itda {}", env!("CARGO_PKG_VERSION"));
    let checks = doctor_checks(home);
    for (ok, what, detail) in &checks {
        println!("{}  {what:<16} {detail}", if *ok { "ok  " } else { "FAIL" });
    }
    if checks.iter().all(|(ok, _, _)| *ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The issue link is printed first, so it is there even when nothing opens:
/// without a terminal, or with --no-open.
fn feedback(home: &Path, no_open: bool) -> ExitCode {
    let url = feedback::issue_url(
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        &doctor_checks(home),
    );
    println!("{url}");
    if !no_open && std::io::stdin().is_terminal() {
        feedback::open(&url);
    }
    ExitCode::SUCCESS
}

/// Each doctor check: whether it passed, what it checked, and the detail
/// people read. The detail can hold a path.
fn user_home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn doctor_checks(home: &Path) -> Vec<(bool, &'static str, String)> {
    let mut checks = Vec::new();
    let mut report = |ok: bool, what: &'static str, detail: String| {
        checks.push((ok, what, detail));
    };
    match Store::open(&home.join("itda.db")) {
        Ok(_) => report(true, "data directory", home.display().to_string()),
        Err(error) => report(false, "data directory", error.to_string()),
    }
    let keyring = KeyringStore::available().map_err(|e: AppError| e.msg);
    match (keyring, std::env::var("ITDA_SECRET_STORE").as_deref()) {
        (_, Ok("env")) => report(
            true,
            "secrets",
            "ITDA_SECRET__ environment variables".into(),
        ),
        (Ok(()), _) => report(
            true,
            "secrets",
            format!(
                "system keyring; long secrets sealed in {} with a key from the keyring",
                tilde(&home.join(itda_vault::SEALED_FILE), &user_home_dir())
            ),
        ),
        (Err(msg), _) => report(
            false,
            "secrets",
            format!("{msg}; set ITDA_SECRET_STORE=env for headless runs"),
        ),
    }
    match audit_log(home).verify() {
        Ok(count) => report(true, "audit log", format!("{count} records, chain intact")),
        Err(error) => report(false, "audit log", error.msg),
    }
    match audit_log(home).unfinished_changes() {
        Ok(unfinished) if unfinished.is_empty() => {
            report(true, "changes", "none with an unknown outcome".into());
        }
        Ok(unfinished) => report(true, "changes", unfinished_words(&unfinished)),
        Err(error) => report(false, "changes", error.msg),
    }
    let installed = sites::list(home);
    let invalid = installed.iter().filter(|s| s.spec.is_err()).count();
    report(
        invalid == 0,
        "sites",
        format!("{} installed, {invalid} invalid", installed.len()),
    );
    let user_home = user_home_dir();
    let cwd = std::env::current_dir().unwrap_or_default();
    let agents = install::connected(&user_home, &cwd);
    report(
        true,
        "agents",
        if agents.is_empty() {
            "none connected (itda setup <url>, or itda mcp install <client>)".into()
        } else {
            agents
                .iter()
                .map(|c| text::client_label(c))
                .collect::<Vec<_>>()
                .join(", ")
        },
    );
    report(
        true,
        "local addresses",
        if allow_local() {
            "allowed"
        } else {
            "refused (ITDA_ALLOW_LOCAL=1 allows them)"
        }
        .into(),
    );
    checks
}

#[cfg(test)]
mod tests {
    use super::{Command, First, Shortcut, first_command, shortcut, tilde};
    use std::path::Path;

    fn argv(parts: &[&str]) -> Vec<std::ffi::OsString> {
        parts.iter().map(|p| (*p).into()).collect()
    }

    #[test]
    fn an_address_after_itda_runs_setup_on_it() {
        match shortcut(argv(&["itda", "news.ycombinator.com", "--yes"])) {
            Shortcut::Args(args) => assert_eq!(
                args,
                argv(&["itda", "setup", "https://news.ycombinator.com", "--yes"])
            ),
            Shortcut::Bundled(_) => panic!("an address is not a ready-made site"),
        }
        match shortcut(argv(&["itda", "http://intranet.local/board"])) {
            Shortcut::Args(args) => assert_eq!(args[2], "http://intranet.local/board"),
            Shortcut::Bundled(_) => panic!(),
        }
        for same in [
            &["itda", "ls"][..],
            &["itda", "--help"],
            &["itda"],
            &["itda", "setup", "x.y"],
        ] {
            match shortcut(argv(same)) {
                Shortcut::Args(args) => assert_eq!(args, argv(same)),
                Shortcut::Bundled(_) => panic!("{same:?}"),
            }
        }
        assert!(
            matches!(shortcut(argv(&["itda", "github"])), Shortcut::Bundled(n) if n == "github")
        );
    }

    #[test]
    fn the_first_prompt_takes_an_address_or_a_ready_made_site() {
        assert!(first_command("  \n").is_none());
        assert!(matches!(first_command("github\n"), Some(First::Bundled(n)) if n == "github"));
        match first_command("news.ycombinator.com\n") {
            Some(First::Run(Command::Setup { url, .. })) => {
                assert_eq!(url, "https://news.ycombinator.com");
            }
            _ => panic!("an address starts setup"),
        }
        match first_command("http://intranet.local/board") {
            Some(First::Run(Command::Setup { url, .. })) => {
                assert_eq!(url, "http://intranet.local/board");
            }
            _ => panic!("a full address is kept"),
        }
    }

    #[test]
    fn shortens_the_home_directory_to_a_tilde() {
        let home = Path::new("/Users/someone");
        assert_eq!(
            tilde(&home.join("Library/Claude/config.json"), home),
            "~/Library/Claude/config.json"
        );
        assert_eq!(tilde(Path::new("/etc/itda.json"), home), "/etc/itda.json");
    }
}
