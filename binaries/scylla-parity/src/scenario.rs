use crate::client::{CallOpts, Client, Meta, OpenStream, code_name};
use crate::normalize::{Normalizer, is_strict_ulid};
use crate::template::{Vars, pick, render, render_str};
use crate::{http, server_log};
use anyhow::{Context as _, Result, anyhow};
use prost_reflect::DescriptorPool;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use sqlx::pool::PoolConnection;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

pub const AGENT_OPEN: &str = "scylla.agent.v1.AgentService/Open";
const DRAIN_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub call: Option<String>,
    #[serde(default = "empty_object")]
    pub req: Value,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub save: BTreeMap<String, String>,
    #[serde(default)]
    pub expect: Option<String>,
    #[serde(default)]
    pub mask: Vec<String>,
    #[serde(default)]
    pub sort: Vec<String>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub send: Vec<Value>,
    #[serde(default)]
    pub drain: Option<String>,
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default)]
    pub close: Option<String>,
    #[serde(default)]
    pub hold: Option<String>,
    #[serde(default)]
    pub release: bool,
    #[serde(default)]
    pub sleep_ms: Option<u64>,
    #[serde(default)]
    pub push: Option<String>,
    #[serde(default)]
    pub until: BTreeMap<String, Value>,
    #[serde(default)]
    pub poll_ms: Option<u64>,
    #[serde(default)]
    pub every_ms: Option<u64>,
    #[serde(default)]
    pub http: Option<Http>,
    #[serde(default)]
    pub log: Option<LogWatch>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Http {
    #[serde(default = "post")]
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub sign: Option<String>,
}

fn post() -> String {
    "POST".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogWatch {
    pub target: String,
    pub message: String,
    #[serde(default)]
    pub keep: Vec<String>,
    #[serde(default)]
    pub save: BTreeMap<String, String>,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub step: usize,
    pub label: String,
    pub service: String,
    pub rpc: String,
    pub code: String,
    pub message: String,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Meta::is_empty")]
    pub meta: Meta,
}

impl Record {
    pub fn local(label: String, rpc: &str, code: &str, message: String, body: Value) -> Self {
        Self {
            step: 0,
            label,
            service: String::new(),
            rpc: rpc.to_string(),
            code: code.to_string(),
            message,
            body,
            meta: Meta::new(),
        }
    }
}

pub struct Run {
    pub records: Vec<Record>,
    pub mismatches: Vec<String>,
}

pub fn parse(sources: &[&str]) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for source in sources {
        let scenario: Scenario = serde_json::from_str(source).context("parse a scenario")?;
        steps.extend(scenario.steps);
    }
    Ok(steps)
}

pub struct Context<'a> {
    pub client: &'a Client,
    pub db: Option<&'a PgPool>,
    pub server: &'a str,
    pub log: Option<&'a Path>,
}

#[derive(Default)]
struct Live {
    streams: HashMap<String, OpenStream>,
    held: Option<PoolConnection<sqlx::Postgres>>,
    log_mark: u64,
}

#[derive(Default)]
pub struct Labels {
    known: Vec<(String, String)>,
    seen: HashSet<String>,
    last: HashMap<String, String>,
    versions: HashMap<String, usize>,
}

impl Labels {
    pub fn observe(&mut self, vars: &Vars) {
        for (name, value) in vars {
            if self.last.get(name) == Some(value) {
                continue;
            }
            self.last.insert(name.clone(), value.clone());
            if !self.seen.insert(value.clone()) {
                continue;
            }
            let version = self.versions.entry(name.clone()).or_default();
            *version += 1;
            let label = if *version == 1 {
                name.clone()
            } else {
                format!("{name}#{version}")
            };
            self.known.push((value.clone(), label));
        }
    }

    pub fn known(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.known.iter().cloned()
    }
}

pub fn request_id(step: usize) -> String {
    format!("parity-{step}")
}

pub async fn run(ctx: &Context<'_>, steps: &[Step], vars: &mut Vars) -> Run {
    let mut records = Vec::new();
    let mut mismatches = Vec::new();
    let mut labels = Labels::default();
    let mut live = Live::default();
    labels.observe(vars);
    for (index, step) in steps.iter().enumerate() {
        let outcome = execute(ctx, step, index, vars, &mut live).await;
        labels.observe(vars);
        let record = match outcome {
            Ok(record) => record,
            Err(e) => Record::local(
                step.label.clone().unwrap_or_default(),
                step.call.as_deref().unwrap_or("step"),
                "HARNESS_ERROR",
                format!("{e:#}"),
                Value::Null,
            ),
        };
        let record = Record {
            step: index,
            ..record
        };
        let expected = step.expect.as_deref().unwrap_or("OK");
        if record.code != expected {
            mismatches.push(format!(
                "step {index} {} {}: expected {expected}, got {} {:?}",
                record.label, record.rpc, record.code, record.message
            ));
        }
        if record.code == "OK" {
            mismatches.extend(bad_ids(index, step, vars));
            if let (Some(want), Value::Array(items)) = (step.count, &record.body)
                && items.len() < want
            {
                mismatches.push(format!(
                    "step {index} {}: expected {want} messages, got {}",
                    record.label,
                    items.len()
                ));
            }
        }
        records.push(record);
    }
    for (_, stream) in live.streams.drain() {
        stream.task.abort();
    }
    let known = labels
        .known()
        .chain([(ctx.server.to_string(), "server".to_string())]);
    let literals: Vec<&Value> = steps
        .iter()
        .flat_map(|step| std::iter::once(&step.req).chain(&step.send))
        .collect();
    let mut normalizer = Normalizer::new(known).keep(&literals);
    let records = records
        .into_iter()
        .zip(steps)
        .map(|(r, step)| {
            let mut body = normalizer.value(&r.body);
            for pointer in &step.sort {
                if let Some(Value::Array(items)) = body.pointer_mut(pointer) {
                    items.sort_by_cached_key(Value::to_string);
                }
            }
            let meta = r
                .meta
                .iter()
                .map(|(k, v)| (k.clone(), normalizer.text(v)))
                .collect();
            Record {
                message: normalizer.text(&r.message),
                rpc: if r.service == "http" {
                    normalizer.text(&r.rpc)
                } else {
                    r.rpc.clone()
                },
                body,
                meta,
                ..r
            }
        })
        .collect();
    Run {
        records,
        mismatches,
    }
}

fn is_id_pointer(pointer: &str) -> bool {
    pointer.ends_with("/value")
        || pointer
            .rsplit('/')
            .next()
            .is_some_and(|s| s.ends_with("_id"))
}

fn is_builtin_role(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').count() > 1
        && value
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
}

fn bad_ids(index: usize, step: &Step, vars: &Vars) -> Vec<String> {
    step.save
        .iter()
        .filter(|(_, pointer)| is_id_pointer(pointer))
        .filter_map(|(name, pointer)| {
            let value = vars.get(name)?;
            (!is_strict_ulid(value) && !is_builtin_role(value))
                .then(|| format!("step {index}: {name} from {pointer} is not a ULID: {value:?}"))
        })
        .collect()
}

pub struct Auth<'a> {
    pub auth: Option<&'a str>,
    pub header: Option<&'a str>,
    pub headers: &'a BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
}

impl Step {
    fn auth(&self) -> Auth<'_> {
        Auth {
            auth: self.auth.as_deref(),
            header: self.header.as_deref(),
            headers: &self.headers,
            timeout_ms: self.timeout_ms,
        }
    }
}

impl Auth<'_> {
    pub fn opts(&self, step: usize, vars: &Vars) -> Result<CallOpts> {
        let mut opts = if let Some(header) = self.header {
            CallOpts {
                authorization: Some(render_str(header, vars)?),
                ..CallOpts::default()
            }
        } else {
            let token = self.auth.map(|t| render_str(t, vars)).transpose()?;
            CallOpts::bearer(token.as_deref())
        };
        opts.headers
            .push(("x-request-id".to_string(), request_id(step)));
        for (name, value) in self.headers {
            opts.headers.push((name.clone(), render_str(value, vars)?));
        }
        opts.timeout = self.timeout_ms.map(Duration::from_millis);
        Ok(opts)
    }
}

fn split(call: &str) -> (String, String) {
    let (service, rpc) = call.split_once('/').unwrap_or((call, ""));
    (service.to_string(), rpc.to_string())
}

async fn execute(
    ctx: &Context<'_>,
    step: &Step,
    index: usize,
    vars: &mut Vars,
    live: &mut Live,
) -> Result<Record> {
    for (name, value) in &step.set {
        let value = render_str(value, vars)?;
        vars.insert(name.clone(), value);
    }
    let label = step.label.clone().unwrap_or_default();
    if let Some(watch) = &step.log {
        return watch_log(ctx, step, watch, label, vars, live).await;
    }
    if let Some(path) = ctx.log {
        live.log_mark = server_log::len(path);
    }
    if let Some(name) = &step.push {
        let stream = live
            .streams
            .get(name)
            .ok_or_else(|| anyhow!("no open stream {name}"))?;
        let messages: Vec<Value> = step
            .send
            .iter()
            .map(|m| render(m, vars))
            .collect::<Result<_>>()?;
        stream.push(&messages).await?;
        return Ok(Record::local(
            label,
            &format!("push {name}"),
            "OK",
            String::new(),
            Value::Null,
        ));
    }
    if let Some(call) = &step.http {
        return send_http(ctx, call, label, vars).await;
    }
    if let Some(sql) = &step.sql {
        let db = ctx
            .db
            .ok_or_else(|| anyhow!("a sql step needs --database-url"))?;
        let sql = render_str(sql, vars)?;
        let done = sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(db).await;
        return Ok(Record::local(
            label,
            "sql",
            if done.is_ok() { "OK" } else { "SQL_ERROR" },
            done.err().map(|e| e.to_string()).unwrap_or_default(),
            Value::Null,
        ));
    }
    if let Some(sql) = &step.query {
        let db = ctx
            .db
            .ok_or_else(|| anyhow!("a query step needs --database-url"))?;
        let sql = format!(
            "SELECT coalesce(json_agg(q), '[]'::json)::text FROM ({}) q",
            render_str(sql, vars)?
        );
        let window = step.poll_ms.map_or(DRAIN_WINDOW, Duration::from_millis);
        let every = Duration::from_millis(step.every_ms.unwrap_or(100));
        let started = Instant::now();
        loop {
            let rows: Result<String, _> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.clone()))
                .fetch_one(db)
                .await;
            let record = match rows {
                Ok(rows) => Record::local(
                    label.clone(),
                    "query",
                    "OK",
                    String::new(),
                    serde_json::from_str(&rows)?,
                ),
                Err(e) => Record::local(
                    label.clone(),
                    "query",
                    "SQL_ERROR",
                    e.to_string(),
                    Value::Null,
                ),
            };
            if step.until.is_empty()
                || reached(&step.until, &record.code, &record.body)
                || started.elapsed() + every > window
            {
                return Ok(record);
            }
            tokio::time::sleep(every).await;
        }
    }
    if let Some(sql) = &step.hold {
        let db = ctx
            .db
            .ok_or_else(|| anyhow!("a hold step needs --database-url"))?;
        let mut conn = db.acquire().await.context("acquire a connection")?;
        sqlx::raw_sql("BEGIN").execute(&mut *conn).await?;
        let sql = render_str(sql, vars)?;
        let done = sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *conn)
            .await;
        live.held = Some(conn);
        return Ok(Record::local(
            label,
            "hold",
            if done.is_ok() { "OK" } else { "SQL_ERROR" },
            done.err().map(|e| e.to_string()).unwrap_or_default(),
            Value::Null,
        ));
    }
    if step.release {
        let mut conn = live
            .held
            .take()
            .ok_or_else(|| anyhow!("release without a hold"))?;
        sqlx::raw_sql("ROLLBACK").execute(&mut *conn).await?;
        return Ok(Record::local(
            label,
            "release",
            "OK",
            String::new(),
            Value::Null,
        ));
    }
    if let Some(name) = &step.open {
        let call = step.call.as_deref().unwrap_or(AGENT_OPEN);
        let messages: Vec<Value> = step
            .send
            .iter()
            .map(|m| render(m, vars))
            .collect::<Result<_>>()?;
        let (service, rpc) = split(call);
        let opened = ctx
            .client
            .open(call, &messages, &step.auth().opts(index, vars)?)
            .await?;
        return Ok(match opened {
            Ok(stream) => {
                let meta = stream
                    .log
                    .lock()
                    .map(|l| l.meta.clone())
                    .unwrap_or_default();
                live.streams.insert(name.clone(), stream);
                Record {
                    service,
                    rpc,
                    meta,
                    ..Record::local(label, "", "OK", "stream open".to_string(), Value::Null)
                }
            }
            Err(outcome) => Record {
                step: 0,
                label,
                service,
                rpc,
                code: code_name(outcome.code).to_string(),
                message: outcome.message,
                body: outcome.body,
                meta: outcome.meta,
            },
        });
    }
    if let Some(name) = &step.drain {
        let stream = live
            .streams
            .get_mut(name)
            .ok_or_else(|| anyhow!("no open stream {name}"))?;
        let window = step.timeout_ms.map_or(DRAIN_WINDOW, Duration::from_millis);
        let started = Instant::now();
        loop {
            let (ready, ended) = stream
                .log
                .lock()
                .map(|l| (l.items.len() - stream.taken, l.end.is_some()))
                .map_err(|_| anyhow!("the stream log is poisoned"))?;
            if step.count.is_some_and(|c| ready >= c) || ended || started.elapsed() >= window {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        return take(stream, label, &format!("drain {name}"));
    }
    if let Some(name) = &step.close {
        let mut stream = live
            .streams
            .remove(name)
            .ok_or_else(|| anyhow!("no open stream {name}"))?;
        drop(stream.sender.take());
        let window = step.timeout_ms.map_or(DRAIN_WINDOW, Duration::from_millis);
        let _ = tokio::time::timeout(window, &mut stream.task).await;
        stream.task.abort();
        return take(&mut stream, label, &format!("close {name}"));
    }
    let Some(call) = &step.call else {
        if let Some(ms) = step.sleep_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            return Ok(Record::local(
                label,
                "sleep",
                "OK",
                String::new(),
                Value::Null,
            ));
        }
        return Ok(Record::local(
            label,
            "set",
            "OK",
            String::new(),
            Value::Null,
        ));
    };
    let request = render(&step.req, vars)?;
    let window = step.poll_ms.map_or(DRAIN_WINDOW, Duration::from_millis);
    let every = Duration::from_millis(step.every_ms.unwrap_or(100));
    let started = Instant::now();
    let outcome = loop {
        let outcome = ctx
            .client
            .call(call, &request, &step.auth().opts(index, vars)?)
            .await?;
        if step.until.is_empty()
            || reached(&step.until, code_name(outcome.code), &outcome.body)
            || started.elapsed() + every > window
        {
            break outcome;
        }
        tokio::time::sleep(every).await;
    };
    let mut body = outcome.body;
    if outcome.code == tonic::Code::Ok {
        for (name, pointer) in &step.save {
            vars.insert(name.clone(), pick(&body, pointer)?);
        }
    }
    for pointer in &step.mask {
        if let Some(slot) = body.pointer_mut(pointer) {
            *slot = Value::String("<masked>".to_string());
        }
    }
    let (service, rpc) = split(call);
    Ok(Record {
        step: 0,
        label,
        service,
        rpc,
        code: code_name(outcome.code).to_string(),
        message: outcome.message,
        body,
        meta: outcome.meta,
    })
}

fn take(stream: &mut OpenStream, label: String, rpc: &str) -> Result<Record> {
    let log = stream
        .log
        .lock()
        .map_err(|_| anyhow!("the stream log is poisoned"))?;
    let items: Vec<Value> = log.items[stream.taken..].to_vec();
    stream.taken = log.items.len();
    let message = match &log.end {
        None => "open".to_string(),
        Some((code, message)) => format!("closed {} {message}", code_name(*code)),
    };
    Ok(Record {
        meta: log.meta.clone(),
        ..Record::local(label, rpc, "OK", message, Value::Array(items))
    })
}

fn reached(until: &BTreeMap<String, Value>, code: &str, body: &Value) -> bool {
    until.iter().all(|(pointer, want)| {
        if pointer == "code" {
            return want.as_str() == Some(code);
        }
        let got = body.pointer(pointer);
        match want.as_str() {
            Some("<present>") => got.is_some(),
            Some("<absent>") => got.is_none(),
            _ => got == Some(want),
        }
    })
}

async fn send_http(ctx: &Context<'_>, call: &Http, label: String, vars: &Vars) -> Result<Record> {
    let path = render_str(&call.path, vars)?;
    let body = render_str(&call.body, vars)?;
    let mut headers = call
        .headers
        .iter()
        .map(|(k, v)| Ok((k.clone(), render_str(v, vars)?)))
        .collect::<Result<Vec<_>>>()?;
    if let Some(name) = &call.sign {
        let secret = vars
            .get(name)
            .ok_or_else(|| anyhow!("unknown variable {name:?}"))?;
        headers.push((
            "X-Scylla-Signature-256".to_string(),
            format!(
                "sha256={}",
                http::hmac_sha256_hex(secret.as_bytes(), body.as_bytes())
            ),
        ));
    }
    let reply = http::send(ctx.server, &call.method, &path, &headers, body.as_bytes()).await?;
    Ok(Record {
        service: "http".to_string(),
        ..Record::local(
            label,
            &path,
            &reply.status.to_string(),
            reply.body,
            Value::Null,
        )
    })
}

async fn watch_log(
    ctx: &Context<'_>,
    step: &Step,
    watch: &LogWatch,
    label: String,
    vars: &mut Vars,
    live: &mut Live,
) -> Result<Record> {
    let path = ctx
        .log
        .ok_or_else(|| anyhow!("a log step needs the log file of the server"))?;
    let window = step.timeout_ms.map_or(DRAIN_WINDOW, Duration::from_millis);
    let started = Instant::now();
    loop {
        let lines = server_log::lines_from(path, live.log_mark);
        let found = lines
            .iter()
            .find(|line| server_log::matches(line, &watch.target, &watch.message));
        if let Some(line) = found {
            let fields = server_log::fields(line, &watch.message);
            let mut body = serde_json::Map::new();
            for name in &watch.keep {
                body.insert(
                    name.clone(),
                    fields.get(name).cloned().unwrap_or(Value::Null),
                );
            }
            for (name, prefix) in &watch.save {
                let value = line
                    .split_once(prefix.as_str())
                    .and_then(|(_, rest)| rest.split_whitespace().next())
                    .ok_or_else(|| anyhow!("no {prefix:?} in the log line"))?
                    .trim_matches('"')
                    .to_string();
                body.insert(format!("{name} length"), Value::from(value.len()));
                vars.insert(name.clone(), value);
            }
            live.log_mark = server_log::len(path);
            return Ok(Record::local(
                label,
                "log",
                "OK",
                watch.message.clone(),
                Value::Object(body),
            ));
        }
        if started.elapsed() >= window {
            live.log_mark = server_log::len(path);
            return Ok(Record::local(
                label,
                "log",
                "NO_LOG",
                watch.message.clone(),
                Value::Null,
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn rpcs(pool: &DescriptorPool) -> BTreeSet<String> {
    pool.services()
        .filter(|s| s.full_name().starts_with("scylla."))
        .flat_map(|s| {
            s.methods()
                .map(|m| format!("{}/{}", s.full_name(), m.name()))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn called(steps: &[Step]) -> BTreeSet<String> {
    steps
        .iter()
        .filter_map(|step| match (&step.call, &step.open) {
            (Some(call), _) => Some(call.clone()),
            (None, Some(_)) => Some(AGENT_OPEN.to_string()),
            (None, None) => None,
        })
        .collect()
}

pub fn uncovered(descriptor_sets: &[&[u8]], scenarios: &[&str]) -> Result<(usize, Vec<String>)> {
    let mut pool = DescriptorPool::new();
    for set in descriptor_sets {
        pool.decode_file_descriptor_set(*set)?;
    }
    let all = rpcs(&pool);
    let called = called(&parse(scenarios)?);
    Ok((all.len(), all.difference(&called).cloned().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::Edition;

    #[test]
    fn every_proto_rpc_is_called_by_a_scenario_step() {
        let edition = Edition::ce();
        let (total, missing) = uncovered(&edition.descriptor_sets, &edition.scenarios).unwrap();
        assert!(missing.is_empty(), "{total} RPCs, not called: {missing:?}");
    }

    #[test]
    fn the_scenario_calls_known_rpcs_only() {
        let edition = Edition::ce();
        let mut pool = DescriptorPool::new();
        for set in &edition.descriptor_sets {
            pool.decode_file_descriptor_set(*set).unwrap();
        }
        let all = rpcs(&pool);
        let steps = parse(&edition.scenarios).unwrap();
        let unknown: Vec<_> = called(&steps).difference(&all).cloned().collect();
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn a_new_value_of_a_variable_gets_a_numbered_label() {
        let mut labels = Labels::default();
        let mut vars = Vars::new();
        vars.insert("tok".into(), "first-value".into());
        labels.observe(&vars);
        labels.observe(&vars);
        vars.insert("tok".into(), "second-value".into());
        vars.insert("alias".into(), "first-value".into());
        labels.observe(&vars);
        vars.insert("tok".into(), "third-value".into());
        labels.observe(&vars);
        let known: Vec<_> = labels.known().collect();
        assert_eq!(
            known,
            vec![
                ("first-value".to_string(), "tok".to_string()),
                ("second-value".to_string(), "tok#2".to_string()),
                ("third-value".to_string(), "tok#3".to_string()),
            ]
        );
    }

    #[test]
    fn a_saved_id_that_is_not_a_ulid_is_a_mismatch() {
        let step = Step {
            save: BTreeMap::from([
                ("a".to_string(), "/x/value".to_string()),
                ("b".to_string(), "/role_id".to_string()),
                ("c".to_string(), "/token".to_string()),
                ("d".to_string(), "/y/value".to_string()),
            ]),
            ..Step::default()
        };
        let vars = Vars::from([
            ("a".to_string(), "01m5tev79p6e446kzd1shmxz02".to_string()),
            ("b".to_string(), "organization-admin".to_string()),
            ("c".to_string(), "not an id".to_string()),
            (
                "d".to_string(),
                "6f1c2a9e-3b4d-4c5e-8f9a-0b1c2d3e4f5a".to_string(),
            ),
        ]);
        let bad = bad_ids(3, &step, &vars);
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].contains("d from /y/value"));
    }
}
