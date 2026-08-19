//! Skills as a harness: discovery, matching, and loading.
//!
//! It reaches the runtime and never the machine. Where skill files live is the
//! daemon's business — packages install them, and `resolve_dirs` walks them —
//! so this harness asks over the protocol rather than holding a read grant
//! spanning the config and home directories to find them itself.
//!
//! The catalogue is not injected into the system prompt. Listing costs a tool
//! call when the model wants one, rather than a tax on every request that
//! grows with the number of skills installed.

// `no_std` and `no_main` are the harness's shape. Off its target this is an
// ordinary binary so `cargo test` can run the tools below natively.
#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

extern crate alloc;

// A skill body is prose meant for a model to follow and is the whole payload
// of this harness — instructions truncated halfway are worse than no skill at
// all, so this needs the same room the OS harness's reads do.
#[berm_lang::harness(buffer = 262144)]
mod tools {
    use alloc::{string::String, vec::Vec};
    use berm_lang::{
        Failed, Out, parse,
        proto::{ClientMessage, GetSkillMsg, ListSkillsMsg, client_message, server_message},
        protocol,
    };
    use core::fmt::Write;

    /// Load a skill by name, or list what is available.
    ///
    /// An exact name returns that skill's instructions. Anything else lists
    /// the catalogue — every skill when the name is empty, and the ones whose
    /// name or description mentions it otherwise.
    #[args(Skill)]
    pub fn skill(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let input: Skill = parse(args, out)?;
        let wanted = input.name.trim();

        if !wanted.is_empty() {
            // An exact name is a load. The listing below is what answers when
            // it is not one, so a miss costs the model one call rather than
            // an error it has to recover from.
            if let Some(body) = load(wanted) {
                out.write(body.as_bytes());
                return Ok(());
            }
        }

        let catalogue = list(out)?;
        if catalogue.is_empty() {
            out.write(b"no skills are available to this agent");
            return Ok(());
        }

        let matches: Vec<&(String, String)> = catalogue
            .iter()
            .filter(|(name, description)| mentions(name, description, wanted))
            .collect();

        if matches.is_empty() {
            // Not a failure: the model asked a reasonable question and the
            // answer is no. Naming the catalogue size lets it widen the query
            // rather than conclude skills are broken.
            let _ = write!(
                out,
                "no skill matches {wanted:?}. Call this tool with an empty name to see all {}.",
                catalogue.len()
            );
            return Ok(());
        }

        if wanted.is_empty() {
            let _ = writeln!(out, "Available skills:");
        } else {
            let _ = writeln!(out, "No skill is named {wanted:?}. Closest matches:");
        }
        for (name, description) in matches {
            if description.is_empty() {
                let _ = writeln!(out, "- {name}");
            } else {
                let _ = writeln!(out, "- {name}: {description}");
            }
        }
        Ok(())
    }

    /// Arguments for `skill`.
    pub struct Skill {
        /// Skill name to load. Leave empty to list every available skill.
        #[serde(default)]
        pub name: String,
    }

    /// Whether a catalogue entry answers to `wanted`. An empty `wanted` is a
    /// request for the whole catalogue, so everything answers.
    fn mentions(name: &str, description: &str, wanted: &str) -> bool {
        if wanted.is_empty() {
            return true;
        }
        contains_ignoring_case(name, wanted) || contains_ignoring_case(description, wanted)
    }

    /// `str::contains`, case-insensitively, without allocating a lowercased
    /// copy of every description on every call.
    fn contains_ignoring_case(haystack: &str, needle: &str) -> bool {
        let haystack: Vec<u8> = haystack.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let needle: Vec<u8> = needle.bytes().map(|b| b.to_ascii_lowercase()).collect();
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// One skill's instructions, or `None` if the runtime has no such skill.
    fn load(name: &str) -> Option<String> {
        let request = ClientMessage {
            msg: Some(client_message::Msg::GetSkill(GetSkillMsg {
                name: String::from(name),
            })),
        };
        match protocol::call(request).ok()?.msg {
            Some(server_message::Msg::SkillBody(skill)) => Some(skill.body),
            _ => None,
        }
    }

    /// Every skill the runtime knows, as `(name, description)`.
    fn list(out: &mut Out) -> Result<Vec<(String, String)>, Failed> {
        let request = ClientMessage {
            msg: Some(client_message::Msg::ListSkills(ListSkillsMsg {})),
        };

        let reply = match protocol::call(request) {
            Ok(reply) => reply,
            Err(error) => {
                out.write(error.as_bytes());
                return Err(Failed);
            }
        };

        let Some(server_message::Msg::SkillList(list)) = reply.msg else {
            out.write(b"the runtime did not return a skill list");
            return Err(Failed);
        };

        Ok(list
            .skills
            .into_iter()
            .map(|skill| (skill.name, skill.description))
            .collect())
    }
}
