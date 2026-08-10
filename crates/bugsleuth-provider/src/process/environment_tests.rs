//! Real-process checks for the provider environment boundary.

use super::*;

const CONTROLLER: &str = "BUGSLEUTH_ENV_TEST_CONTROLLER";
const SECRET: &str = "BUGSLEUTH_TEST_SECRET_7D9C";
const TEST: &str = "process::environment_tests::child_does_not_inherit_unapproved_environment";

fn shell() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        (
            "cmd.exe",
            vec![
                "/D".into(),
                "/C".into(),
                format!(
                    "if not defined PATH (exit /b 9) else (if defined {SECRET} (echo %{SECRET}%) \
                     else (echo absent))"
                ),
            ],
        )
    } else {
        (
            "sh",
            vec![
                "-c".into(),
                format!(
                    "[ -n \"$PATH\" ] || exit 9; if [ -n \"${{{SECRET}+set}}\" ]; then printf %s \
                     \"${SECRET}\"; else printf absent; fi"
                ),
            ],
        )
    }
}

async fn read_secret(env: &[(String, String)]) -> CliOutput {
    let (binary, args) = shell();
    run(Invocation {
        binary,
        args: &args,
        cwd: Path::new("."),
        stdin: None,
        env,
        timeout: Duration::from_secs(10),
        what: "environment test",
    })
    .await
    .expect("run environment helper")
}

#[tokio::test]
async fn child_does_not_inherit_unapproved_environment() {
    if std::env::var_os(CONTROLLER).is_none() {
        let status = Command::new(std::env::current_exe().expect("find this test executable"))
            .args(["--exact", TEST, "--nocapture"])
            .env(CONTROLLER, "1")
            .env(SECRET, "inherited-secret")
            .status()
            .await
            .expect("start environment-test controller");
        assert!(status.success(), "environment-test controller failed");
        return;
    }

    assert_eq!(
        std::env::var(SECRET).expect("controller has the sentinel secret"),
        "inherited-secret",
        "the fixture has no secret, so an empty child proves nothing"
    );

    let inherited = read_secret(&[]).await;
    assert!(inherited.succeeded(), "required environment was lost");
    assert_eq!(
        inherited.stdout.trim(),
        "absent",
        "an unapproved secret reached the provider child"
    );

    let explicit = vec![(SECRET.to_string(), "explicit-value".to_string())];
    let supplied = read_secret(&explicit).await;
    assert!(supplied.succeeded(), "explicit environment was unusable");
    assert_eq!(supplied.stdout.trim(), "explicit-value");
}
