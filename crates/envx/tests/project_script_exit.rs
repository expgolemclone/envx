use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn project_run_returns_nonzero_when_script_fails() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("envx-project-script-{unique}"));
    fs::create_dir_all(&temp_dir).unwrap();

    let failing_command = if cfg!(windows) { "exit /b 42" } else { "exit 42" };
    let config_path = temp_dir.join("config.yaml");
    let config = format!(
        "name: test\ndescription: null\nrequired: []\ndefaults: {{}}\nauto_load: []\nprofile: null\nscripts:\n  fail:\n    description: fail intentionally\n    run: '{failing_command}'\n    env: {{}}\nvalidation:\n  warn_unused: false\n  strict_names: false\n  patterns: {{}}\ninherit: true\n"
    );
    fs::write(&config_path, config).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_envx"))
        .args(["project", "run", "fail", "--file"])
        .arg(&config_path)
        .env("TERM", "dumb")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    assert!(!stdout.contains("completed"));
    assert!(stderr.contains("exit code 42"), "stderr was: {stderr}");

    let _ = fs::remove_dir_all(temp_dir);
}
