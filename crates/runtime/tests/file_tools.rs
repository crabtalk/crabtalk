//! Tests for read and edit tools.

use crabtalk_runtime::{Env, NoHost};
use wcore::repos::mem::InMemoryRepos;

async fn test_env(cwd: std::path::PathBuf) -> Env<NoHost, InMemoryRepos> {
    let repos = InMemoryRepos::new();
    Env::new(repos, cwd, None, NoHost)
}

// --- read ---

#[tokio::test]
async fn read_basic() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    std::fs::write(&file, "line one\nline two\nline three\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(r#"{{"path":"{}"}}"#, file.display());
    let result = hook.dispatch_read(&args, None).await.unwrap();

    assert!(result.contains("1\tline one"));
    assert!(result.contains("2\tline two"));
    assert!(result.contains("3\tline three"));
    assert!(result.contains("--- 3 total lines ---"));
}

#[tokio::test]
async fn read_offset_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lines.txt");
    let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(r#"{{"path":"{}","offset":10,"limit":5}}"#, file.display());
    let result = hook.dispatch_read(&args, None).await.unwrap();

    assert!(result.contains("10\tline 10"));
    assert!(result.contains("14\tline 14"));
    assert!(!result.contains("15\tline 15"));
    assert!(result.contains("showing lines 10-14"));
    assert!(result.contains("100 total lines"));
}

#[tokio::test]
async fn read_missing() {
    let dir = tempfile::tempdir().unwrap();
    let hook = test_env(dir.path().to_path_buf()).await;
    let args = r#"{"path":"/nonexistent/file.txt"}"#;
    let err = hook.dispatch_read(args, None).await.unwrap_err();

    assert!(err.contains("error reading"));
}

#[tokio::test]
async fn read_offset_past_end() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("short.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(r#"{{"path":"{}","offset":999}}"#, file.display());
    let result = hook.dispatch_read(&args, None).await.unwrap();

    assert!(result.contains("past end of file"));
}

#[tokio::test]
async fn read_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rel.txt"), "content\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = r#"{"path":"rel.txt"}"#;
    let result = hook.dispatch_read(args, None).await.unwrap();

    assert!(result.contains("1\tcontent"));
}

#[tokio::test]
async fn read_large_file_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("big.bin");
    // Create a file that exceeds MAX_FILE_SIZE by writing sparse content.
    let f = std::fs::File::create(&file).unwrap();
    f.set_len(51 * 1024 * 1024).unwrap(); // 51 MB

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(r#"{{"path":"{}"}}"#, file.display());
    let err = hook.dispatch_read(&args, None).await.unwrap_err();

    assert!(err.contains("file is too large"));
}

// --- edit ---

#[tokio::test]
async fn edit_basic() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(
        r#"{{"path":"{}","old_string":"hello","new_string":"goodbye"}}"#,
        file.display()
    );
    let result = hook.dispatch_edit(&args, None).await.unwrap();

    assert_eq!(result, "ok");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye world\n");
}

#[tokio::test]
async fn edit_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(
        r#"{{"path":"{}","old_string":"missing","new_string":"x"}}"#,
        file.display()
    );
    let err = hook.dispatch_edit(&args, None).await.unwrap_err();

    assert_eq!(err, "old_string not found");
}

#[tokio::test]
async fn edit_not_unique() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dup.txt");
    std::fs::write(&file, "aaa bbb aaa\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(
        r#"{{"path":"{}","old_string":"aaa","new_string":"ccc"}}"#,
        file.display()
    );
    let err = hook.dispatch_edit(&args, None).await.unwrap_err();

    assert!(err.contains("not unique"));
    assert!(err.contains("2 occurrences"));
}

#[tokio::test]
async fn edit_identical_strings() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("same.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(
        r#"{{"path":"{}","old_string":"hello","new_string":"hello"}}"#,
        file.display()
    );
    let err = hook.dispatch_edit(&args, None).await.unwrap_err();

    assert!(err.contains("identical"));
}

#[tokio::test]
async fn edit_empty_old_string() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let args = format!(
        r#"{{"path":"{}","old_string":"","new_string":"x"}}"#,
        file.display()
    );
    let err = hook.dispatch_edit(&args, None).await.unwrap_err();

    assert!(err.contains("must not be empty"));
}

#[tokio::test]
async fn edit_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let hook = test_env(dir.path().to_path_buf()).await;
    let args = r#"{"path":"/nonexistent/file.txt","old_string":"a","new_string":"b"}"#;
    let err = hook.dispatch_edit(args, None).await.unwrap_err();

    assert!(err.contains("error reading"));
}

#[tokio::test]
async fn file_tools_no_sender_restriction() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("gateway.txt");
    std::fs::write(&file, "test content\n").unwrap();

    let hook = test_env(dir.path().to_path_buf()).await;
    let config = wcore::AgentConfig::new("agent");
    hook.register_scope("agent".to_owned(), &config);

    // read works for gateway senders.
    let args = format!(r#"{{"path":"{}"}}"#, file.display());
    let result = hook
        .dispatch_tool("read", &args, "agent", "gateway:telegram", None)
        .await
        .unwrap();
    assert!(result.contains("1\ttest content"));

    // edit works for gateway senders.
    let args = format!(
        r#"{{"path":"{}","old_string":"test","new_string":"real"}}"#,
        file.display()
    );
    let result = hook
        .dispatch_tool("edit", &args, "agent", "gateway:telegram", None)
        .await
        .unwrap();
    assert_eq!(result, "ok");

    // bash is blocked for gateway senders.
    let err = hook
        .dispatch_tool(
            "bash",
            r#"{"command":"echo hi"}"#,
            "agent",
            "gateway:telegram",
            None,
        )
        .await
        .unwrap_err();
    assert!(err.contains("only available in the command line interface"));
}
