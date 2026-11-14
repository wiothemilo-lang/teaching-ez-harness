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
    };

    let (client, tools) = McpClient::connect("demo", &config).await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    assert!(tools.iter().any(|tool| tool.name == "add"));

    let output = client.call("add", json!({"a": 2, "b": 3})).await.unwrap();
    assert_eq!(output.content, "5");
    assert!(!output.is_error);
}
