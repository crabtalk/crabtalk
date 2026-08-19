//! `http` — requests to the hosts a declaration named.
//!
//! Named `crabtalk.` rather than `berm.` because it is not the machine. A
//! harness holding `exec` already has a shell, and a shell has curl, so an
//! unbounded fetch is nothing berm needs to hand out. What this adds is the
//! bound: a client that goes only where the declaration said, through a pool
//! the daemon already runs.
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

use crate::system::http::Fetch;
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use std::{sync::LazyLock, time::Duration};
use tokio::runtime::Handle;

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

/// What one declaration's `hosts` grant is served by.
///
/// The reactor is held rather than looked up per call: hyper needs one and the
/// sandbox is sync, so the embedder's runtime is part of what this is built
/// with — the same way a root is what `fs` is built with.
pub struct Http {
    hosts: Vec<String>,
    reactor: Handle,
}

impl Http {
    pub fn new(hosts: Vec<String>, reactor: Handle) -> Self {
        Self { hosts, reactor }
    }

    /// The harness serving `crabtalk.http.fetch`, named by the declaration
    /// rather than by a string written here.
    pub fn harness(self) -> berm::Harness {
        crate::system::http::fetch(move |method, url, body, headers| {
            self.fetch(method, url, body, headers)
        })
    }

    /// Perform one request, if the grant names where it is going.
    pub fn fetch(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Fetch> {
        let uri: Uri = url.parse().with_context(|| format!("{url} is not a url"))?;
        let Some(host) = uri.host() else {
            bail!("{url} has no host");
        };
        if !self
            .hosts
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
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Full::new(Bytes::copy_from_slice(body)))?;

        self.reactor.block_on(async {
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
                    .to_bytes()
                    .to_vec();
                Ok::<_, anyhow::Error>(Fetch {
                    status,
                    headers,
                    body,
                })
            })
            .await
            .context("request timed out")?
        })
    }
}
