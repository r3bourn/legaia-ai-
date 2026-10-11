use anyhow::{Context, Result, anyhow, bail};
use legaia_ai_broker::{DecisionRequest, ProviderConfig, decide};
use legaia_ai_controller::TelemetrySnapshot;
use legaia_ai_orchestrator::{Directive, OrchestrationStep, Orchestrator};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const DEFAULT_BIND: &str = "127.0.0.1:4317";
const MAX_REQUEST_BYTES: usize = 512 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;

fn main() {
    if let Err(err) = run() {
        eprintln!("legaia-ai-broker: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match env::args().nth(1).as_deref() {
        None => run_stdin_once(),
        Some("serve") => run_server(),
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(other) => bail!("unknown command `{other}`; run with --help"),
    }
}

fn print_help() {
    println!(
        "legaia-ai-broker\n\
         \n\
         Usage:\n\
           legaia-ai-broker            Read one DecisionRequest JSON from stdin\n\
           legaia-ai-broker serve      Run the local HTTP decision/orchestration service\n\
         \n\
         Server environment:\n\
           LEGAIA_AI_BIND              Bind address (default {DEFAULT_BIND})\n\
           LEGAIA_AI_BEARER_TOKEN      Required when binding a non-loopback address\n\
           LEGAIA_AI_CORS_ORIGIN       Optional exact Access-Control-Allow-Origin value\n\
           LEGAIA_GUIDE_PATH           Optional local YAML/JSON checkpoint guide\n"
    );
}

fn run_stdin_once() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read decision request from stdin")?;

    let request: DecisionRequest = serde_json::from_str(&input)
        .context("stdin must contain one DecisionRequest JSON object")?;

    let config = ProviderConfig::from_env()?;
    let decision = decide(&config, &request)?;

    println!("{}", serde_json::to_string(&decision)?);
    Ok(())
}

fn run_server() -> Result<()> {
    let bind = env::var("LEGAIA_AI_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid LEGAIA_AI_BIND `{bind}`"))?;
    let bearer = env::var("LEGAIA_AI_BEARER_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let mut orchestrator = load_orchestrator()?;

    if !addr.ip().is_loopback() && bearer.is_none() {
        bail!("refusing to bind AI broker to non-loopback {addr} without LEGAIA_AI_BEARER_TOKEN");
    }

    let listener =
        TcpListener::bind(addr).with_context(|| format!("failed to bind AI broker to {addr}"))?;
    eprintln!(
        "legaia-ai-broker: listening on http://{addr} (guide={})",
        if orchestrator.is_some() {
            "loaded"
        } else {
            "none"
        }
    );

    for incoming in listener.incoming() {
        match incoming {
            Ok(mut stream) => {
                if let Err(err) =
                    handle_connection(&mut stream, bearer.as_deref(), &mut orchestrator)
                {
                    eprintln!("legaia-ai-broker: request error: {err:#}");
                    let _ = write_json_error(&mut stream, 500, "internal broker error");
                }
            }
            Err(err) => eprintln!("legaia-ai-broker: accept error: {err}"),
        }
    }

    Ok(())
}

fn load_orchestrator() -> Result<Option<Orchestrator>> {
    let Some(path) = env::var("LEGAIA_GUIDE_PATH")
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        return Ok(None);
    };

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read LEGAIA_GUIDE_PATH `{path}`"))?;
    let orchestrator = if Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        Orchestrator::from_json(&text)
    } else {
        Orchestrator::from_yaml(&text)
    }
    .with_context(|| format!("failed to load checkpoint guide `{path}`"))?;

    Ok(Some(orchestrator))
}

fn handle_connection(
    stream: &mut TcpStream,
    bearer: Option<&str>,
    orchestrator: &mut Option<Orchestrator>,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .context("set read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(60)))
        .context("set write timeout")?;

    let request = read_http_request(stream)?;

    if request.method == "OPTIONS" {
        return write_response(stream, 204, "text/plain", b"");
    }

    if request.method == "GET" && request.path == "/health" {
        let body = serde_json::to_vec(&serde_json::json!({
            "ok": true,
            "guide_loaded": orchestrator.is_some(),
        }))?;
        return write_response(stream, 200, "application/json", &body);
    }

    if let Some(expected) = bearer {
        let got = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.as_str());
        let want = format!("Bearer {expected}");
        if got != Some(want.as_str()) {
            return write_json_error(stream, 401, "unauthorized");
        }
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/decision") => handle_decision(stream, &request.body),
        ("POST", "/guide") => handle_guide(stream, &request.body, orchestrator),
        ("POST", "/step") => handle_step(stream, &request.body, orchestrator),
        ("POST", "/guide/advance") => handle_guide_advance(stream, orchestrator),
        _ => write_json_error(stream, 404, "not found"),
    }
}

fn handle_decision(stream: &mut TcpStream, body: &[u8]) -> Result<()> {
    let decision_request: DecisionRequest = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(err) => {
            return write_json_error(stream, 400, &format!("invalid decision JSON: {err}"));
        }
    };

    let config = ProviderConfig::from_env()?;
    let decision = decide(&config, &decision_request)?;
    let body = serde_json::to_vec(&decision)?;
    write_response(stream, 200, "application/json", &body)
}

fn parse_telemetry(body: &[u8]) -> std::result::Result<TelemetrySnapshot, serde_json::Error> {
    serde_json::from_slice(body)
}

fn guide_step(
    body: &[u8],
    orchestrator: &mut Option<Orchestrator>,
) -> std::result::Result<OrchestrationStep, String> {
    let state = parse_telemetry(body).map_err(|e| format!("invalid telemetry JSON: {e}"))?;
    let Some(orchestrator) = orchestrator.as_mut() else {
        return Err("no checkpoint guide is loaded; set LEGAIA_GUIDE_PATH".into());
    };
    orchestrator
        .step(&state)
        .map_err(|e| format!("guide orchestration failed: {e:#}"))
}

fn handle_guide(
    stream: &mut TcpStream,
    body: &[u8],
    orchestrator: &mut Option<Orchestrator>,
) -> Result<()> {
    if orchestrator.is_none() {
        return write_json_error(
            stream,
            503,
            "no checkpoint guide is loaded; set LEGAIA_GUIDE_PATH",
        );
    }
    let step = match guide_step(body, orchestrator) {
        Ok(step) => step,
        Err(err) => return write_json_error(stream, 400, &err),
    };
    let body = serde_json::to_vec(&step)?;
    write_response(stream, 200, "application/json", &body)
}

fn handle_step(
    stream: &mut TcpStream,
    body: &[u8],
    orchestrator: &mut Option<Orchestrator>,
) -> Result<()> {
    if orchestrator.is_none() {
        return write_json_error(
            stream,
            503,
            "no checkpoint guide is loaded; set LEGAIA_GUIDE_PATH",
        );
    }
    let state = match parse_telemetry(body) {
        Ok(state) => state,
        Err(err) => {
            return write_json_error(stream, 400, &format!("invalid telemetry JSON: {err}"));
        }
    };
    let step = match orchestrator.as_mut().expect("checked").step(&state) {
        Ok(step) => step,
        Err(err) => {
            return write_json_error(stream, 400, &format!("guide orchestration failed: {err:#}"));
        }
    };

    let decision = if let Directive::NeedDecision {
        objective,
        walkthrough_context,
        ..
    } = &step.directive
    {
        let request = DecisionRequest {
            objective: objective.clone(),
            state: serde_json::to_value(&state)?,
            walkthrough_context: Some(walkthrough_context.clone()),
        };
        Some(decide(&ProviderConfig::from_env()?, &request)?)
    } else {
        None
    };

    let body = serde_json::to_vec(&serde_json::json!({
        "guide": step,
        "decision": decision,
    }))?;
    write_response(stream, 200, "application/json", &body)
}

fn handle_guide_advance(
    stream: &mut TcpStream,
    orchestrator: &mut Option<Orchestrator>,
) -> Result<()> {
    let Some(orchestrator) = orchestrator.as_mut() else {
        return write_json_error(
            stream,
            503,
            "no checkpoint guide is loaded; set LEGAIA_GUIDE_PATH",
        );
    };
    let advanced = orchestrator.manual_advance();
    let body = serde_json::to_vec(&serde_json::json!({
        "advanced": advanced,
        "checkpoint_index": orchestrator.checkpoint_index(),
        "finished": orchestrator.is_finished(),
    }))?;
    write_response(stream, 200, "application/json", &body)
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::with_capacity(4096);
    let mut buf = [0u8; 4096];
    let header_end;

    loop {
        let n = stream.read(&mut buf).context("read HTTP request")?;
        if n == 0 {
            bail!("connection closed before complete HTTP headers");
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() > MAX_REQUEST_BYTES {
            bail!("HTTP request exceeds {MAX_REQUEST_BYTES} byte limit");
        }
        if let Some(pos) = find_bytes(&bytes, b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            bail!("HTTP headers exceed {MAX_HEADER_BYTES} byte limit");
        }
    }

    let head = std::str::from_utf8(&bytes[..header_end - 4])
        .context("HTTP headers are not valid UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or_else(|| anyhow!("missing HTTP method"))?;
    let path = parts.next().ok_or_else(|| anyhow!("missing HTTP path"))?;
    let version = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP version"))?;
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        bail!("unsupported HTTP request line");
    }

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("malformed HTTP header"))?;
        let name = name.trim().to_string();
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().context("invalid Content-Length")?;
        }
        headers.push((name, value));
    }

    if content_length > MAX_REQUEST_BYTES.saturating_sub(header_end) {
        bail!("HTTP body exceeds request size limit");
    }

    // Own the request-line fields before the body read grows `bytes`.
    let method = method.to_string();
    let path = path.split('?').next().unwrap_or(path).to_string();

    let target_len = header_end + content_length;
    while bytes.len() < target_len {
        let want = (target_len - bytes.len()).min(buf.len());
        let n = stream
            .read(&mut buf[..want])
            .context("read HTTP request body")?;
        if n == 0 {
            bail!("connection closed before complete HTTP body");
        }
        bytes.extend_from_slice(&buf[..n]);
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[header_end..target_len].to_vec(),
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn write_json_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    let body = serde_json::to_vec(&serde_json::json!({ "error": message }))?;
    write_response(stream, status, "application/json", &body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Response",
    };
    let cors = env::var("LEGAIA_AI_CORS_ORIGIN")
        .ok()
        .filter(|v| !v.contains('\r') && !v.contains('\n'));

    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    )?;
    if let Some(origin) = cors {
        write!(
            stream,
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nVary: Origin\r\n"
        )?;
    }
    write!(stream, "\r\n")?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_finder_handles_http_separator() {
        assert_eq!(find_bytes(b"abc\r\n\r\ndef", b"\r\n\r\n"), Some(3));
        assert_eq!(find_bytes(b"abc", b"\r\n\r\n"), None);
    }

    #[test]
    fn default_bind_is_loopback() {
        let addr: SocketAddr = DEFAULT_BIND.parse().unwrap();
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn telemetry_parser_rejects_non_snapshot_json() {
        assert!(parse_telemetry(br#"{"x":1}"#).is_err());
    }
}
