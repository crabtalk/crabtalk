//! Name the other agents in the runtime.
//!
//! The smallest thing that reaches the runtime rather than the machine: one
//! tool, one capability, one `ClientMessage`. It exists to exercise the
//! protocol door end to end — the grant, the decode-time allowlist, and the
//! redaction — with nothing else in the way.
//!
//! It also mirrors real code: `apps/tui/src/repl/delegate.rs` builds the same
//! list from the same message to tell a model which agents it can delegate to.

#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

extern crate alloc;

#[crabtalk_harness_sdk::harness(capabilities = ["protocol:read"])]
mod tools {
    use core::fmt::Write;
    use crabtalk_harness_proto::{ClientMessage, ListAgentsMsg, client_message, server_message};
    use crabtalk_harness_sdk::{Failed, Out, protocol};

    /// List the other agents in this runtime, with their descriptions.
    pub fn peers(_args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let request = ClientMessage {
            msg: Some(client_message::Msg::ListAgents(ListAgentsMsg {})),
        };

        let reply = match protocol::call(request) {
            Ok(reply) => reply,
            Err(error) => {
                out.write(error.as_bytes());
                return Err(Failed);
            }
        };

        let Some(server_message::Msg::AgentList(list)) = reply.msg else {
            out.write(b"the runtime did not return an agent list");
            return Err(Failed);
        };

        for agent in &list.agents {
            let _ = writeln!(out, "{}\t{}", agent.name, agent.description);
        }
        Ok(())
    }
}
