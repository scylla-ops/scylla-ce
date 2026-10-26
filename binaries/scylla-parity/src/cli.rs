use crate::client::{CallOpts, Client};
use crate::compare;
use crate::contract;
use crate::edition::Edition;
use crate::scenario::{self, Context, Record};
use crate::stack::{Postgres, ServerProcess};
use crate::template::Vars;
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use serde_json::json;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(about = "Run one fixed gRPC scenario against a server and compare the outputs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the scenario and the contract against a running server and write one JSON line for each call
    Run {
        #[arg(long, env = "PARITY_ADDR", default_value = "127.0.0.1:8080")]
        addr: String,
        #[arg(long, env = "PARITY_DATABASE_URL")]
        database_url: Option<String>,
        #[arg(long)]
        out: PathBuf,
        /// The log file of the server, for the `log` steps
        #[arg(long)]
        server_log: Option<PathBuf>,
    },
    /// Print the differences between two outputs of `run`
    Compare {
        left: PathBuf,
        right: PathBuf,
        /// A response metadata key to leave out of the comparison (an accepted difference)
        #[arg(long = "ignore-meta")]
        ignore_meta: Vec<String>,
    },
    /// Check the client contract against a running server
    Contract {
        #[arg(long, env = "PARITY_ADDR", default_value = "127.0.0.1:8080")]
        addr: String,
        #[arg(long, env = "PARITY_DATABASE_URL")]
        database_url: Option<String>,
    },
    /// Print the RPCs of the descriptor sets that no scenario step calls; fails when there is one
    Coverage,
    /// Start Postgres and the server for each run, run the scenario and the contract, and compare the runs
    Baseline {
        #[arg(long)]
        server: PathBuf,
        #[arg(long, default_value_t = 2)]
        runs: usize,
        #[arg(long, default_value_t = 18080)]
        port: u16,
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// A second server (B): one more run, compared with the first run of `--server` (A)
        #[arg(long)]
        b_server: Option<PathBuf>,
        /// An environment variable K=V for the B server
        #[arg(long = "b-env", value_parser = parse_env)]
        b_env: Vec<(String, String)>,
        /// A suffix for the Postgres container names, to run two baselines at once
        #[arg(long)]
        suffix: Option<String>,
        /// A response metadata key to leave out of the comparisons
        #[arg(long = "ignore-meta")]
        ignore_meta: Vec<String>,
    },
}

fn parse_env(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .filter(|(k, _)| !k.is_empty())
        .ok_or_else(|| format!("{text:?} is not K=V"))
}

pub fn main(edition: &Edition) -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(dispatch(edition, cli.command)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(edition: &Edition, command: Command) -> Result<bool> {
    match command {
        Command::Run {
            addr,
            database_url,
            out,
            server_log,
        } => {
            let db = pool(database_url.as_deref()).await?;
            let client = Client::connect(&addr, &edition.descriptor_sets).await?;
            let ctx = Context {
                client: &client,
                db: db.as_ref(),
                server: &addr,
                log: server_log.as_deref(),
            };
            let side = run_side(edition, &ctx, &out).await?;
            Ok(side.scenario_ok && side.contract_ok)
        }
        Command::Compare {
            left,
            right,
            ignore_meta,
        } => {
            let diffs = compare::differences(
                &compare::read(&left)?,
                &compare::read(&right)?,
                &ignore_meta,
            );
            Ok(print_differences(&diffs))
        }
        Command::Contract { addr, database_url } => {
            let db = pool(database_url.as_deref()).await?;
            let client = Client::connect(&addr, &edition.descriptor_sets).await?;
            let ctx = Context {
                client: &client,
                db: db.as_ref(),
                server: &addr,
                log: None,
            };
            Ok(run_contract(edition, &ctx, 0).await?.0)
        }
        Command::Coverage => {
            let (total, missing) =
                scenario::uncovered(&edition.descriptor_sets, &edition.scenarios)?;
            for rpc in &missing {
                println!("not called: {rpc}");
            }
            println!("coverage: {} of {total} RPCs called", total - missing.len());
            Ok(missing.is_empty())
        }
        Command::Baseline {
            server,
            runs,
            port,
            out_dir,
            b_server,
            b_env,
            suffix,
            ignore_meta,
        } => {
            let out_dir =
                out_dir.unwrap_or_else(|| PathBuf::from("target/parity").join(edition.name));
            let plan = Plan {
                edition,
                port,
                out_dir: &out_dir,
                suffix: suffix.as_deref(),
                ignore_meta: &ignore_meta,
            };
            plan.baseline(&server, runs, b_server.as_deref(), &b_env)
                .await
        }
    }
}

async fn pool(url: Option<&str>) -> Result<Option<PgPool>> {
    Ok(match url {
        Some(url) => Some(PgPool::connect(url).await?),
        None => None,
    })
}

struct Side {
    scenario_ok: bool,
    contract_ok: bool,
    contract: String,
}

async fn run_side(edition: &Edition, ctx: &Context<'_>, out: &Path) -> Result<Side> {
    let steps = scenario::parse(&edition.scenarios)?;
    let run = scenario::run(ctx, &steps, &mut Vars::new()).await;
    for mismatch in &run.mismatches {
        println!("unexpected: {mismatch}");
    }
    println!(
        "scenario: {} steps, {} unexpected, output in {}",
        run.records.len(),
        run.mismatches.len(),
        out.display()
    );
    let mut records = run.records;
    let (contract_ok, contract, contract_records) =
        run_contract(edition, ctx, records.len()).await?;
    records.extend(contract_records);
    compare::write(out, &records)?;
    Ok(Side {
        scenario_ok: run.mismatches.is_empty(),
        contract_ok,
        contract,
    })
}

async fn run_contract(
    edition: &Edition,
    ctx: &Context<'_>,
    first_step: usize,
) -> Result<(bool, String, Vec<Record>)> {
    let contract = contract::parse(&edition.contracts)?;
    let report = contract::check(ctx, &contract, first_step).await?;
    for failure in &report.failures {
        println!("contract failure: {failure}");
    }
    let summary = format!(
        "contract: {} checks passed, {} failed",
        report.passed,
        report.failures.len()
    );
    println!("{summary}");
    Ok((report.failures.is_empty(), summary, report.records))
}

fn print_differences(diffs: &[String]) -> bool {
    for diff in diffs {
        println!("{diff}");
    }
    println!("{} differences", diffs.len());
    diffs.is_empty()
}

struct Plan<'a> {
    edition: &'a Edition,
    port: u16,
    out_dir: &'a Path,
    suffix: Option<&'a str>,
    ignore_meta: &'a [String],
}

impl Plan<'_> {
    async fn side(
        &self,
        server: &Path,
        env: &[(String, String)],
        name: &str,
    ) -> Result<(Side, PathBuf)> {
        let addr = format!("127.0.0.1:{}", self.port);
        let dir = std::path::absolute(self.out_dir.join(name))?;
        std::fs::create_dir_all(&dir)?;
        let container = match self.suffix {
            Some(suffix) => format!("scylla-parity-{}-{suffix}-{name}", self.edition.name),
            None => format!("scylla-parity-{}-{name}", self.edition.name),
        };
        drop(std::net::TcpListener::bind(&addr).with_context(|| {
            format!("{addr} is in use: stop the other server or give another --port")
        })?);
        let postgres = Postgres::start(&container)?;
        let mut process = ServerProcess::start(
            server,
            &dir,
            &addr,
            &postgres.url,
            self.edition.config_extra,
            env,
        )?;
        let client = wait_ready(&addr, self.edition, &mut process).await?;
        let db = PgPool::connect(&postgres.url).await?;
        let ctx = Context {
            client: &client,
            db: Some(&db),
            server: &addr,
            log: Some(&process.log),
        };
        let out = dir.join("scenario.jsonl");
        let side = run_side(self.edition, &ctx, &out).await;
        db.close().await;
        drop(process);
        drop(postgres);
        Ok((side?, out))
    }

    fn compare(&self, left: &Path, right: &Path) -> Result<bool> {
        println!("==> compare {} and {}", left.display(), right.display());
        Ok(print_differences(&compare::differences(
            &compare::read(left)?,
            &compare::read(right)?,
            self.ignore_meta,
        )))
    }

    async fn baseline(
        &self,
        server: &Path,
        runs: usize,
        b_server: Option<&Path>,
        b_env: &[(String, String)],
    ) -> Result<bool> {
        if runs == 0 {
            bail!("--runs must be at least 1");
        }
        let a_env: Vec<(String, String)> = self
            .edition
            .server_env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        let mut ok = true;
        let mut outputs = Vec::new();
        let mut contracts = Vec::new();
        for run in 1..=runs {
            println!("==> run {run} of {runs} (A: {})", server.display());
            let (side, out) = self.side(server, &a_env, &format!("run-{run}")).await?;
            ok &= side.scenario_ok && side.contract_ok;
            contracts.push(format!("A run {run}: {}", side.contract));
            outputs.push(out);
        }
        let b = match b_server {
            Some(b_server) => {
                println!("==> run B ({}, env {b_env:?})", b_server.display());
                let mut env = a_env.clone();
                env.extend(b_env.iter().cloned());
                let (side, out) = self.side(b_server, &env, "run-b").await?;
                ok &= side.scenario_ok && side.contract_ok;
                ok &= mounted(
                    &self.out_dir.join("run-b").join("server.log"),
                    self.edition,
                    b_env,
                )?;
                contracts.push(format!("B: {}", side.contract));
                Some(out)
            }
            None => None,
        };
        let first = outputs.first().ok_or_else(|| anyhow!("no output"))?.clone();
        for other in &outputs[1..] {
            ok &= self.compare(&first, other)?;
        }
        if let Some(b) = &b {
            println!("==> acceptance: A run 1 against B");
            let same = self.compare(&first, b)?;
            ok &= same;
            println!(
                "{}",
                if same {
                    "==> B equals A"
                } else {
                    "==> B differs from A"
                }
            );
        }
        for line in &contracts {
            println!("==> {line}");
        }
        println!(
            "{}",
            if ok {
                "==> baseline passed"
            } else {
                "==> baseline failed"
            }
        );
        Ok(ok)
    }
}

/// The B server serves every slice of the edition on strata, and the strata engine runs each
/// worker and the webhook ingress. A key of `--b-env SCYLLA_STRATA_SLICES` must also be served.
fn mounted(log: &Path, edition: &Edition, b_env: &[(String, String)]) -> Result<bool> {
    let text = crate::server_log::strip_ansi(&std::fs::read_to_string(log)?);
    let check = strata_check(&text, edition, b_env);
    if check.missing.is_empty() && check.on_ce.is_empty() {
        println!("==> B serves on strata: {}", check.served.join(","));
        return Ok(true);
    }
    if !check.missing.is_empty() {
        println!(
            "==> B does not serve on strata: {}",
            check.missing.join(",")
        );
    }
    for line in &check.on_ce {
        println!("==> B runs on the CE engine: {line}");
    }
    Ok(false)
}

struct StrataCheck {
    served: Vec<String>,
    missing: Vec<String>,
    on_ce: Vec<String>,
}

const SERVES: &str = "strata serves the slice";
const RUNS_WORKER: &str = "the engine runs the worker";
const RUNS_WEBHOOK: &str = "the engine runs the webhook ingress";

fn strata_check(text: &str, edition: &Edition, b_env: &[(String, String)]) -> StrataCheck {
    let field = |line: &str, message: &str, key: &str| {
        crate::server_log::fields(line, message)
            .get(key)
            .and_then(|v| v.as_str().map(str::to_owned))
    };
    let served: Vec<String> = text
        .lines()
        .filter(|line| line.contains(SERVES))
        .filter_map(|line| field(line, SERVES, "slice"))
        .collect();
    let on_strata = |line: &str, message: &str| {
        line.contains(message) && field(line, message, "engine").as_deref() == Some("strata")
    };
    let switched = b_env
        .iter()
        .find(|(k, _)| k == "SCYLLA_STRATA_SLICES")
        .map(|(_, keys)| keys.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut missing: Vec<String> = edition
        .strata_slices
        .iter()
        .copied()
        .chain(switched)
        .filter(|key| !key.is_empty() && !served.iter().any(|s| s == key))
        .map(|key| format!("slice={key}"))
        .collect();
    missing.dedup();
    for worker in &edition.strata_workers {
        let runs = text.lines().any(|line| {
            on_strata(line, RUNS_WORKER)
                && field(line, RUNS_WORKER, "worker").as_deref() == Some(worker)
        });
        if !runs {
            missing.push(format!("worker={worker}"));
        }
    }
    if !text.lines().any(|line| on_strata(line, RUNS_WEBHOOK)) {
        missing.push("webhook ingress".to_owned());
    }
    let on_ce = text
        .lines()
        .filter(|line| {
            [RUNS_WORKER, RUNS_WEBHOOK].iter().any(|message| {
                line.contains(message) && field(line, message, "engine").as_deref() == Some("ce")
            })
        })
        .map(str::to_owned)
        .collect();
    StrataCheck {
        served,
        missing,
        on_ce,
    }
}

async fn wait_ready(addr: &str, edition: &Edition, process: &mut ServerProcess) -> Result<Client> {
    let started = Instant::now();
    loop {
        if process.exited() {
            bail!("the server stopped, see {}", process.log.display());
        }
        if let Ok(client) = Client::connect(addr, &edition.descriptor_sets).await {
            let probe = client
                .call(
                    "scylla.auth.v1.AuthService/ValidateToken",
                    &json!({"token": "probe"}),
                    &CallOpts::default(),
                )
                .await;
            if matches!(probe, Ok(ref o) if o.code == tonic::Code::Ok) {
                return Ok(client);
            }
        }
        if started.elapsed() > Duration::from_secs(120) {
            bail!(
                "the server is not ready after 120 s, see {}",
                process.log.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(engine: &str, slices: &[&str]) -> String {
        let mut lines = Vec::new();
        for worker in ["CronTriggers", "DispatchJobs", "ReapJobs", "SweepSessions"] {
            lines.push(format!(
                "2026-10-26T09:32:22.164991Z  INFO scylla_strata: the engine runs the worker worker={worker} engine=\"{engine}\""
            ));
        }
        lines.push(format!(
            "2026-10-26T09:32:22.165059Z  INFO scylla_strata: the engine runs the webhook ingress engine=\"{engine}\""
        ));
        for slice in slices {
            lines.push(format!(
                "2026-10-26T09:32:22.700035Z  INFO scylla_strata::host: strata serves the slice slice={slice} edition=\"scylla_strata::ScyllaCeOnDb\""
            ));
        }
        lines.join("\n")
    }

    #[test]
    fn a_b_server_on_strata_for_everything_passes() {
        let edition = Edition::ce();
        let check = strata_check(&log("strata", &edition.strata_slices), &edition, &[]);
        assert!(check.missing.is_empty(), "{:?}", check.missing);
        assert!(check.on_ce.is_empty());
        assert_eq!(check.served.len(), 12);
    }

    #[test]
    fn a_missing_slice_or_a_ce_engine_fails() {
        let edition = Edition::ce();
        let slices = &edition.strata_slices[1..];
        let check = strata_check(&log("strata", slices), &edition, &[]);
        assert_eq!(check.missing, ["slice=projects"]);

        let check = strata_check(&log("ce", &edition.strata_slices), &edition, &[]);
        assert_eq!(check.on_ce.len(), 5);
        assert_eq!(check.missing.len(), 5);

        let check = strata_check("", &edition, &[]);
        assert_eq!(check.missing.len(), 12 + 4 + 1);
    }

    #[test]
    fn a_key_of_the_old_b_env_must_also_be_served() {
        let edition = Edition::ce();
        let env = [(
            "SCYLLA_STRATA_SLICES".to_owned(),
            "jobs, ee-limits".to_owned(),
        )];
        let check = strata_check(&log("strata", &edition.strata_slices), &edition, &env);
        assert_eq!(check.missing, ["slice=ee-limits"]);
    }
}
