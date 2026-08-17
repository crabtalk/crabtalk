//! Name the other agents in the runtime.
//!
//! The smallest thing that reaches the runtime rather than the machine: one
//! tool, one capability, one `ClientMessage`. It exists to exercise the
//! protocol door end to end — the grant, the decode-time allowlist, and the
//! redaction — with nothing else in the way.
//!
//! Naming the peers is all it does. Reaching one is a turn spent on another
//! agent's behalf, which is in no group a harness can hold.

#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

extern crate alloc;

#[berm_sdk::harness(capabilities = ["protocol:read"])]
mod tools {
    use berm_sdk::{
        Failed, Out,
        proto::{ClientMessage, ListAgentsMsg, client_message, server_message},
        protocol,
    };
    use core::fmt::Write;

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
