//! Tests for read, edit, and bash tool handlers.

use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tools::os;
use wcore::ToolDispatch;

fn no_cwds() -> runtime::ConversationCwds {
    Arc::new(Mutex::new(HashMap::new()))
}

fn dispatch(args: &str) -> ToolDispatch {
    ToolDispatch {
        args: args.to_owned(),
        agent: "agent".into(),
        sender: String::new(),
        conversation_id: None,
    }
}

// --- read ---

#[tokio::test]
async fn read_basic() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    std::fs::write(&file, "line one\nline two\nline three\n").unwrap();

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(r#"{{"path":"{}"}}"#, file.display());
    let result = handler(dispatch(&args)).await.unwrap();

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

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(r#"{{"path":"{}","offset":10,"limit":5}}"#, file.display());
    let result = handler(dispatch(&args)).await.unwrap();

    assert!(result.contains("10\tline 10"));
    assert!(result.contains("14\tline 14"));
    assert!(!result.contains("15\tline 15"));
    assert!(result.contains("showing lines 10-14"));
    assert!(result.contains("100 total lines"));
}

#[tokio::test]
async fn read_missing() {
    let entry = os::read(PathBuf::from("/tmp"), no_cwds());
    let handler = &entry.handler;
    let args = r#"{"path":"/nonexistent/file.txt"}"#;
    let err = handler(dispatch(args)).await.unwrap_err();
    assert!(err.contains("error reading"));
}

#[tokio::test]
async fn read_offset_past_end() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("short.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(r#"{{"path":"{}","offset":999}}"#, file.display());
    let result = handler(dispatch(&args)).await.unwrap();
    assert!(result.contains("past end of file"));
}

#[tokio::test]
async fn read_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rel.txt"), "content\n").unwrap();

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let result = handler(dispatch(r#"{"path":"rel.txt"}"#)).await.unwrap();
    assert!(result.contains("1\tcontent"));
}

#[tokio::test]
async fn read_large_file_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("big.bin");
    let f = std::fs::File::create(&file).unwrap();
    f.set_len(51 * 1024 * 1024).unwrap();

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(r#"{{"path":"{}"}}"#, file.display());
    let err = handler(dispatch(&args)).await.unwrap_err();
    assert!(err.contains("file is too large"));
}

// --- edit ---

#[tokio::test]
async fn edit_basic() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    let entry = os::edit(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(
        r#"{{"path":"{}","old_string":"hello","new_string":"goodbye"}}"#,
        file.display()
    );
    let result = handler(dispatch(&args)).await.unwrap();
    assert_eq!(result, "ok");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye world\n");
}

#[tokio::test]
async fn edit_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    let entry = os::edit(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(
        r#"{{"path":"{}","old_string":"missing","new_string":"x"}}"#,
        file.display()
    );
    let err = handler(dispatch(&args)).await.unwrap_err();
    assert_eq!(err, "old_string not found");
}

#[tokio::test]
async fn edit_not_unique() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dup.txt");
    std::fs::write(&file, "aaa bbb aaa\n").unwrap();

    let entry = os::edit(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(
        r#"{{"path":"{}","old_string":"aaa","new_string":"ccc"}}"#,
        file.display()
    );
    let err = handler(dispatch(&args)).await.unwrap_err();
    assert!(err.contains("not unique"));
    assert!(err.contains("2 occurrences"));
}

#[tokio::test]
async fn edit_identical_strings() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("same.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let entry = os::edit(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(
        r#"{{"path":"{}","old_string":"hello","new_string":"hello"}}"#,
        file.display()
    );
    let err = handler(dispatch(&args)).await.unwrap_err();
    assert!(err.contains("identical"));
}

#[tokio::test]
async fn edit_empty_old_string() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let entry = os::edit(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let args = format!(
        r#"{{"path":"{}","old_string":"","new_string":"x"}}"#,
        file.display()
    );
    let err = handler(dispatch(&args)).await.unwrap_err();
    assert!(err.contains("must not be empty"));
}

#[tokio::test]
async fn edit_missing_file() {
    let entry = os::edit(PathBuf::from("/tmp"), no_cwds());
    let handler = &entry.handler;
    let args = r#"{"path":"/nonexistent/file.txt","old_string":"a","new_string":"b"}"#;
    let err = handler(dispatch(args)).await.unwrap_err();
    assert!(err.contains("error reading"));
}

// --- sender restrictions ---

#[tokio::test]
async fn bash_rejected_for_gateway_sender() {
    let entry = os::bash(PathBuf::from("/tmp"), no_cwds());
    let handler = &entry.handler;
    let call = ToolDispatch {
        args: r#"{"command":"echo hi"}"#.to_owned(),
        agent: "agent".into(),
        sender: "gateway:telegram".into(),
        conversation_id: None,
    };
    let err = handler(call).await.unwrap_err();
    assert!(err.contains("only available in the command line interface"));
}

#[tokio::test]
async fn read_allowed_for_gateway_sender() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("gateway.txt");
    std::fs::write(&file, "test content\n").unwrap();

    let entry = os::read(dir.path().to_path_buf(), no_cwds());
    let handler = &entry.handler;
    let call = ToolDispatch {
        args: format!(r#"{{"path":"{}"}}"#, file.display()),
        agent: "agent".into(),
        sender: "gateway:telegram".into(),
        conversation_id: None,
    };
    let result = handler(call).await.unwrap();
    assert!(result.contains("1\ttest content"));
}
