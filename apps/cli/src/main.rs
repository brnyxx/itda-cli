//! The `itda` command. This is the composition root: it picks the data
//! directory, the secret store and the network settings, and hands them to
//! the runtime. Commands that need no network never start an async runtime.
mod output;
mod sites;

use clap::{Parser, Subcommand};
use itda_net::{Gate, Network};
use itda_policy::operation_effect;
use itda_runtime::{Call, Runtime};
use itda_store::Store;
use itda_types::error::{AppError, AppResult, usage};
use itda_vault::{EnvStore, KeyringStore, Secret, SecretStore, Vault};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, IsTerminal},
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
    command: Command,
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
    /// Call an action: itda call <site>.<action> [--input value]... [--account id] [--json]
    Call {
        /// <site>.<action>
        target: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },
    /// Store a secret for a site account. The value is read from stdin
    Login {
        site: String,
        #[arg(long, default_value = itda_runtime::DEFAULT_ACCOUNT)]
        account: String,
        /// The secret name the site definition refers to (store:<site>/<name>)
        #[arg(long, default_value = "api-key")]
        secret: String,
        /// A label to recognise the account by
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Resume requests paused after a block or an expired sign-in
    Resume {
        site: String,
        #[arg(long)]
        account: Option<String>,
    },
    /// Check the data directory, the keyring and the installed sites
    Doctor,
    /// Serve installed actions to an AI agent over MCP (stdio)
    Mcp {
        /// direct: one tool per action. catalog: search, describe and call.
        /// Chosen from the number of actions when left out
        #[arg(long, value_parser = ["direct", "catalog"])]
        mode: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let as_json = matches!(&cli.command, Command::Ls { json: true, .. })
        || matches!(&cli.command, Command::Call { rest, .. } if rest.iter().any(|a| a == "--json"));
    let result = home().and_then(|home| match cli.command {
        Command::Ls { site, json } => ls(&home, site.as_deref(), json),
        Command::Call { target, rest } => block_on(call(&home, &target, rest)),
        Command::Login {
            site,
            account,
            secret,
            label,
        } => block_on(login(&home, &site, &account, &secret, &label)),
        Command::Resume { site, account } => block_on(resume(&home, &site, account.as_deref())),
        Command::Doctor => Ok(doctor(&home)),
        Command::Mcp { mode } => block_on(mcp(&home, mode.as_deref())),
    });
    match result {
        Ok(code) => code,
        Err(error) => output::fail(&error, as_json),
    }
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
}

fn open(home: &Path) -> AppResult<Parts> {
    let store = Store::open(&home.join("itda.db"))?;
    let gate = Arc::new(Gate::open(store.clone())?);
    Ok(Parts {
        net: Arc::new(Network::new(gate, allow_local())),
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

/// `--name value` or `--name=value` inputs, plus `--account` and `--json`.
fn parse_call(rest: Vec<String>) -> AppResult<(BTreeMap<String, String>, Option<String>, bool)> {
    let mut args = BTreeMap::new();
    let (mut account, mut as_json) = (None, false);
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
        if name == "account" {
            account = Some(value);
        } else {
            args.insert(name, value);
        }
    }
    Ok((args, account, as_json))
}

async fn call(home: &Path, target: &str, rest: Vec<String>) -> AppResult<ExitCode> {
    let (site, action) = target.split_once('.').ok_or_else(|| {
        usage(
            format!("{target} is not <site>.<action>"),
            "Run `itda ls` to see installed actions.",
        )
    })?;
    let (args, account, as_json) = parse_call(rest)?;
    let spec = sites::load(home, site)?;
    let parts = open(home)?;
    let envelope = Runtime::new(parts.net, parts.vault)
        .call(
            &spec,
            Call {
                action: action.to_owned(),
                args,
                account,
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

async fn resume(home: &Path, site: &str, account: Option<&str>) -> AppResult<ExitCode> {
    let spec = sites::load(home, site)?;
    let net = open(home)?.net;
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
    eprintln!("Resumed {site}.");
    Ok(ExitCode::SUCCESS)
}

/// Serve MCP on stdin and stdout. Stdout belongs to the protocol, so nothing
/// else is printed there; invalid sites are reported on stderr and skipped.
async fn mcp(home: &Path, mode: Option<&str>) -> AppResult<ExitCode> {
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
    let runtime = Arc::new(Runtime::new(parts.net, parts.vault));
    let server = itda_mcp::Server::new(runtime, itda_mcp::Catalog::new(specs), mode);
    server
        .serve_stdio()
        .await
        .map_err(|e| usage(format!("MCP session ended: {e}"), "Restart the MCP client."))?;
    Ok(ExitCode::SUCCESS)
}

fn doctor(home: &Path) -> ExitCode {
    let mut healthy = true;
    let mut report = |ok: bool, what: &str, detail: String| {
        healthy &= ok;
        println!("{}  {what:<16} {detail}", if ok { "ok  " } else { "FAIL" });
    };
    println!("itda {}", env!("CARGO_PKG_VERSION"));
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
    let installed = sites::list(home);
    let invalid = installed.iter().filter(|s| s.spec.is_err()).count();
    report(
        invalid == 0,
        "sites",
        format!("{} installed, {invalid} invalid", installed.len()),
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
    if healthy {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
