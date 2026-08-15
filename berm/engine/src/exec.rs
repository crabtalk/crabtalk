//! `exec` — commands under the granted root.
//!
//! The result is the same JSON the OS tools have always returned, because its
//! destination is a model that has been reading that shape. A harness passes it
//! through untouched.

use crate::{root, wire};
use anyhow::{Result, bail};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long a command may run. Every capability needs its own timeout: rvtime
/// can interrupt a looping guest, but not a host call of ours that never
/// returns.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How often to check whether the command is done.
const POLL: Duration = Duration::from_millis(5);

/// Run a command. Request: `[command, cwd, key, value, key, value, …]`.
pub fn run(root: &Path, request: &[u8]) -> Result<Vec<u8>> {
    let fields = wire::fields(request)?;
    let command = wire::text(&fields, 0, "command")?;
    let cwd = root::resolve(root, wire::text(&fields, 1, "cwd")?)?;

    let mut process = Command::new("bash");
    process
        .args(["-c", command])
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let environment = &fields[2.min(fields.len())..];
    if environment.len() % 2 != 0 {
        bail!("environment has a key with no value");
    }
    for pair in environment.chunks(2) {
        process.env(
            String::from_utf8_lossy(pair[0]).as_ref(),
            String::from_utf8_lossy(pair[1]).as_ref(),
        );
    }

    let mut child = process.spawn()?;

    // Drained on their own threads for the whole run. Polling the child while
    // leaving the pipes alone deadlocks as soon as a command writes more than
    // one pipe buffer, which is exactly the commands worth running.
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(POLL);
    };

    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();

    let Some(status) = status else {
        bail!("command timed out after {} seconds", TIMEOUT.as_secs());
    };

    Ok(serde_json::to_vec(&serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": status.code().unwrap_or(-1),
    }))?)
}

/// Read a pipe to end on its own thread.
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut pipe) = pipe {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text
    })
}
