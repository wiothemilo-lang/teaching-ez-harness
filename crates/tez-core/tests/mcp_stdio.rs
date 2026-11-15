use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use serde_json::json;
use tez_core::mcp::{McpClient, McpServerConfig};

#[tokio::test]
async fn discovers_and_calls_demo_server() {
    let server = format!(
        "{}/../../examples/plugins/study-toolkit/server.py",
        env!("CARGO_MANIFEST_DIR")
    );
    if !Path::new(&server).exists() {
        return;
    }
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }

    let config = McpServerConfig {
        command: "python3".to_string(),
        args: vec![server],
        env: HashMap::new(),
        cwd: None,
        enabled: true,
        read_only: false,
        url: None,
        headers: HashMap::new(),
        bearer_token_env: None,
        oauth: None,
        sandbox: tez_core::mcp::McpSandboxSettings {
            enabled: false,
            ..Default::default()
        },
    };

    let (client, tools) = McpClient::connect("demo", &config, None).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    assert!(tools.iter().any(|tool| tool.name == "add"));

    let output = client.call("add", json!({"a": 2, "b": 3})).await.unwrap();
    assert_eq!(output.content, "5");
    assert!(!output.is_error);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn sandboxed_demo_server_discovers_tools() {
    let server = format!(
        "{}/../../examples/plugins/study-toolkit/server.py",
        env!("CARGO_MANIFEST_DIR")
    );
    if !Path::new(&server).exists() {
        return;
    }
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    if tez_core::sandbox::probe().is_err() {
        return;
    }

    let data = tempfile::tempdir().unwrap();
    let config = McpServerConfig {
        command: "python3".to_string(),
        args: vec![server],
        env: HashMap::new(),
        cwd: None,
        enabled: true,
        read_only: false,
        url: None,
        headers: HashMap::new(),
        bearer_token_env: None,
        oauth: None,
        sandbox: tez_core::mcp::McpSandboxSettings {
            enabled: true,
            allow_network: false,
            writable_paths: Vec::new(),
        },
    };

    let (client, tools) = McpClient::connect("demo", &config, Some(data.path()))
        .await
        .unwrap();
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    let output = client.call("add", json!({"a": 4, "b": 5})).await.unwrap();
    assert_eq!(output.content, "9");
}
