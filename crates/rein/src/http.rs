//! The HTTP part that `rein debug` and `rein watch` share: read one request, answer it, close. Local only: a
//! request must name this server's own address in its Host header.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

/// The largest request body read. Both servers take short requests.
const MAX_BODY: usize = 1 << 20;

/// The answer to one request: status line, content type, body.
pub type Reply = (&'static str, &'static str, String);

/// Reads one request from `stream`, answers it with `respond(method, path, body)`, and closes.
pub fn answer(stream: TcpStream, port: u16, respond: impl Fn(&str, &str, &str) -> Reply) -> std::io::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    exchange(reader, stream, port, respond)
}

/// The request and response of [`answer`], over any reader and writer.
pub fn exchange<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    port: u16,
    respond: impl Fn(&str, &str, &str) -> Reply,
) -> std::io::Result<()> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let path = words.next().unwrap_or_default().to_string();
    let (mut length, mut host) = (0usize, String::new());
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(0);
        }
        if name.eq_ignore_ascii_case("host") {
            host = value.trim().to_string();
        }
    }
    let mut body = vec![0; length.min(MAX_BODY)];
    reader.read_exact(&mut body)?;
    // The query string is for the page's own state. The routes do not read it.
    let route = path.split_once('?').map_or(path.as_str(), |(route, _)| route);
    let (status, kind, text) = if local_host(&host, port) {
        respond(&method, route, &String::from_utf8_lossy(&body))
    } else {
        (
            "403 Forbidden",
            "text/plain",
            "only 127.0.0.1 and localhost may ask this server\n".to_string(),
        )
    };
    write!(
        writer,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{text}",
        text.len()
    )
}

/// The Host header names this server, so a page of another site cannot read it through DNS rebinding.
pub fn local_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}
