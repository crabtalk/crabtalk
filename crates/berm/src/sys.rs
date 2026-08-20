//! The host half of the `crabtalk` namespace: one constructor per name, taking
//! the implementation and returning the [`berm::Harness`] that serves it.
//!
//! `berm-crabtalk` declares the guest half of the same thing in its own crate.

berm::hosts! {
    namespace = "crabtalk";

    /// The runtime, as a harness sees it.
    mod protocol {
        /// Send one encoded `ClientMessage`; the reply is an encoded `ServerMessage`.
        fn call(message: &[u8]) -> Vec<u8>;
    }

    /// Requests to the hosts a declaration named.
    mod http {
        /// Perform one request. The body stays bytes: a response is HTML or
        /// JSON far more often than it is UTF-8 anyone verified.
        fn fetch(
            method: &str,
            url: &str,
            body: &[u8],
            headers: &[(&str, &str)],
        ) -> (status: u16, headers: String, body: Vec<u8>);
    }
}
