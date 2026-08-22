//! `${VAR}` interpolation for values a config file should not hold in clear.

use anyhow::{Result, bail};

/// Replace every `${VAR}` with the environment's value for `VAR`.
///
/// An unset variable is an error rather than a passthrough: a literal
/// `${...}` forwarded as a bearer token fails upstream as an opaque 401,
/// pointing nowhere near the config that caused it.
pub fn interpolate(value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find("${") {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 2..];
        let Some(close) = tail.find('}') else {
            bail!("unterminated `${{` in a configuration value");
        };
        let name = &tail[..close];
        let Ok(resolved) = std::env::var(name) else {
            bail!("environment variable `{name}` is not set");
        };
        out.push_str(&resolved);
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
