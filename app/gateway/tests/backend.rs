//! Tests for the MemoryBackend enum dispatch and configuration integration.

use walrus_gateway::MemoryBackend;

#[test]
fn in_memory_backend_set_and_get() {
    use agent::Memory;
    let backend = MemoryBackend::in_memory();
    assert!(backend.get("key").is_none());
    backend.set("key", "value");
    assert_eq!(backend.get("key").unwrap(), "value");
}

#[test]
fn in_memory_backend_entries() {
    use agent::Memory;
    let backend = MemoryBackend::in_memory();
    backend.set("a", "1");
    backend.set("b", "2");
    let entries = backend.entries();
    assert_eq!(entries.len(), 2);
}

#[test]
fn in_memory_backend_remove() {
    use agent::Memory;
    let backend = MemoryBackend::in_memory();
    backend.set("key", "value");
    let old = backend.remove("key");
    assert_eq!(old.unwrap(), "value");
    assert!(backend.get("key").is_none());
}

#[test]
fn sqlite_backend_set_and_get() {
    use agent::Memory;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let backend = MemoryBackend::sqlite(path.to_str().unwrap()).unwrap();
    assert!(backend.get("key").is_none());
    backend.set("key", "value");
    assert_eq!(backend.get("key").unwrap(), "value");
}

#[test]
fn sqlite_backend_entries() {
    use agent::Memory;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let backend = MemoryBackend::sqlite(path.to_str().unwrap()).unwrap();
    backend.set("a", "1");
    backend.set("b", "2");
    let entries = backend.entries();
    assert_eq!(entries.len(), 2);
}

#[test]
fn sqlite_backend_remove() {
    use agent::Memory;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let backend = MemoryBackend::sqlite(path.to_str().unwrap()).unwrap();
    backend.set("key", "value");
    let old = backend.remove("key");
    assert_eq!(old.unwrap(), "value");
    assert!(backend.get("key").is_none());
}

#[tokio::test]
async fn in_memory_backend_store() {
    use agent::Memory;
    let backend = MemoryBackend::in_memory();
    backend.store("key", "value").await.unwrap();
    assert_eq!(backend.get("key").unwrap(), "value");
}

#[tokio::test]
async fn sqlite_backend_store() {
    use agent::Memory;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let backend = MemoryBackend::sqlite(path.to_str().unwrap()).unwrap();
    backend.store("key", "value").await.unwrap();
    assert_eq!(backend.get("key").unwrap(), "value");
}

#[tokio::test]
async fn in_memory_backend_compile_relevant() {
    use agent::Memory;
    let backend = MemoryBackend::in_memory();
    backend.set("fact", "the sky is blue");
    let compiled = backend.compile_relevant("sky color").await;
    assert!(compiled.contains("the sky is blue"));
}

#[test]
fn memory_backend_from_config_inmemory() {
    use walrus_gateway::config::{MemoryBackendKind, MemoryConfig};
    let config = MemoryConfig {
        backend: MemoryBackendKind::InMemory,
        path: None,
    };
    assert_eq!(config.backend, MemoryBackendKind::InMemory);
    // Constructing in-memory should always succeed.
    let _backend = MemoryBackend::in_memory();
}

#[test]
fn memory_backend_from_config_sqlite() {
    use walrus_gateway::config::{MemoryBackendKind, MemoryConfig};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cfg.db");
    let config = MemoryConfig {
        backend: MemoryBackendKind::Sqlite,
        path: Some(path.to_str().unwrap().to_string()),
    };
    assert_eq!(config.backend, MemoryBackendKind::Sqlite);
    let backend = MemoryBackend::sqlite(config.path.as_deref().unwrap()).unwrap();
    use agent::Memory;
    backend.set("test", "ok");
    assert_eq!(backend.get("test").unwrap(), "ok");
}

#[test]
fn default_bind_address() {
    let config = walrus_gateway::GatewayConfig::from_toml(
        r#"
[server]
[llm]
model = "deepseek-chat"
api_key = "test-key"
"#,
    )
    .unwrap();
    assert_eq!(config.bind_address(), "127.0.0.1:3000");
}
