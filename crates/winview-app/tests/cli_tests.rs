// crates/winview-app/tests/cli_tests.rs

#[cfg(test)]
mod cli_tests {
    use std::process::Command;

    fn get_binary() -> &'static str {
        env!("CARGO_BIN_EXE_winview-app")
    }

    #[test]
    fn cli_help_works() {
        let binary_path = get_binary();

        let output = Command::new(binary_path)
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
    fn cli_dump_outputs_valid_snapshot() {
        use winview_core::model::{InspectionSnapshot, InspectionStatus};

        let output = Command::new(get_binary())
            .args(["dump", "--format", "json"])
            .output()
            .expect("Failed to run CLI binary");

        let snapshot: InspectionSnapshot =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "Invalid JSON snapshot: {}; stderr: {}",
                    error,
                    String::from_utf8_lossy(&output.stderr)
                )
            });

        let expected_code = match snapshot.status {
            InspectionStatus::Complete => 0,
            InspectionStatus::Partial { .. } => 3,
            InspectionStatus::Failed { .. } => 1,
        };

        assert_eq!(output.status.code(), Some(expected_code));

        assert!(!snapshot.metadata.captured_at.is_empty());
    }

    #[test]
    fn cli_invalid_subcommand_returns_error() {
        let binary_path = get_binary();

        let output = Command::new(binary_path)
            .arg("nonexistent-command")
            .output()
            .expect("Failed to run CLI binary");
        assert_eq!(output.status.code(), Some(2));
    }

    #[test]
    fn cli_version_flag_works() {
        let binary_path = get_binary();

        let output = Command::new(binary_path)
            .arg("--version")
            .output()
            .expect("Failed to run CLI binary");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn cli_unsupported_format_returns_error() {
        let output = Command::new(get_binary())
            .args(["dump", "--format", "xml"])
            .output()
            .expect("Failed to run CLI binary");

        assert!(output.stdout.is_empty());

        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(stderr.contains("Unsupported format: xml"));

        assert_eq!(output.status.code(), Some(2));
    }

    #[test]
    fn cli_subcommand_help_works() {
        for command in ["inspect", "dump", "export"] {
            let output = Command::new(get_binary())
                .args([command, "--help"])
                .output()
                .expect("Failed to run CLI binary");

            assert!(output.status.success(), "Help failed for {command}");

            let stdout = String::from_utf8_lossy(&output.stdout);

            assert!(stdout.contains(command));
            assert!(stdout.contains("Usage:"));
        }
    }

    #[test]
    fn cli_inspect_outputs_human_readable_summary() {
        let output = Command::new(get_binary())
            .arg("inspect")
            .output()
            .expect("Failed to run CLI binary");

        let stdout = String::from_utf8(output.stdout).expect("Expected UTF-8 output");

        for expected in [
            "WinViewRs Inspection Snapshot",
            "Captured at:",
            "Status:",
            "HKCU Folder Types:",
            "HKLM Folder Types:",
            "Bags:",
            "Streams:",
            "Advanced:",
            "CLSID:",
            "Policies:",
            "Errors:",
        ] {
            assert!(
                stdout.contains(expected),
                "Missing summary field: {expected}"
            );
        }

        let status = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Status: "))
            .expect("Missing inspection status");

        let expected_code = if status == "Complete" {
            0
        } else if status.starts_with("Partial {") {
            3
        } else if status.starts_with("Failed {") {
            1
        } else {
            panic!("Unexpected inspection status: {status}");
        };

        assert_eq!(output.status.code(), Some(expected_code));
    }
}
