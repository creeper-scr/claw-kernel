//! Example: Script Hot-Reload in Layer 3
//!
//! This example demonstrates the hot-reload mechanism for Layer 3 scripts.
//! It watches a directory for Lua script changes and automatically reloads them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use claw_script::{
    HotReloadConfig, HotReloadManager, LuaEngine, ScriptContext, ScriptEvent,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Note: In a real application, you would initialize tracing here
    // tracing_subscriber::fmt::init();

    // Create the Lua script engine
    let engine = Arc::new(LuaEngine::new());

    // Configure hot-reload
    let config = HotReloadConfig::new()
        .watch_dir("./examples/scripts")
        .extension("lua")
        .debounce_delay(Duration::from_millis(100))
        .auto_reload(true)
        .validate_before_reload(true);

    // Create the hot-reload manager
    let manager = HotReloadManager::new(config, engine)?;

    // Subscribe to events
    let mut events = manager.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                ScriptEvent::Loaded { entry, path } => {
                    println!("✅ Loaded: {} from {:?}", entry.name, path);
                }
                ScriptEvent::Reloaded {
                    entry,
                    previous_version,
                    new_version,
                    ..
                } => {
                    println!(
                        "🔄 Reloaded: {} (v{} -> v{})",
                        entry.name, previous_version, new_version
                    );
                }
                ScriptEvent::Unloaded { name, .. } => {
                    println!("❌ Unloaded: {}", name);
                }
                ScriptEvent::Failed { path, error, .. } => {
                    eprintln!("❌ Failed to load {:?}: {}", path, error);
                }
                ScriptEvent::Started { directories } => {
                    println!("👁️  Started watching: {:?}", directories);
                }
                ScriptEvent::Stopped => {
                    println!("🛑 Stopped watching");
                }
                _ => {}
            }
        }
    });

    // Create example scripts directory and initial script
    let scripts_dir = PathBuf::from("./examples/scripts");
    tokio::fs::create_dir_all(&scripts_dir).await.ok();

    // Create an initial example script
    let example_script = r#"-- Example hot-reloadable script
return {
    message = "Hello from hot-reload!",
    timestamp = os.time(),
    version = 1
}
"#;
    tokio::fs::write(scripts_dir.join("example.lua"), example_script).await.ok();

    println!("Hot-reload example started.");
    println!("Edit ./examples/scripts/example.lua to see hot-reload in action.");
    println!("Press Ctrl+C to stop.\n");

    // Run for 30 seconds
    println!("Running for 30 seconds...");
    tokio::time::sleep(Duration::from_secs(30)).await;
    println!("Timeout reached, stopping...");

    // Demonstrate manual script operations
    println!("\n--- Manual Operations Demo ---");

    // Load a script manually
    let test_script = scripts_dir.join("example.lua");
    if test_script.exists() {
        let module = manager.load_file(&test_script).await?;
        println!("Manually loaded: {} (v{})", module.current().name, module.version());

        // Execute the script
        let ctx = ScriptContext::new("example-agent");
        match manager.execute("example", &ctx).await {
            Ok(result) => println!("Execution result: {}", result),
            Err(e) => eprintln!("Execution error: {}", e),
        }

        // Show version history
        if let Some(module) = manager.registry().get_or_create("example") {
            println!("Version history: {:?}", module.history_versions());
        }
    }

    // Cleanup
    tokio::fs::remove_dir_all(&scripts_dir).await.ok();
    println!("\nExample completed!");

    Ok(())
}
