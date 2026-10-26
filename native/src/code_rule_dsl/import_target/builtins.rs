//! The Node built-in module names eslint-plugin-n reads: Node's `isBuiltin` (here, Node 26.5.0's
//! `module.builtinModules`, each also with `node:`, and the `node:`-only ones), which decides a
//! target's module type and keeps built-ins out of the visitors' targets; and the keys of the
//! plugin's `NodeBuiltinModules` table (lib/unsupported-features/node-builtins.js), which
//! `ImportTarget` gives enhanced-resolve as a `false` fallback.

/// Node 26.5.0's `module.isBuiltin` names, sorted.
const NODE_BUILTINS: &[&str] = &[
    "_http_agent", "_http_client", "_http_common", "_http_incoming", "_http_outgoing",
    "_http_server", "_tls_common", "_tls_wrap", "assert", "assert/strict", "async_hooks",
    "buffer", "child_process", "cluster", "console", "constants", "crypto", "dgram",
    "diagnostics_channel", "dns", "dns/promises", "domain", "events", "fs", "fs/promises", "http",
    "http2", "https", "inspector", "inspector/promises", "module", "net", "node:_http_agent",
    "node:_http_client", "node:_http_common", "node:_http_incoming", "node:_http_outgoing",
    "node:_http_server", "node:_tls_common", "node:_tls_wrap", "node:assert",
    "node:assert/strict", "node:async_hooks", "node:buffer", "node:child_process",
    "node:cluster", "node:console", "node:constants", "node:crypto", "node:dgram",
    "node:diagnostics_channel", "node:dns", "node:dns/promises", "node:domain", "node:events",
    "node:fs", "node:fs/promises", "node:http", "node:http2", "node:https", "node:inspector",
    "node:inspector/promises", "node:module", "node:net", "node:os", "node:path",
    "node:path/posix", "node:path/win32", "node:perf_hooks", "node:process", "node:punycode",
    "node:querystring", "node:readline", "node:readline/promises", "node:repl", "node:sea",
    "node:sqlite", "node:stream", "node:stream/consumers", "node:stream/promises",
    "node:stream/web", "node:string_decoder", "node:sys", "node:test", "node:test/reporters",
    "node:timers", "node:timers/promises", "node:tls", "node:trace_events", "node:tty",
    "node:url", "node:util", "node:util/types", "node:v8", "node:vm", "node:wasi",
    "node:worker_threads", "node:zlib", "os", "path", "path/posix", "path/win32", "perf_hooks",
    "process", "punycode", "querystring", "readline", "readline/promises", "repl", "stream",
    "stream/consumers", "stream/promises", "stream/web", "string_decoder", "sys", "timers",
    "timers/promises", "tls", "trace_events", "tty", "url", "util", "util/types", "v8", "vm",
    "wasi", "worker_threads", "zlib",
];

/// The keys of eslint-plugin-n 18.4.1's `NodeBuiltinModules`, in the table's order.
pub(super) const PLUGIN_BUILTINS: &[&str] = &[
    "assert", "node:assert", "assert/strict", "node:assert/strict", "async_hooks",
    "node:async_hooks", "buffer", "node:buffer", "child_process", "node:child_process", "cluster",
    "node:cluster", "console", "node:console", "crypto", "node:crypto", "dgram", "node:dgram",
    "diagnostics_channel", "node:diagnostics_channel", "dns", "node:dns", "dns/promises",
    "node:dns/promises", "domain", "events", "node:events", "fs", "node:fs", "fs/promises",
    "node:fs/promises", "http2", "node:http2", "http", "node:http", "node:_http_agent",
    "node:_http_client", "node:_http_common", "node:_http_incoming", "node:_http_outgoing",
    "node:_http_server", "https", "node:https", "inspector", "node:inspector",
    "inspector/promises", "node:inspector/promises", "module", "node:module", "net", "node:net",
    "os", "node:os", "path", "node:path", "path/posix", "node:path/posix", "path/win32",
    "node:path/win32", "perf_hooks", "node:perf_hooks", "process", "node:process", "punycode",
    "querystring", "node:querystring", "readline", "node:readline", "readline/promises",
    "node:readline/promises", "repl", "node:repl", "node:sea", "stream", "node:stream",
    "stream/promises", "node:stream/promises", "stream/web", "node:stream/web",
    "stream/consumers", "node:stream/consumers", "node:_stream_duplex",
    "node:_stream_passthrough", "node:_stream_readable", "node:_stream_transform",
    "node:_stream_wrap", "node:_stream_writable", "string_decoder", "node:string_decoder",
    "node:sqlite", "node:test", "timers", "node:timers", "timers/promises",
    "node:timers/promises", "tls", "node:tls", "node:_tls_common", "node:_tls_wrap",
    "trace_events", "node:trace_events", "tty", "node:tty", "url", "node:url", "util",
    "node:util", "util/types", "node:util/types", "v8", "node:v8", "vm", "node:vm", "wasi",
    "node:wasi", "worker_threads", "node:worker_threads", "zlib", "node:zlib",
];

/// Node's `module.isBuiltin(name)`.
pub(crate) fn is_builtin(name: &str) -> bool {
    NODE_BUILTINS.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_builtin_list_is_sorted_and_reads_as_node_does() {
        assert!(super::NODE_BUILTINS.windows(2).all(|pair| pair[0] < pair[1]));
        for (name, builtin) in [
            ("fs/", false),
            ("node:fs/", false),
            ("buffer/", false),
            ("node:test", true),
            ("test", false),
            ("node:fs/promises", true),
            ("node:_http_agent", true),
            ("sys", true),
        ] {
            assert_eq!(super::is_builtin(name), builtin, "{name}");
        }
    }
}
