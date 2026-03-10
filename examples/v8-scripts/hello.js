/**
 * Example JavaScript tool for claw-kernel V8 engine.
 * 
 * This demonstrates the claw.* API available in JavaScript/TypeScript scripts.
 */

// Access global agent_id
const currentAgent = agent_id;

// Example: Log to console (goes to Rust tracing)
console.log(`Hello from agent: ${currentAgent}`);

// Example: Use tools bridge
// const result = await claw.tools.call("some_tool", { param: "value" });

// Example: Emit event
claw.events.emit("script_started", {
    agent: currentAgent,
    script: "hello.js",
    timestamp: Date.now()
});

// Example: Access directory paths
const configDir = claw.dirs.configDir();
const dataDir = claw.dirs.dataDir();

// Example: Memory operations (if memory store is configured)
// await claw.memory.set("last_run", Date.now());
// const lastRun = await claw.memory.get("last_run");

// Example: File system operations (if configured)
// const files = claw.fs.listDir("/tmp");

// Example: Network requests (if configured)
// const response = await claw.net.get("https://api.example.com/data");

// Return a value (will be converted to JSON)
({
    success: true,
    message: "Hello from V8 JavaScript!",
    agent: currentAgent,
    directories: {
        config: configDir,
        data: dataDir
    },
    features: {
        es2022: true,
        typescript: false,
        asyncAwait: true
    }
});
