//! Benchmark task definitions — one task per agent primitive.

use serde_json::json;

/// A mock tool exposed by the mock MCP server.
pub struct MockTool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value,
}

/// A scripted response for a mock tool call.
pub struct MockResponse {
    pub tool: &'static str,
    pub output: &'static str,
    pub is_error: bool,
}

/// A benchmark task exercising a specific agent primitive.
pub struct Task {
    pub name: &'static str,
    pub prompt: &'static str,
    pub tools: Vec<MockTool>,
    pub responses: Vec<MockResponse>,
    pub expected_tool_calls: usize,
}

fn string_param(name: &str, desc: &str) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": { name: { "type": "string", "description": desc } },
        "required": [name]
    })
}

/// All 9 benchmark tasks.
pub fn tasks() -> Vec<Task> {
    vec![
        // 1. Tool selection — 10 tools available, only 1 needed.
        Task {
            name: "tool_selection",
            prompt: "What's the weather in Tokyo?",
            tools: vec![
                MockTool {
                    name: "get_weather",
                    description: "Get current weather for a city",
                    parameters: string_param("city", "City name"),
                },
                MockTool {
                    name: "get_time",
                    description: "Get current time in a timezone",
                    parameters: string_param("timezone", "Timezone"),
                },
                MockTool {
                    name: "get_calendar",
                    description: "Get calendar events for a date",
                    parameters: string_param("date", "Date"),
                },
                MockTool {
                    name: "get_news",
                    description: "Get latest news headlines",
                    parameters: string_param("topic", "Topic"),
                },
                MockTool {
                    name: "get_stock",
                    description: "Get stock price",
                    parameters: string_param("symbol", "Ticker symbol"),
                },
                MockTool {
                    name: "translate",
                    description: "Translate text between languages",
                    parameters: string_param("text", "Text to translate"),
                },
                MockTool {
                    name: "calculate",
                    description: "Evaluate a math expression",
                    parameters: string_param("expression", "Math expression"),
                },
                MockTool {
                    name: "search_web",
                    description: "Search the web",
                    parameters: string_param("query", "Search query"),
                },
                MockTool {
                    name: "send_email",
                    description: "Send an email",
                    parameters: string_param("to", "Recipient"),
                },
                MockTool {
                    name: "set_reminder",
                    description: "Set a reminder",
                    parameters: string_param("message", "Reminder text"),
                },
            ],
            responses: vec![MockResponse {
                tool: "get_weather",
                output: "Tokyo: 22°C, sunny, humidity 45%",
                is_error: false,
            }],
            expected_tool_calls: 1,
        },
        // 2. Sequential chain — read config, extract value.
        Task {
            name: "sequential_chain",
            prompt: "Read config.toml and tell me the database host.",
            tools: vec![
                MockTool {
                    name: "read_file",
                    description: "Read a file's contents",
                    parameters: string_param("path", "File path"),
                },
                MockTool {
                    name: "ping",
                    description: "Ping a host",
                    parameters: string_param("host", "Hostname"),
                },
            ],
            responses: vec![
                MockResponse {
                    tool: "read_file",
                    output: "[database]\nhost = \"db.prod.internal\"\nport = 5432\nname = \"crabtalk\"",
                    is_error: false,
                },
                MockResponse {
                    tool: "ping",
                    output: "PING db.prod.internal: 64 bytes, time=2.3ms",
                    is_error: false,
                },
            ],
            expected_tool_calls: 2,
        },
        // 3. Parallel dispatch — check 3 independent services.
        Task {
            name: "parallel_dispatch",
            prompt: "Check the status of the auth, billing, and notifications services.",
            tools: vec![MockTool {
                name: "check_status",
                description: "Check if a service is running",
                parameters: string_param("service", "Service name"),
            }],
            responses: vec![MockResponse {
                tool: "check_status",
                output: "running (pid 1234, uptime 48h)",
                is_error: false,
            }],
            expected_tool_calls: 3,
        },
        // 4. Error recovery — first call fails, agent must adapt.
        Task {
            name: "error_recovery",
            prompt: "Read the file /etc/config.yaml and show me its contents.",
            tools: vec![MockTool {
                name: "read_file",
                description: "Read a file's contents",
                parameters: string_param("path", "File path"),
            }],
            responses: vec![
                MockResponse {
                    tool: "read_file",
                    output: "error: file not found: /etc/config.yaml",
                    is_error: true,
                },
                MockResponse {
                    tool: "read_file",
                    output: "database:\n  host: localhost\n  port: 5432",
                    is_error: false,
                },
            ],
            expected_tool_calls: 2,
        },
        // 5. Conditional branch — different path based on result.
        Task {
            name: "conditional_branch",
            prompt: "If the auth service is running, get its logs. If it's stopped, start it.",
            tools: vec![
                MockTool {
                    name: "check_status",
                    description: "Check if a service is running",
                    parameters: string_param("service", "Service name"),
                },
                MockTool {
                    name: "get_logs",
                    description: "Get recent logs for a service",
                    parameters: string_param("service", "Service name"),
                },
                MockTool {
                    name: "start_service",
                    description: "Start a stopped service",
                    parameters: string_param("service", "Service name"),
                },
            ],
            responses: vec![
                MockResponse {
                    tool: "check_status",
                    output: "running (pid 5678, uptime 12h)",
                    is_error: false,
                },
                MockResponse {
                    tool: "get_logs",
                    output: "[2026-03-31 10:00] auth: 150 requests/sec\n[2026-03-31 10:01] auth: 148 requests/sec",
                    is_error: false,
                },
                MockResponse {
                    tool: "start_service",
                    output: "service auth started (pid 9012)",
                    is_error: false,
                },
            ],
            expected_tool_calls: 2,
        },
        // 6. Aggregation — read multiple files, compare.
        Task {
            name: "aggregation",
            prompt: "Read these 5 files and tell me which one has the most lines: a.txt, b.txt, c.txt, d.txt, e.txt",
            tools: vec![MockTool {
                name: "read_file",
                description: "Read a file's contents",
                parameters: string_param("path", "File path"),
            }],
            responses: vec![
                MockResponse {
                    tool: "read_file",
                    output: "line1\nline2\nline3",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "line1\nline2\nline3\nline4\nline5\nline6\nline7",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "line1\nline2",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "line1\nline2\nline3\nline4",
                    is_error: false,
                },
            ],
            expected_tool_calls: 5,
        },
        // 7. Iteration — test, fix, re-test loop.
        Task {
            name: "iteration",
            prompt: "Run the test suite. If tests fail, fix the error in main.py, then run tests again.",
            tools: vec![
                MockTool {
                    name: "run_tests",
                    description: "Run the test suite",
                    parameters: json!({"type": "object", "properties": {}}),
                },
                MockTool {
                    name: "edit_file",
                    description: "Edit a file",
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path" },
                            "content": { "type": "string", "description": "New content" }
                        },
                        "required": ["path", "content"]
                    }),
                },
            ],
            responses: vec![
                MockResponse {
                    tool: "run_tests",
                    output: "FAIL: test_login (AssertionError: expected 200, got 401)\n1 failed, 4 passed",
                    is_error: false,
                },
                MockResponse {
                    tool: "edit_file",
                    output: "file updated: main.py",
                    is_error: false,
                },
                MockResponse {
                    tool: "run_tests",
                    output: "OK: 5 passed, 0 failed",
                    is_error: false,
                },
            ],
            expected_tool_calls: 3,
        },
        // 8. Context pressure — many tool calls in one session.
        Task {
            name: "context_pressure",
            prompt: "Read all 20 log files (log_01.txt through log_20.txt) and summarize the error counts.",
            tools: vec![MockTool {
                name: "read_file",
                description: "Read a file's contents",
                parameters: string_param("path", "File path"),
            }],
            responses: {
                let mut r = Vec::new();
                for i in 1..=20 {
                    r.push(MockResponse {
                        tool: "read_file",
                        output: match i % 4 {
                            0 => "INFO: all systems nominal\nINFO: health check passed",
                            1 => "ERROR: connection timeout\nWARN: retrying\nINFO: recovered",
                            2 => "ERROR: disk space low\nERROR: write failed\nCRITICAL: service degraded",
                            _ => "INFO: request processed\nINFO: cache hit ratio 94%",
                        },
                        is_error: false,
                    });
                }
                r
            },
            expected_tool_calls: 20,
        },
        // 9. Termination — stop after finding first match.
        Task {
            name: "termination",
            prompt: "Find the first file that contains a TODO comment. Check: main.rs, lib.rs, utils.rs, config.rs, test.rs",
            tools: vec![MockTool {
                name: "read_file",
                description: "Read a file's contents",
                parameters: string_param("path", "File path"),
            }],
            responses: vec![
                MockResponse {
                    tool: "read_file",
                    output: "fn main() {\n    println!(\"hello\");\n}",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "pub fn init() {}\npub fn run() {}",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "// TODO: refactor this function\npub fn helper() {}",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "pub const MAX: usize = 100;",
                    is_error: false,
                },
                MockResponse {
                    tool: "read_file",
                    output: "#[test]\nfn test_basic() { assert!(true); }",
                    is_error: false,
                },
            ],
            expected_tool_calls: 3,
        },
    ]
}
