// winview-app/src/cli/inspect.rs

use anyhow::Result;
use winview_core::RegistryReader;

pub fn run_inspect() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;
    let snapshot = reader.inspect_all()?;

    // Human-readable table output
    println!("WinViewRs Inspection Snapshot");
    println!("=============================");
    println!("Captured at: {}", snapshot.metadata.captured_at);
    println!("Windows Build: {:?}", snapshot.metadata.windows_build);
    println!("Windows UBR: {:?}", snapshot.metadata.windows_ubr);
    println!("Product Name: {:?}", snapshot.metadata.windows_product_name);
    println!("Detection Method: {:?}", snapshot.metadata.detection_method);
    println!("Status: {:?}", snapshot.status);
    println!();

    println!(
        "HKCU Folder Types: {} types",
        snapshot.hkcu_folder_types.types.len()
    );
    println!(
        "HKLM Folder Types: {} types",
        snapshot.hklm_folder_types.types.len()
    );
    println!("Bags: {} keys", snapshot.bags.len());
    println!("Streams: {} keys", snapshot.streams.len());
    println!("Advanced: {} values", snapshot.advanced.values.len());
    println!("CLSID: {} keys", snapshot.clsid.len());
    println!("Policies: {} keys", snapshot.policies.len());
    println!("Errors: {}", snapshot.errors.len());

    Ok(())
}

pub fn run_dump_json() -> Result<()> {
    let reader = winview_registry::WindowsRegistryReader;
    let snapshot = reader.inspect_all()?;

    let json = serde_json::to_string_pretty(&snapshot)?;
    println!("{}", json);

    Ok(())
}
