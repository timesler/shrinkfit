//! `shrinkfit serve`: a tiny dependency-free HTTP server for the playground
//! page in `web/index.html`. Local development aid, not a production server:
//! it binds to loopback only and handles one request per connection.

use shrinkfit::{json, shrink_str, Options, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;

const INDEX: &str = include_str!("../web/index.html");
const EXAMPLES_JS: &str = include_str!("../web/examples.js");
const TWG_MANIFEST: &str = include_str!("../web/twg/manifest.json");
// Explicit compile-time assets: the server never reads arbitrary filesystem paths.
const TWG_EXAMPLES: &[(&str, &str)] = &[
    ("jira-search", include_str!("../web/twg/jira-search.json")),
    (
        "jira-all-fields",
        include_str!("../web/twg/jira-all-fields.json"),
    ),
    ("jira-details", include_str!("../web/twg/jira-details.json")),
    (
        "confluence-search",
        include_str!("../web/twg/confluence-search.json"),
    ),
    ("teams", include_str!("../web/twg/teams.json")),
    ("projects", include_str!("../web/twg/projects.json")),
    ("goals", include_str!("../web/twg/goals.json")),
    ("boards", include_str!("../web/twg/boards.json")),
];
const MAX_BODY: usize = 256 * 1024 * 1024;

fn twg_asset(path: &str) -> Option<&'static str> {
    if path == "/twg/manifest.json" {
        return Some(TWG_MANIFEST);
    }
    let id = path.strip_prefix("/twg/")?.strip_suffix(".json")?;
    TWG_EXAMPLES
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, text)| *text)
}

pub const USAGE: &str = "\
usage: shrinkfit serve [--port N] [--host ADDR]

Start the local playground web app (default http://127.0.0.1:8765).
Paste JSON or YAML, tweak every option, and see the shrunk output live.";

pub fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut port: u16 = 8765;
    let mut host = "127.0.0.1".to_string();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-p" | "--port" => {
                port = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--port needs a number")?
            }
            "--host" => host = args.next().ok_or("--host needs an address")?,
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    let listener =
        TcpListener::bind((host.as_str(), port)).map_err(|e| format!("{host}:{port}: {e}"))?;
    println!("shrinkfit playground: http://{host}:{port}/  (Ctrl+C to stop)");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = handle(stream);
        });
    }
    Ok(())
}

fn handle(mut s: TcpStream) -> std::io::Result<()> {
    let (method, path, body) = match read_request(&mut s)? {
        Some(r) => r,
        None => return Ok(()),
    };
    match (method.as_str(), path.split('?').next().unwrap_or("")) {
        ("GET", "/") | ("GET", "/index.html") => {
            respond(&mut s, 200, "text/html; charset=utf-8", INDEX.as_bytes())
        }
        ("GET", "/examples.js") => respond(
            &mut s,
            200,
            "text/javascript; charset=utf-8",
            EXAMPLES_JS.as_bytes(),
        ),
        ("GET", path) if twg_asset(path).is_some() => respond(
            &mut s,
            200,
            "application/json; charset=utf-8",
            twg_asset(path).unwrap().as_bytes(),
        ),
        ("POST", "/api/shrink") => {
            let out = api_shrink(&body);
            respond(&mut s, 200, "application/json", out.as_bytes())
        }
        _ => respond(&mut s, 404, "text/plain", b"not found"),
    }
}

fn read_request(s: &mut TcpStream) -> std::io::Result<Option<(String, String, Vec<u8>)>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(None);
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Ok(None);
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(Some((method, path, body)))
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = if code == 200 { "OK" } else { "Not Found" };
    write!(
        s,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)
}

fn err(msg: String) -> String {
    json::to_string(
        &Value::Object(vec![
            ("ok".into(), Value::Bool(false)),
            ("error".into(), Value::String(msg)),
        ]),
        json::Layout::Compact,
    )
}

/// Request: `{"input": "<text>", "options": {…}}`; response: the same
/// envelope the C ABI returns, so the page exercises the real interface.
fn api_shrink(body: &[u8]) -> String {
    let text = match std::str::from_utf8(body) {
        Ok(t) => t,
        Err(_) => return err("request is not valid UTF-8".into()),
    };
    let req = match json::parse(text) {
        Ok(Value::Object(e)) => e,
        _ => return err("request must be a JSON object".into()),
    };
    let get = |k: &str| req.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    let input = match get("input") {
        Some(Value::String(s)) => s.as_str(),
        _ => return err("missing string field \"input\"".into()),
    };
    let default_opts = Value::Object(Vec::new());
    let opts = match Options::from_value(get("options").unwrap_or(&default_opts)) {
        Ok(o) => o,
        Err(e) => return err(format!("invalid options: {e}")),
    };
    match shrink_str(input, &opts) {
        Ok(out) => json::to_string(
            &Value::Object(vec![
                ("ok".into(), Value::Bool(true)),
                ("text".into(), Value::String(out.text)),
                ("report".into(), out.report.to_value()),
            ]),
            json::Layout::Compact,
        ),
        Err(e) => err(e.to_string()),
    }
}
