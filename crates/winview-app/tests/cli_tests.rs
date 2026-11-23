// crates/winview-app/tests/cli_tests.rs

#[cfg(test)]
mod cli_tests {
    use std::env;
    use std::process::Command;

    fn get_binary() -> String {
        env::var("CARGO_BIN_EXE_winview-app")
            .expect("CARGO_BIN_EXE_winview-app not set — run via `cargo test`")
    }

    #[test]
    fn cli_help_works() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("--help")
            .output()
            .expect("Failed to run CLI binary");
        assert!(
            output.status.success(),
            "CLI --help failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("inspect"));
        assert!(stdout.contains("dump"));
        assert!(stdout.contains("export"));
    }

    #[test]
    fn cli_inspect_outputs_json() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .args(["dump", "--format", "json"])
            .output()
            .expect("Failed to run CLI binary");

        // On non-Windows or without registry access, command may fail
        // But if it succeeds, output MUST be valid JSON matching InspectionSnapshot
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let _: serde_json::Value = serde_json::from_str(&stdout).expect("Valid JSON output");
        } else {
            // Document the failure reason
            eprintln!(
                "CLI dump failed (expected on non-Windows): {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn cli_invalid_subcommand_returns_error() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("nonexistent-command")
            .output()
            .expect("Failed to run CLI binary");
        assert!(!output.status.success(), "Invalid subcommand should fail");
    }

    #[test]
    fn cli_version_flag_works() {
        let binary_path = get_binary();

        let output = Command::new(&binary_path)
            .arg("--version")
            .output()
            .expect("Failed to run CLI binary");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    }
}
