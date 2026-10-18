//! Adversarial Mock Server for empirical stress-testing of LightCraft MCP stdio transport.
//! Built using pure Rust std (no external crates required).

use std::io::{self, BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut scenario = "normal".to_string();

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--scenario" && i + 1 < args.len() {
            scenario = args[i + 1].clone();
            i += 2;
        } else {
            i += 1;
        }
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut lines = stdin.lock().lines();

    match scenario.as_str() {
        "handshake_malformed" => {
            if let Some(Ok(_line)) = lines.next() {
                // Return invalid JSON during handshake
                let _ = writeln!(stdout, "{{CORRUPTED_JSON_HANDSHAKE");
                let _ = stdout.flush();
            }
        }
        "handshake_error" => {
            if let Some(Ok(_line)) = lines.next() {
                // Return JSON-RPC error during handshake
                let _ = writeln!(
                    stdout,
                    r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32603,"message":"Handshake rejected: internal server error"}}}}"#
                );
                let _ = stdout.flush();
            }
        }
        _ => {
            // Standard handshake
            if let Some(Ok(_line)) = lines.next() {
                let init_resp = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"lightcraft-adversarial","version":"0.8.0"}}}"#;
                let _ = writeln!(stdout, "{}", init_resp);
                let _ = stdout.flush();
            }

            // Expect notifications/initialized
            if let Some(Ok(_notif)) = lines.next() {
                // Handshake complete
            }

            // Now handle scenario-specific traffic
            match scenario.as_str() {
                "malformed_syntax" => {
                    // 1. Read request 2
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(2);
                        // Emit syntax error line
                        let _ = writeln!(stdout, "<<<RAW CORRUPTED NON-JSON BYTES>>>");
                        let _ = stdout.flush();
                        // Emit truncated JSON
                        let _ = writeln!(stdout, "{{\"jsonrpc\":\"2.0\", \"id\":");
                        let _ = stdout.flush();
                        // Emit response missing ID
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","result":{{"status":"missing_id"}}}}"#
                        );
                        let _ = stdout.flush();
                        // Emit response with invalid jsonrpc version string
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"1.0","id":{},"result":{{"status":"bad_version_accepted"}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }

                    // 2. Read request 3 to verify client didn't crash and is still operational
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(3);
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"status":"recovered_ok"}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }
                }

                "server_errors" => {
                    // Request 2: Method Not Found (-32601)
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(2);
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32601,"message":"Method not found","data":{{"attempted":"unknown_method"}}}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }

                    // Request 3: Tool Execution Error (isError: true)
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(3);
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"isError":true,"content":[{{"type":"text","text":"Tool failed: Out of memory"}}]}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }

                    // Request 4: Custom Error (-32000)
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(4);
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32000,"message":"Custom LightCraft pipeline error"}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }
                }

                "out_of_order" => {
                    // Collect 10 requests
                    let mut req_ids = Vec::new();
                    for _ in 0..10 {
                        if let Some(Ok(line)) = lines.next() {
                            if let Some(id) = extract_id(&line) {
                                req_ids.push(id);
                            }
                        }
                    }

                    // Emit an untracked response ID (99999) first
                    let _ = writeln!(
                        stdout,
                        r#"{{"jsonrpc":"2.0","id":99999,"result":{{"unexpected":true}}}}"#
                    );
                    let _ = stdout.flush();

                    // Emit responses in reverse order
                    for &id in req_ids.iter().rev() {
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"order_received":{}}}}}"#,
                            id, id
                        );
                        let _ = stdout.flush();
                    }
                }

                "server_crash" => {
                    // Read request 2 and crash immediately
                    if let Some(Ok(_line)) = lines.next() {
                        std::process::exit(42);
                    }
                }

                "large_payload" => {
                    // Send 5 MB base64 payload
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(2);
                        let size = 5 * 1024 * 1024; // 5 MB
                        let base64_chunk = "A".repeat(size);

                        let _ = write!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"content":[{{"type":"image","mimeType":"image/jpeg","data":""#,
                            id
                        );
                        let _ = write!(stdout, "{}", base64_chunk);
                        let _ = writeln!(stdout, r#""}}]}}}}"#);
                        let _ = stdout.flush();
                    }
                }

                "oversized_payload" => {
                    // Send 33 MB line (exceeding 32 MB line limit)
                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(2);
                        let size = 33 * 1024 * 1024;
                        let junk = "X".repeat(size);
                        let _ = write!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"data":""#,
                            id
                        );
                        let _ = write!(stdout, "{}", junk);
                        let _ = writeln!(stdout, r#""}}}}"#);
                        let _ = stdout.flush();
                    }
                }

                "stderr_flood" => {
                    // Flood 100 KB to stderr before answering
                    let err_junk =
                        "WARNING: Pipe buffer saturation stress test line\n".repeat(2000);
                    eprint!("{}", err_junk);

                    if let Some(Ok(line)) = lines.next() {
                        let id = extract_id(&line).unwrap_or(2);
                        let _ = writeln!(
                            stdout,
                            r#"{{"jsonrpc":"2.0","id":{},"result":{{"flood_passed":true}}}}"#,
                            id
                        );
                        let _ = stdout.flush();
                    }
                }

                _ => {
                    // Echo response for default
                    while let Some(Ok(line)) = lines.next() {
                        if let Some(id) = extract_id(&line) {
                            let _ = writeln!(
                                stdout,
                                r#"{{"jsonrpc":"2.0","id":{},"result":{{"echo":true}}}}"#,
                                id
                            );
                            let _ = stdout.flush();
                        }
                    }
                }
            }
        }
    }
}

fn extract_id(line: &str) -> Option<u64> {
    if let Some(pos) = line.find("\"id\":") {
        let remainder = &line[pos + 5..].trim_start();
        let end = remainder
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(remainder.len());
        remainder[..end].parse::<u64>().ok()
    } else {
        None
    }
}
