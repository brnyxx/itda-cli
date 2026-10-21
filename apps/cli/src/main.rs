//! The `itda` command. This is the composition root: it picks the data
//! directory, the secret store and the network settings, and hands them to
//! the runtime. Commands that need no network never start an async runtime.
mod check;
mod feedback;
mod install;
mod login;
mod output;
mod record;
mod setup;
mod sites;
mod text;

use clap::{CommandFactory, Parser, Subcommand};
use itda_net::{Gate, Network};
use itda_policy::{Approvals, operation_effect};
use itda_runtime::{
    Call, Runtime,
    audit::{AuditLog, Event},
};
use itda_store::Store;
use itda_types::error::{AppError, AppResult, usage};
use itda_vault::{EnvStore, KeyringStore, Secret, SecretStore, Vault};
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
    /// Sign a site account in: store an API key read from stdin, or with
    /// --browser sign in in your own browser and keep that site's session
    Login {
        site: String,
        #[arg(long, default_value = itda_runtime::DEFAULT_ACCOUNT)]
        account: String,
        /// Sign in in a browser window instead of pasting a secret
        #[arg(long)]
        browser: bool,
        /// The page to open for --browser; defaults to the site's address
        #[arg(long, requires = "browser")]
        url: Option<String>,
        /// The secret name the site definition refers to (store:<site>/<name>)
        #[arg(long, default_value = "api-key")]
        secret: String,
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
        /// The agent to connect: claude-desktop, claude-code, cursor, vscode,
        /// codex or none
        #[arg(long)]
        client: Option<String>,
        /// Answer yes to every question, including replacing an installed site
        #[arg(long, short = 'y')]
        yes: bool,
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
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        json: bool,
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
    let cli = Cli::parse();
    let command = match cli.command.map(First::Run).or_else(welcome) {
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
        || matches!(&command, Command::Approvals { json: true })
        || matches!(&command, Command::Accounts { json: true, .. })
        || matches!(&command, Command::Audit { json: true, .. })
        || matches!(&command, Command::Check { json: true, .. })
        || matches!(&command, Command::Call { rest, .. } if rest.iter().any(|a| a == "--json"));
    let result = home().and_then(|home| match command {
        Command::Ls { site, json } => ls(&home, site.as_deref(), json),
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
            secret,
            label,
            ..
        } => block_on(login(&home, &site, &account, &secret, &label)),
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
            } => draft(
                &home,
                draft_har,
                &file,
                itda_prep::Options {
                    name,
                    base_url,
                    include,
                    lang: lang(),
                },
                install,
                force,
            ),
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
            client,
            yes,
        } => block_on(setup::run(
            &home,
            setup::Flags {
                url,
                name,
                record: if record {
                    Some(true)
                } else if no_record {
                    Some(false)
                } else {
                    None
                },
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
            account,
            json,
        } => block_on(check(&home, site.as_deref(), account.as_deref(), json)),
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
        } => block_on(mcp(
            &home,
            mode.as_deref(),
            approve_in_client || std::env::var("ITDA_APPROVE_IN_CLIENT").is_ok_and(|v| v == "1"),
        )),
    });
    match result {
        Ok(code) => code,
        Err(error) => output::fail(&error, as_json),
    }
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
        client: None,
        yes: false,
    }))
}

fn home() -> AppResult<PathBuf> {
    itda_types::paths::home().ok_or_else(|| {
        usage(
            "No data directory could be found",
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

/// `ITDA_SECRET_STORE=env` reads `ITDA_SECRET__` variables instead of the keyring.
fn secret_store() -> Arc<dyn SecretStore> {
    if std::env::var("ITDA_SECRET_STORE").is_ok_and(|v| v == "env") {
        Arc::new(EnvStore::from_env())
    } else {
        Arc::new(KeyringStore::new())
    }
}

struct Parts {
    net: Arc<Network>,
    vault: Arc<Vault>,
    approvals: Arc<Approvals>,
    audit: Arc<AuditLog>,
}

impl Parts {
    /// Reads run directly; changes wait for the user's approval. Everything
    /// is recorded in the audit log.
    fn runtime(self) -> Runtime {
        Runtime::new(self.net, self.vault)
            .with_approvals(self.approvals)
            .with_audit(self.audit)
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
        audit: Arc::new(audit_log(home)),
        vault: Arc::new(Vault::open(store, secret_store())?),
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

struct CallFlags {
    args: BTreeMap<String, String>,
    account: Option<String>,
    approval: Option<String>,
    as_json: bool,
}

/// `--name value` or `--name=value` inputs, plus `--account`, `--approval`
/// and `--json`.
fn parse_call(rest: Vec<String>) -> AppResult<CallFlags> {
    let mut args = BTreeMap::new();
    let (mut account, mut approval, mut as_json) = (None, None, false);
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
            _ => {
                args.insert(name, value);
            }
        }
    }
    Ok(CallFlags {
        args,
        account,
        approval,
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
    }
    Ok(ExitCode::SUCCESS)
}

async fn check(
    home: &Path,
    only: Option<&str>,
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
    } else {
        let width = outcomes
            .iter()
            .map(|o| o.site.len() + o.action.len() + 1)
            .max()
            .unwrap_or(0);
        for site in &verdicts {
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
    Ok(ExitCode::from(output::exit_code(status)))
}

async fn login(
    home: &Path,
    site: &str,
    account: &str,
    secret: &str,
    label: &str,
) -> AppResult<ExitCode> {
    sites::load(home, site)?;
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
    let vault = open(home)?.vault;
    vault.add_account(site, account, label).await?;
    vault
        .set_secret(site, account, secret, Secret::new(value))
        .await?;
    eprintln!("Saved {secret} for {site}/{account}.");
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
    if std::env::var("ITDA_SECRET_STORE").is_ok_and(|v| v == "env") {
        return Err(usage(
            "Environment secrets are read-only",
            "Sign in where the system keyring is available, or set the session in an ITDA_SECRET__ variable.",
        ));
    }
    KeyringStore::available()?;
    let parts = open(home)?;
    let session = tokio::task::spawn_blocking(move || login::browser(&spec, url.as_deref()))
        .await
        .map_err(|_| AppError::api("internal", 500))??;
    parts.vault.add_account(site, account, label).await?;
    parts
        .vault
        .set_secret(
            site,
            account,
            itda_runtime::SESSION_SECRET,
            Secret::new(session.cookies),
        )
        .await?;
    // A new sign-in replaces an expired one, so the account may run again.
    resume_scopes(home, &parts.net, site, Some(account)).await?;
    eprintln!(
        "Saved the session for {site}/{account} ({} cookies) in the system keyring. Agents only see the account id.",
        session.count
    );
    Ok(ExitCode::SUCCESS)
}

async fn resume(home: &Path, site: &str, account: Option<&str>) -> AppResult<ExitCode> {
    resume_scopes(home, &open(home)?.net, site, account).await?;
    eprintln!("Resumed {site}.");
    Ok(ExitCode::SUCCESS)
}

/// Lift pauses on every origin the site's actions use.
async fn resume_scopes(
    home: &Path,
    net: &Network,
    site: &str,
    account: Option<&str>,
) -> AppResult<()> {
    let spec = sites::load(home, site)?;
    let mut origins: Vec<String> = spec
        .operations
        .iter()
        .filter_map(|op| url::Url::parse(spec.operation_base(op)).ok())
        .map(|url| url.origin().ascii_serialization())
        .collect();
    origins.sort();
    origins.dedup();
    for origin in &origins {
        net.resume(origin, account.unwrap_or_default()).await?;
    }
    Ok(())
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
    eprintln!("Installed {} at {}.", spec.site.name, path.display());
    for line in sites::review(&spec) {
        eprintln!("  {line}");
    }
    eprintln!("Try `itda ls {}`.", spec.site.name);
    Ok(ExitCode::SUCCESS)
}

fn remove(home: &Path, site: &str) -> AppResult<ExitCode> {
    sites::remove(home, site)?;
    eprintln!("Removed {site}. Accounts signed in to it stay until `itda logout {site}`.");
    Ok(ExitCode::SUCCESS)
}

type Drafter = fn(&[u8], &itda_prep::Options) -> AppResult<itda_prep::Draft>;

/// A saved recording holds the pages the person saw as well as the calls
/// those pages made; both become actions.
fn draft_har(text: &[u8], options: &itda_prep::Options) -> AppResult<itda_prep::Draft> {
    itda_prep::from_recording(text, options, "`itda draft har`")
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
    publish(home, from(&text, &options)?, install, force)
}

/// Fetch each page with the honest client, then draft from what they show.
/// A page that refuses or redirects to a sign-in is reported, never worked
/// around; `itda record` covers sites that need an account.
async fn draft_page(
    home: &Path,
    urls: &[String],
    options: itda_prep::Options,
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
        pages.push(itda_prep::Page {
            url: response.url.clone(),
            html: String::from_utf8_lossy(&response.bytes).into_owned(),
        });
    }
    publish(
        home,
        itda_prep::from_pages(&pages, &options)?,
        install,
        force,
    )
}

/// Record the site in the person's own browser and draft from the recording,
/// which stays in memory unless `save` names a file for it. With `--install`,
/// the token or session the recording used can go straight to the keyring,
/// each after a yes in the terminal.
async fn record(
    home: &Path,
    url: &str,
    options: itda_prep::Options,
    install: bool,
    force: bool,
    save: Option<&Path>,
) -> AppResult<ExitCode> {
    let url = url.to_owned();
    let recording = tokio::task::spawn_blocking(move || record::browser(&url))
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
    let draft = itda_prep::from_recording(&har, &options, "`itda record`")?;
    let site = draft.site.clone();
    let code = publish(home, draft, install, force)?;
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
    let stdin = std::io::stdin();
    if !stdin.is_terminal() || std::env::var("ITDA_SECRET_STORE").is_ok_and(|v| v == "env") {
        return Ok(false);
    }
    let site = &spec.site.name;
    let account = itda_runtime::DEFAULT_ACCOUNT;
    let mut offers: Vec<(String, String, Secret)> = Vec::new();
    for auth in &spec.auth {
        match auth.kind.as_str() {
            "header" => {
                let Some(name) = auth.value_from.strip_prefix(&format!("store:{site}/")) else {
                    continue;
                };
                if let Some(value) =
                    record::header_value(har, &spec.site.base_url, &auth.header, &auth.prefix)
                {
                    offers.push((
                        format!("the {} header the pages sent", auth.header),
                        name.to_owned(),
                        Secret::new(value),
                    ));
                }
            }
            "browser" if !cookies.is_empty() => offers.push((
                format!("the browser session ({} cookies)", cookies.len()),
                itda_runtime::SESSION_SECRET.to_owned(),
                Secret::new(itda_browser::header(cookies)),
            )),
            _ => {}
        }
    }
    if offers.is_empty() {
        return Ok(false);
    }
    KeyringStore::available()?;
    let parts = open(home)?;
    let mut kept = false;
    for (what, name, secret) in offers {
        eprint!(
            "Keep {what} as {site}/{account} in the system keyring? Agents only see the account id. [y/N] "
        );
        let mut answer = String::new();
        stdin.lock().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            continue;
        }
        parts.vault.add_account(site, account, "").await?;
        parts.vault.set_secret(site, account, &name, secret).await?;
        resume_scopes(home, &parts.net, site, Some(account)).await?;
        eprintln!(
            "Saved {name} for {site}/{account}. Tokens expire; record again or use `itda login {site}` when calls answer auth_required."
        );
        kept = true;
    }
    Ok(kept)
}

/// Print the draft, or install it, and list what to review on stderr.
fn publish(
    home: &Path,
    draft: itda_prep::Draft,
    install: bool,
    force: bool,
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
    itda_types::util::atomic_file(&path, draft.yaml.as_bytes())?;
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
    Ok(ExitCode::SUCCESS)
}

/// Serve MCP on stdin and stdout. Stdout belongs to the protocol, so nothing
/// else is printed there; invalid sites are reported on stderr and skipped.
async fn mcp(home: &Path, mode: Option<&str>, approve_in_client: bool) -> AppResult<ExitCode> {
    let mut specs = Vec::new();
    for site in sites::list(home) {
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
    let mut server = itda_mcp::Server::new(
        Arc::new(parts.runtime()),
        itda_mcp::Catalog::new(specs),
        mode,
    );
    if approve_in_client {
        server = server.with_client_approval(approvals, audit);
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
        .ok_or_else(|| usage("No home directory was found", "Set HOME and try again."))?;
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
        (Ok(()), _) => report(true, "secrets", "system keyring".into()),
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
    let installed = sites::list(home);
    let invalid = installed.iter().filter(|s| s.spec.is_err()).count();
    report(
        invalid == 0,
        "sites",
        format!("{} installed, {invalid} invalid", installed.len()),
    );
    let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
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
    use super::{Command, First, first_command, tilde};
    use std::path::Path;

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
