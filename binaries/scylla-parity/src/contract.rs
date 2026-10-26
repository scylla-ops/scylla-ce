use crate::client::{CallOpts, Client, code_name};
use crate::normalize::Normalizer;
use crate::scenario::{self, Auth, Context, Labels, Record, Step, request_id};
use crate::template::{Vars, render, render_str};
use anyhow::{Context as _, Result, anyhow};
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const LOAD_CONNECTIONS: usize = 12;
const STORE_CODE: &str = "INTERNAL";
const STORE_MESSAGE: &str = "Internal server error";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    #[serde(default)]
    pub setup: Vec<Step>,
    #[serde(default)]
    pub unauthenticated: Option<Unauthenticated>,
    #[serde(default)]
    pub load: OneOrMany<Load>,
    #[serde(default)]
    pub store_failure: OneOrMany<StoreFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            Self::One(one) => vec![one],
            Self::Many(many) => many,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unauthenticated {
    #[serde(default)]
    pub calls: Vec<String>,
    #[serde(default)]
    pub cases: Vec<Case>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub service_cases: Vec<Case>,
    /// The code of a call that the harness sends with an empty request, when the caller passes
    /// the authentication: it replaces `OK` for that call.
    #[serde(default)]
    pub ok_codes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub codes: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub call: String,
    #[serde(default = "empty_object")]
    pub req: Value,
    #[serde(default)]
    pub auth: Option<String>,
    pub parallel: usize,
    #[serde(default)]
    pub expect: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreFailure {
    #[serde(default)]
    pub name: Option<String>,
    pub break_sql: String,
    pub repair_sql: String,
    pub auth: String,
    #[serde(default)]
    pub calls: Vec<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

impl Case {
    fn auth(&self) -> Auth<'_> {
        Auth {
            auth: self.auth.as_deref(),
            header: self.header.as_deref(),
            headers: &self.headers,
            timeout_ms: self.timeout_ms,
        }
    }
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Default)]
pub struct Merged {
    pub setup: Vec<Step>,
    pub unauthenticated: Unauthenticated,
    pub load: Vec<Load>,
    pub store_failure: Vec<StoreFailure>,
}

pub fn parse(sources: &[&str]) -> Result<Merged> {
    let mut merged = Merged::default();
    for source in sources {
        let part: Contract = serde_json::from_str(source).context("parse a contract")?;
        merged.setup.extend(part.setup);
        if let Some(part) = part.unauthenticated {
            merged.unauthenticated.calls.extend(part.calls);
            merged.unauthenticated.cases.extend(part.cases);
            merged.unauthenticated.services.extend(part.services);
            merged
                .unauthenticated
                .service_cases
                .extend(part.service_cases);
            merged.unauthenticated.ok_codes.extend(part.ok_codes);
        }
        merged.load.extend(part.load.into_vec());
        merged.store_failure.extend(part.store_failure.into_vec());
    }
    Ok(merged)
}

pub struct Report {
    pub passed: usize,
    pub failures: Vec<String>,
    pub records: Vec<Record>,
}

impl Report {
    fn check(&mut self, ok: bool, failure: impl FnOnce() -> String) {
        if ok {
            self.passed += 1;
        } else {
            self.failures.push(failure());
        }
    }
}

fn record(
    step: usize,
    label: String,
    call: &str,
    code: &str,
    message: String,
    body: Value,
) -> Record {
    let (service, rpc) = call.split_once('/').unwrap_or((call, ""));
    Record {
        step,
        service: service.to_string(),
        rpc: rpc.to_string(),
        ..Record::local(label, "", code, message, body)
    }
}

pub async fn check(ctx: &Context<'_>, contract: &Merged, first_step: usize) -> Result<Report> {
    let mut vars = Vars::new();
    let mut report = Report {
        passed: 0,
        failures: Vec::new(),
        records: Vec::new(),
    };
    let setup = scenario::run(ctx, &contract.setup, &mut vars).await;
    report
        .failures
        .extend(setup.mismatches.into_iter().map(|m| format!("setup: {m}")));
    let next = |report: &Report| first_step + report.records.len();

    let unauth = &contract.unauthenticated;
    for case in &unauth.cases {
        for call in &unauth.calls {
            let step = next(&report);
            let outcome = ctx
                .client
                .call(call, &json!({}), &case.auth().opts(step, &vars)?)
                .await?;
            let code = code_name(outcome.code);
            let ok_code = (case.code == "OK")
                .then(|| unauth.ok_codes.get(call))
                .flatten();
            let special = case.codes.get(call).or(ok_code);
            let expected = special.unwrap_or(&case.code);
            let message_ok =
                special.is_some() || case.message.as_deref().is_none_or(|m| m == outcome.message);
            report.check(code == expected && message_ok, || {
                format!(
                    "{call} with a {} token: expected {expected} {:?}, got {code} {:?}",
                    case.name, case.message, outcome.message
                )
            });
            let mut r = record(
                step,
                format!("contract {}", case.name),
                call,
                code,
                outcome.message,
                Value::Null,
            );
            r.meta = outcome.meta;
            report.records.push(r);
        }
    }

    let mut every_rpc = Vec::new();
    for service in &unauth.services {
        every_rpc.extend(ctx.client.unary_calls(service)?);
    }
    for case in &unauth.service_cases {
        for call in &every_rpc {
            let step = next(&report);
            let outcome = ctx
                .client
                .call(call, &json!({}), &case.auth().opts(step, &vars)?)
                .await?;
            let code = code_name(outcome.code);
            let message_ok = case.message.as_deref().is_none_or(|m| m == outcome.message);
            report.check(code == case.code && message_ok, || {
                format!(
                    "{call} (every RPC) with a {} token: expected {} {:?}, got {code} {:?}",
                    case.name, case.code, case.message, outcome.message
                )
            });
            let mut r = record(
                step,
                format!("contract every rpc {}", case.name),
                call,
                code,
                outcome.message,
                Value::Null,
            );
            r.meta = outcome.meta;
            report.records.push(r);
        }
    }

    for (index, store) in contract.store_failure.iter().enumerate() {
        let name = store
            .name
            .clone()
            .unwrap_or_else(|| format!("store failure {}", index + 1));
        let db = ctx
            .db
            .ok_or_else(|| anyhow!("the store failure contract needs --database-url"))?;
        let token = render_str(&store.auth, &vars)?;
        let calls = if store.calls.is_empty() {
            &unauth.calls
        } else {
            &store.calls
        };
        let code = store.code.as_deref().unwrap_or(STORE_CODE);
        let message = store.message.as_deref().unwrap_or(STORE_MESSAGE);
        sqlx::raw_sql(sqlx::AssertSqlSafe(store.break_sql.clone()))
            .execute(db)
            .await
            .with_context(|| format!("{name}: break the store"))?;
        let mut outcomes = Vec::new();
        for call in calls {
            let step = next(&report) + outcomes.len();
            let opts = CallOpts::bearer(Some(&token)).header("x-request-id", request_id(step));
            outcomes.push((call, step, ctx.client.call(call, &json!({}), &opts).await));
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(store.repair_sql.clone()))
            .execute(db)
            .await
            .with_context(|| format!("{name}: repair the store"))?;
        for (call, step, outcome) in outcomes {
            let outcome = outcome?;
            let got = code_name(outcome.code);
            report.check(got == code && outcome.message == message, || {
                format!(
                    "{name}: {call}: expected {code} {message:?}, got {got} {:?}",
                    outcome.message
                )
            });
            let mut r = record(
                step,
                format!("contract {name}"),
                call,
                got,
                outcome.message,
                Value::Null,
            );
            r.meta = outcome.meta;
            report.records.push(r);
        }
    }

    for load in &contract.load {
        let step = next(&report);
        let r = run_load(ctx, load, &vars, step).await?;
        let expected = load.expect.as_deref().unwrap_or("OK");
        let codes = &r.codes;
        let total: usize = codes.values().sum();
        println!(
            "load: {} x {} on {} connections, peak outstanding {}, codes {codes:?}, took {:.2?}",
            load.parallel, load.call, r.connections, r.peak, r.took
        );
        let all_expected = codes.keys().all(|c| *c == expected);
        report.check(all_expected && total == load.parallel, || {
            format!(
                "load {}: expected {} x {expected}, got {total} results {codes:?}",
                load.call, load.parallel
            )
        });
        report.records.push(record(
            step,
            "contract load".to_string(),
            &load.call,
            if all_expected && total == load.parallel {
                "OK"
            } else {
                "LOAD_FAILED"
            },
            String::new(),
            json!({"parallel": load.parallel, "total": total, "codes": codes}),
        ));
    }

    let mut labels = Labels::default();
    labels.observe(&vars);
    let mut normalizer = Normalizer::new(
        labels
            .known()
            .chain([(ctx.server.to_string(), "server".to_string())]),
    );
    for r in &mut report.records {
        r.message = normalizer.text(&r.message);
        r.meta = r
            .meta
            .iter()
            .map(|(k, v)| (k.clone(), normalizer.text(v)))
            .collect();
    }
    Ok(report)
}

struct LoadResult {
    codes: BTreeMap<&'static str, usize>,
    connections: usize,
    peak: usize,
    took: Duration,
}

async fn run_load(ctx: &Context<'_>, load: &Load, vars: &Vars, step: usize) -> Result<LoadResult> {
    let token = load
        .auth
        .as_deref()
        .map(|t| render_str(t, vars))
        .transpose()?;
    let request = render(&load.req, vars)?;
    let opts = CallOpts::bearer(token.as_deref()).header("x-request-id", request_id(step));
    let mut clients: Vec<Client> = Vec::with_capacity(LOAD_CONNECTIONS);
    for _ in 0..LOAD_CONNECTIONS {
        clients.push(ctx.client.reconnect().await?);
    }
    let outstanding = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let started = Instant::now();
    let calls = (0..load.parallel).map(|i| {
        let client = &clients[i % clients.len()];
        let (outstanding, peak, opts, request) = (&outstanding, &peak, &opts, &request);
        async move {
            let now = outstanding.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            let outcome = client.call(&load.call, request, opts).await;
            outstanding.fetch_sub(1, Ordering::SeqCst);
            outcome
        }
    });
    let mut codes: BTreeMap<&'static str, usize> = BTreeMap::new();
    for outcome in join_all(calls).await {
        let code = outcome.map_or("HARNESS_ERROR", |o| code_name(o.code));
        *codes.entry(code).or_default() += 1;
    }
    Ok(LoadResult {
        codes,
        connections: clients.len(),
        peak: peak.load(Ordering::SeqCst),
        took: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_of_all_contract_files_are_merged() {
        let ce = r#"{
            "unauthenticated": {"calls": ["a/B"], "cases": [{"name": "missing", "code": "UNAUTHENTICATED"}]},
            "load": [{"call": "a/B", "parallel": 2}],
            "store_failure": [{"break_sql": "x", "repair_sql": "y", "auth": "t"}]
        }"#;
        let ee = r#"{
            "unauthenticated": {"calls": ["c/D"]},
            "load": {"call": "c/D", "parallel": 3, "req": {"k": "${v}"}},
            "store_failure": [{"break_sql": "x2", "repair_sql": "y2", "auth": "t2", "calls": ["c/D"]}]
        }"#;
        let merged = parse(&[ce, ee]).unwrap();
        assert_eq!(merged.unauthenticated.calls, vec!["a/B", "c/D"]);
        assert_eq!(merged.load.len(), 2);
        assert_eq!(merged.load[1].parallel, 3);
        assert_eq!(merged.load[0].req, json!({}));
        assert_eq!(merged.store_failure.len(), 2);
        assert!(merged.store_failure[0].calls.is_empty());
        assert_eq!(merged.store_failure[1].calls, vec!["c/D"]);
    }

    #[test]
    fn the_ce_contract_parses() {
        let merged = parse(&crate::edition::Edition::ce().contracts).unwrap();
        assert_eq!(merged.unauthenticated.calls.len(), 13);
        assert!(merged.store_failure.len() >= 2);
        assert!(merged.load.len() >= 2);
    }
}
