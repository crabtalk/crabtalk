// The guest half of the `crabtalk` namespace. `crabtalk-berm` declares the host
// half of the same thing, the way `berm-lang` and `berm` split berm's own set:
// drift hashes to a number nothing is registered for, loud on the first call.

berm_lang::harnesses! {
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
