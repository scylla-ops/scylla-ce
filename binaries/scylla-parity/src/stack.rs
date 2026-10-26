use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const POSTGRES_IMAGE: &str = "postgres:18-alpine";
const READY_TIMEOUT: Duration = Duration::from_secs(90);

pub struct Postgres {
    name: String,
    pub url: String,
}

impl Postgres {
    pub fn start(name: &str) -> Result<Self> {
        let _ = docker(&["rm", "-f", name]);
        docker(&[
            "run",
            "-d",
            "--rm",
            "--name",
            name,
            "-e",
            "POSTGRES_USER=scylla",
            "-e",
            "POSTGRES_PASSWORD=scylla",
            "-e",
            "POSTGRES_DB=scylla",
            "-p",
            "127.0.0.1::5432",
            POSTGRES_IMAGE,
        ])?;
        let mut postgres = Self {
            name: name.to_string(),
            url: String::new(),
        };
        let port = docker(&["port", name, "5432/tcp"])?;
        let port = port
            .lines()
            .next()
            .and_then(|line| line.rsplit(':').next())
            .context("read the port of the Postgres container")?
            .trim()
            .to_string();
        let started = Instant::now();
        while docker(&[
            "exec",
            name,
            "pg_isready",
            "-U",
            "scylla",
            "-d",
            "scylla",
            "-h",
            "127.0.0.1",
        ])
        .is_err()
        {
            if started.elapsed() > READY_TIMEOUT {
                bail!("Postgres in {name} is not ready");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        postgres.url = format!("postgres://scylla:scylla@127.0.0.1:{port}/scylla");
        Ok(postgres)
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.name]);
    }
}

fn docker(args: &[&str]) -> Result<String> {
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("run docker")?;
    if !out.status.success() {
        bail!(
            "docker {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub struct ServerProcess {
    child: Child,
    pub log: PathBuf,
}

impl ServerProcess {
    pub fn start(
        binary: &Path,
        dir: &Path,
        addr: &str,
        database_url: &str,
        config_extra: &str,
        env: &[(String, String)],
    ) -> Result<Self> {
        let config = dir.join("config.toml");
        std::fs::write(&config, config_toml(addr, database_url, config_extra))?;
        let log = dir.join("server.log");
        let out = std::fs::File::create(&log)?;
        let child = Command::new(resolve(binary))
            .arg("--config")
            .arg(&config)
            .arg("--no-ui")
            .env_remove("SCYLLA_STRATA_SLICES")
            .env_remove("SCYLLA_STRATA_PROJECTS")
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env_remove("DATABASE_URL")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .spawn()
            .with_context(|| format!("start {}", binary.display()))?;
        Ok(Self { child, log })
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn resolve(binary: &Path) -> PathBuf {
    if !binary.exists() && cfg!(windows) {
        let exe = binary.with_extension("exe");
        if exe.exists() {
            return exe;
        }
    }
    binary.to_path_buf()
}

fn config_toml(addr: &str, database_url: &str, extra: &str) -> String {
    format!(
        r#"[server]
address = "{addr}"
public_url = "http://{addr}"

[ui]
enabled = false

[database]
url = "{database_url}"
max_connections = 16
min_connections = 1
acquire_timeout = "10s"
run_migrations = true

[bootstrap]
username = "admin"
password = "admin123"
email = "admin@example.com"

[secrets]
master_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
{extra}"#
    )
}
