//! The tests of the install scripts of the agent that the server serves.

use scylla_core::api::ServeOptions;
use scylla_testkit::TestApp;
use sqlx::PgPool;

/// Reads a path of the server: its status, its content type and its body.
async fn get(app: &TestApp, path: &str) -> (u16, String, String) {
    let response = reqwest::get(format!("http://{}{path}", app.addr))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, kind, response.text().await.unwrap())
}

/// Each script has the URL of the server, its version and the folder of the downloads, and no
/// mark stays.
#[sqlx::test(migrations = false)]
async fn the_install_scripts_have_the_values_of_the_server(pool: PgPool) {
    // given
    let options = ServeOptions {
        agent_downloads: "https://downloads.example.com/agent".to_owned(),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let (url, version) = (format!("http://{}", app.addr), env!("CARGO_PKG_VERSION"));

    // when
    let (sh_status, sh_kind, sh) = get(&app, "/install.sh").await;
    let (ps1_status, ps1_kind, ps1) = get(&app, "/install.ps1").await;

    // then
    assert_eq!((sh_status, ps1_status), (200, 200));
    assert!(sh_kind.starts_with("text/x-shellscript"), "{sh_kind}");
    assert!(ps1_kind.starts_with("text/plain"), "{ps1_kind}");
    assert!(sh.starts_with("#!/bin/sh\n"));
    assert!(sh.contains(&format!("SCYLLA_URL='{url}'\n")));
    assert!(sh.contains(&format!("VERSION='{version}'\n")));
    assert!(sh.contains("DEFAULT_DOWNLOADS='https://downloads.example.com/agent'\n"));
    assert!(ps1.contains(&format!("$ScyllaUrl = '{url}'")));
    assert!(ps1.contains(&format!("$Version = '{version}'")));
    assert!(ps1.contains("$Downloads = 'https://downloads.example.com/agent'"));
    assert!(!sh.contains("@SCYLLA_") && !ps1.contains("@SCYLLA_"));
    assert!(!sh.contains("@DOWNLOADS") && !ps1.contains("@DOWNLOADS"));
}

/// The flags that the commands of the web app give are in each script.
#[sqlx::test(migrations = false)]
async fn the_install_scripts_take_the_flags_of_the_commands(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;

    // when
    let (_, _, sh) = get(&app, "/install.sh").await;
    let (_, _, ps1) = get(&app, "/install.ps1").await;

    // then
    for flag in [
        "--url",
        "--capacity",
        "--container-runtime",
        "--no-service",
        "--binary",
        "--update",
        "--uninstall",
        "--force",
    ] {
        assert!(sh.contains(flag), "{flag}");
    }
    assert!(sh.ends_with("main \"$@\"\n"));
    assert!(sh.contains("RestartPreventExitStatus=78"));
    for flag in [
        "$Url",
        "$Capacity",
        "$ContainerRuntime",
        "$CurrentUser",
        "$NoService",
        "$Binary",
        "$Update",
        "$Uninstall",
        "$Force",
    ] {
        assert!(ps1.contains(flag), "{flag}");
    }
}

/// With no administrator, the Windows script installs the agent for its user, and a task of the
/// user starts it at each sign-in with no window.
#[sqlx::test(migrations = false)]
async fn the_windows_script_installs_for_the_user_with_no_administrator(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;

    // when
    let (_, _, ps1) = get(&app, "/install.ps1").await;

    // then
    assert!(ps1.contains("$forUser = $NoService -or $CurrentUser -or -not $admin"));
    assert!(ps1.contains("New-ScheduledTaskTrigger -AtLogOn"));
    assert!(ps1.contains("-LogonType Interactive -RunLevel Limited"));
    assert!(ps1.contains("--headless cmd.exe"));
    assert!(ps1.contains("Unregister-ScheduledTask"));
}
