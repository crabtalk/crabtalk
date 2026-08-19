//! `http` — requests to the hosts a declaration named.
//!
//! This lives here rather than in berm because hyper needs a reactor and the
//! engine has neither one nor a dependency that wants one. `fs` and `exec` are
//! about the machine and ship with the sandbox; this is about a client the
//! host already runs, so it arrives the way any embedder's own does.
//!
//! The allowlist is the whole of the confinement, and it is checked per
//! request rather than per connection. hyper's client does not follow
//! redirects, which makes that sufficient instead of merely likely: a 3xx
//! comes back to the harness as a 3xx, and following it means another `fetch`
//! that is checked exactly like the first. There is no path where the host
//! chases a `Location` to somewhere the declaration never named.
//!
//! What this does not defend against is a granted name resolving somewhere
//! unexpected — the check is on the URL's host, not on the address it
//! resolves to.

use anyhow::{Context, Result, bail};
use berm::wire;
use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use std::{sync::LazyLock, time::Duration};

/// What the harness calls it. `crabtalk.` rather than `berm.` for the reason
/// [`crate::protocol::CALL`] is: the name says who implements it.
pub(crate) const FETCH: &str = "crabtalk.http.fetch";

/// Cap on a response body. A harness reads one through its own heap, so an
/// unbounded body is an unbounded allocation inside the sandbox — the same
/// reason `berm`'s `fs` refuses an oversized file.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Total round trip. A harness blocked in a host call cannot notice an
/// interrupt until the call returns, so berm's watchdog is set to outlast the
/// longest one — this stays within that bound rather than moving it.
const TIMEOUT: Duration = Duration::from_secs(30);

type Connector = hyper_tls::HttpsConnector<HttpConnector>;

/// One pooled client for every harness. An invocation is ~17µs and a TLS
/// handshake is orders of magnitude more, so a client per call would make the
/// sandbox's cost the rounding error. Nothing agent-specific is kept in it —
/// hyper has no cookie store, and what a harness sends is on the request.
static CLIENT: LazyLock<Client<Connector, Full<Bytes>>> =
    LazyLock::new(|| Client::builder(TokioExecutor::new()).build(Connector::new()));

/// Perform one request, if `hosts` names where it is going.
///
/// Request: `[method, url, body, name, value, name, value, …]`.
/// Reply: `[status, headers, body]`, the status as decimal text and the
/// headers as `name: value` lines. The body stays bytes: it is HTML or JSON
/// far more often than it is UTF-8 anyone verified.
pub fn call(hosts: &[String], request: &[u8]) -> Result<Vec<u8>> {
    let fields = wire::fields(request)?;
    let method = wire::text(&fields, 0, "method")?;
    let url = wire::text(&fields, 1, "url")?;
    let body = fields.get(2).copied().unwrap_or_default();

    let uri: Uri = url.parse().with_context(|| format!("{url} is not a url"))?;
    let Some(host) = uri.host() else {
        bail!("{url} has no host");
    };
    if !hosts
        .iter()
        .any(|granted| granted.eq_ignore_ascii_case(host))
    {
        bail!("{host} is not a host this harness was granted");
    }

    // hyper-util's legacy client does not reliably populate `Host` for
    // HTTP/1.1, and servers built on hyper reject a request without it.
    let authority = match uri.port_u16() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(&uri)
        .header("host", &authority);

    let headers = &fields[3.min(fields.len())..];
    if headers.len() % 2 != 0 {
        bail!("a header has no value");
    }
    for pair in headers.chunks(2) {
        builder = builder.header(pair[0], pair[1]);
    }
    let request = builder.body(Full::new(Bytes::copy_from_slice(body)))?;

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow::anyhow!("http needs a running reactor"))?;
    let (status, headers, body) = handle.block_on(async {
        tokio::time::timeout(TIMEOUT, async {
            let response = CLIENT.request(request).await.context("request failed")?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    format!("{name}: {}\n", String::from_utf8_lossy(value.as_bytes()))
                })
                .collect::<String>();
            let body = Limited::new(response.into_body(), MAX_BODY)
                .collect()
                .await
                .map_err(|e| anyhow::anyhow!("could not read the response body: {e}"))?
                .to_bytes();
            Ok::<_, anyhow::Error>((status, headers, body))
        })
        .await
        .context("request timed out")?
    })?;

    Ok(wire::frame(&[
        status.to_string().as_bytes(),
        headers.as_bytes(),
        &body,
    ]))
}
