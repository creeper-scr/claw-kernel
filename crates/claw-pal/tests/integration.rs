//! Integration tests for claw-pal.
//!
//! Tests that exercise multiple components together: IPC framing + transport,
//! and ProcessManager interactions.

use claw_pal::{
    ExitStatus, InterprocessTransport, IpcConnection, IpcError, IpcListener, IpcTransport,
    ProcessConfig, ProcessManager, ProcessSignal, TokioProcessManager,
};
use std::time::Duration;

// ─── IPC integration ─────────────────────────────────────────────────────────

/// Trait metadata methods work correctly for the integration layer.
#[tokio::test]
#[ignore]
async fn test_ipc_connect_returns_connection() {
    let result = claw_pal::InterprocessTransport::connect("/tmp/claw_pal_integ.sock").await;
    assert!(result.is_ok());
    let conn: IpcConnection = result.unwrap();
    assert_eq!(conn.endpoint, "/tmp/claw_pal_integ.sock");
}

#[tokio::test]
#[ignore]
async fn test_ipc_listen_returns_listener() {
    let result = claw_pal::InterprocessTransport::listen("/tmp/claw_pal_integ.sock").await;
    assert!(result.is_ok());
    let listener: IpcListener = result.unwrap();
    assert_eq!(listener.endpoint, "/tmp/claw_pal_integ.sock");
}

#[tokio::test]
#[ignore]
async fn test_ipc_empty_endpoint_fails() {
    let conn_result = claw_pal::InterprocessTransport::connect("").await;
    assert!(conn_result.is_err());
    assert_eq!(conn_result.unwrap_err(), IpcError::InvalidMessage);

    let listen_result = claw_pal::InterprocessTransport::listen("").await;
    assert!(listen_result.is_err());
    assert_eq!(listen_result.unwrap_err(), IpcError::InvalidMessage);
}

/// Full round-trip: server accepts, client sends, server receives.
#[cfg(unix)]
#[tokio::test]
#[ignore]
async fn test_ipc_full_roundtrip() {
    let sock_path = format!("/tmp/claw_pal_integ_roundtrip_{}.sock", std::process::id());

    // Remove stale socket
    let _ = std::fs::remove_file(&sock_path);

    let sock_path_clone = sock_path.clone();
    let server_handle = tokio::spawn(async move {
        let server = InterprocessTransport::new_server(&sock_path_clone)
            .await
            .expect("server bind failed");
        server.recv().await.expect("server recv failed")
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = InterprocessTransport::new_client(&sock_path)
        .await
        .expect("client connect failed");
    client
        .send(b"hello from client")
        .await
        .expect("client send failed");

    let received = server_handle.await.expect("server task panicked");
    assert_eq!(received, b"hello from client");

    let _ = std::fs::remove_file(&sock_path);
}

// ─── ProcessManager integration ──────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_process_manager_spawn_and_wait() {
    let manager = TokioProcessManager::new();
    let config = ProcessConfig::new("echo".to_string()).with_arg("integration_test".to_string());
    let handle = manager.spawn(config).await.expect("spawn failed");
    assert!(handle.pid > 0);

    let status: ExitStatus = manager.wait(handle).await.expect("wait failed");
    assert!(status.success);
    assert_eq!(status.code, Some(0));
}

#[tokio::test]
#[ignore]
async fn test_process_manager_kill_process() {
    let manager = TokioProcessManager::new();

    #[cfg(unix)]
    let config = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
    #[cfg(windows)]
    let config = ProcessConfig::new("ping".to_string()).with_args(vec![
        "-n".to_string(),
        "60".to_string(),
        "127.0.0.1".to_string(),
    ]);

    let handle = manager.spawn(config).await.expect("spawn failed");
    manager.kill(handle).await.expect("kill failed");
}

#[tokio::test]
#[ignore]
async fn test_process_manager_terminate_with_grace() {
    let manager = TokioProcessManager::new();
    let config = ProcessConfig::new("echo".to_string()).with_arg("bye".to_string());
    let handle = manager.spawn(config).await.expect("spawn failed");
    // Fast process: terminate with grace period should succeed
    manager
        .terminate(handle, Duration::from_secs(5))
        .await
        .expect("terminate failed");
}

#[tokio::test]
#[ignore]
async fn test_process_manager_signal() {
    let manager = TokioProcessManager::new();

    #[cfg(unix)]
    let config = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
    #[cfg(windows)]
    let config = ProcessConfig::new("ping".to_string()).with_args(vec![
        "-n".to_string(),
        "60".to_string(),
        "127.0.0.1".to_string(),
    ]);

    let handle = manager.spawn(config).await.expect("spawn failed");
    manager
        .signal(handle, ProcessSignal::Kill)
        .await
        .expect("signal failed");
}

#[tokio::test]
#[ignore]
async fn test_process_manager_spawn_invalid_program_fails() {
    let manager = TokioProcessManager::new();
    let config = ProcessConfig::new("__no_such_binary_12345__".to_string());
    let result = manager.spawn(config).await;
    assert!(result.is_err());
}
