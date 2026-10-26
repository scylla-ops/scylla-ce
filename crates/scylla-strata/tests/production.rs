use scylla_strata::ScyllaCe;
use strata::Edition;

#[tokio::test]
async fn production_refuses_each_dev_slot() {
    let config = strata::Config::new();
    let Err(error) = ScyllaCe::builder(&config).production().build().await else {
        panic!("a production build of the dev edition must fail");
    };
    let message = error.to_string();
    for slot in [
        "Access::authenticator",
        "Access::permissions",
        "Projects::store",
        "Projects::scopes",
        "Projects::directory",
        "Projects::jobs",
    ] {
        assert!(
            message
                .lines()
                .any(|line| line.contains(slot) && line.contains("for development only")),
            "{slot} is not refused as a dev slot in: {message}"
        );
    }
}
