// winview-app/src/cli/output.rs

use anyhow::Result;
use winview_core::RegistryReader;

pub fn run_export_toml() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;
    let snapshot = reader.inspect_all()?;

    // TOML export (lossy, for debugging)
    let toml_str = toml::to_string_pretty(&snapshot)?;
    println!("{}", toml_str);

    Ok(())
}
