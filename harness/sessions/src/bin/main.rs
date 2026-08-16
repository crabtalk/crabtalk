//! Session search as a harness.
//!
//! This was a `Hook` compiled into the daemon, which meant the daemon owned
//! both the index *and* how a hit is worded to a model. The index is properly
//! its own — sessions are its state, and it already keeps them ranked — but
//! the wording is policy, and policy belongs somewhere it can be forked.
//!
//! So the split is: the daemon answers `SearchSessions` over the protocol,
//! where any client can ask it too, and this formats the answer. It holds no
//! storage grant and never sees a session file. Asking is narrower than
//! reading, and the daemon keeps the query shape, the caps, and the redaction.
//!
//! Which conversations it may ask about is not this harness's decision: the
//! host overwrites the agent filter with whoever declared it.

// `no_std` and `no_main` are the guest's shape. Off its target this is an
// ordinary binary so `cargo test` can run the tools below natively.
#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

extern crate alloc;

// Twenty hits, each with a window of messages truncated at 1 KiB, is what the
// daemon will return at full stretch. An excerpt cut in half is worse than a
// missing one — the model would cite it anyway — so the buffer is sized for
// the answer rather than for the common case.
#[berm_sdk::harness(
    capabilities = ["protocol:sessions"],
    usage_file = "usage.md",
    buffer = 262144
)]
mod tools {
    use alloc::string::String;
    use berm_sdk::{
        Failed, Out, parse,
        proto::{ClientMessage, SearchSessionsMsg, SessionHit, client_message, server_message},
        protocol,
    };
    use core::fmt::Write;

    /// Search past conversations by keyword.
    ///
    /// Returns ranked excerpts — the matched message plus surrounding
    /// context — never whole sessions. Use a hit's session handle to drill in.
    #[args(SearchSessions)]
    pub fn search_sessions(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let input: SearchSessions = parse(args, out)?;

        let request = ClientMessage {
            msg: Some(client_message::Msg::SearchSessions(SearchSessionsMsg {
                query: input.query,
                limit: input.limit,
                context_before: input.context_before,
                context_after: input.context_after,
                // Whatever goes here is replaced by the host with the agent
                // that declared this harness.
                agent: String::new(),
                sender: input.sender,
            })),
        };

        let reply = match protocol::call(request) {
            Ok(reply) => reply,
            Err(error) => {
                out.write(error.as_bytes());
                return Err(Failed);
            }
        };

        let Some(server_message::Msg::SessionHits(list)) = reply.msg else {
            out.write(b"the runtime did not return session hits");
            return Err(Failed);
        };

        if list.hits.is_empty() {
            out.write(b"no sessions found");
            return Ok(());
        }

        for (at, hit) in list.hits.iter().enumerate() {
            if at > 0 {
                let _ = writeln!(out, "---");
            }
            write_hit(out, hit);
        }
        Ok(())
    }

    /// Arguments for `search_sessions`.
    pub struct SearchSessions {
        /// Keyword or phrase to match against message content. Short queries
        /// of two to six words work best.
        pub query: String,
        /// Maximum number of session hits. Defaults to 5, capped at 20.
        #[serde(default)]
        pub limit: Option<u32>,
        /// Messages to include before each match. Defaults to 4.
        #[serde(default)]
        pub context_before: Option<u32>,
        /// Messages to include after each match. Defaults to 4.
        #[serde(default)]
        pub context_after: Option<u32>,
        /// Restrict to conversations with this sender. Empty means all of
        /// them.
        #[serde(default)]
        pub sender: String,
    }

    fn write_hit(out: &mut Out, hit: &SessionHit) {
        let title = if hit.title.is_empty() {
            "(untitled)"
        } else {
            &hit.title
        };
        let _ = writeln!(out, "## {title}");
        let _ = writeln!(
            out,
            "session: {} · agent: {} · sender: {}",
            hit.session_handle, hit.agent, hit.sender
        );
        let _ = writeln!(
            out,
            "updated: {} · matched message #{}",
            hit.updated_at, hit.msg_idx
        );

        for item in &hit.window {
            let _ = writeln!(
                out,
                "- [{} #{}] {}{}",
                label(&item.role, &item.tool_name),
                item.msg_idx,
                item.snippet,
                if item.truncated { " …" } else { "" },
            );
        }
    }

    /// The protocol carries a role and a tool name; which of them leads is a
    /// presentation choice, and this is where presentation lives.
    fn label(role: &str, tool: &str) -> String {
        let mut label = String::new();
        match (role, tool.is_empty()) {
            ("assistant", false) => {
                let _ = write!(label, "tool-call:{tool}");
            }
            ("tool", false) => {
                let _ = write!(label, "tool:{tool}");
            }
            _ => label.push_str(role),
        }
        label
    }
}
