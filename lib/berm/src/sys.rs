// The guest half of the `crabtalk` namespace. `crabtalk-berm` declares the host
// half of the same thing, and drift between them hashes to a number nothing is
// registered for — loud on the first call.

berm_lang::harnesses! {
    namespace = "crabtalk";

    /// Files, bounded by a granted root.
    mod fs {
        /// Read a file whole.
        fn read(path: &str) -> Vec<u8>;
        /// Write a file, replacing what was there.
        fn write(path: &str, content: &[u8]);
        /// Paths under `path` matching `pattern`, newest first.
        fn glob(pattern: &str, path: &str) -> Vec<u8>;
        /// Search file contents under `path`. `mode` is `files`, `content` or `count`.
        fn grep(pattern: &str, path: &str, include: &str, mode: &str) -> Vec<u8>;
    }

    /// Commands, under the same root `fs` is bounded by.
    mod exec {
        /// Run a command through a shell, in `cwd` relative to the root.
        fn run(command: &str, cwd: &str, env: &[(&str, &str)]) -> Vec<u8>;
    }

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
