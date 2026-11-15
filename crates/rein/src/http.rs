//! The HTTP part that `rein debug` and `rein watch` share: read one request, answer it, close. Local only: a
//! request must name this server's own address in its Host header.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The largest request body read. Both servers take short requests.
const MAX_BODY: usize = 1 << 20;

/// How long a client may take to send its request. The servers answer one client at a time, so a silent client
/// must not hold the others.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The answer to one request: status line, content type, body.
pub type Reply = (&'static str, &'static str, String);

/// Reads one request from `stream`, answers it with `respond(method, path, body)`, and closes.
pub fn answer(stream: TcpStream, port: u16, respond: impl Fn(&str, &str, &str) -> Reply) -> std::io::Result<()> {
    answer_within(stream, port, READ_TIMEOUT, respond)
}

fn answer_within(
    stream: TcpStream,
    port: u16,
    timeout: Duration,
    respond: impl Fn(&str, &str, &str) -> Reply,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(timeout))?;
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
            // A number too large for usize is above MAX_BODY, not 0.
            let digits = value.trim();
            let number = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
            length = digits.parse().unwrap_or(if number { usize::MAX } else { 0 });
        }
        if name.eq_ignore_ascii_case("host") {
            host = value.trim().to_string();
        }
    }
    // A body above MAX_BODY is not read: the client gets 413 at once instead of waiting on a body never read.
    let too_large = length > MAX_BODY;
    let mut body = vec![0; if too_large { 0 } else { length }];
    reader.read_exact(&mut body)?;
    // The query string is for the page's own state. The routes do not read it.
    let route = path.split_once('?').map_or(path.as_str(), |(route, _)| route);
    let (status, kind, text) = if local_host(&host, port) && too_large {
        (
            "413 Content Too Large",
            "text/plain",
            format!("the request body is larger than {MAX_BODY} bytes\n"),
        )
    } else if local_host(&host, port) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// An idle client holds the one-at-a-time server for the timeout only. The next client still gets its answer.
    #[test]
    fn a_silent_client_does_not_block_the_next_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                let _ = answer_within(stream, port, Duration::from_millis(200), |_, _, _| {
                    ("200 OK", "text/plain", "answered\n".to_string())
                });
            }
        });
        // Connects and sends nothing.
        let idle = TcpStream::connect(addr).unwrap();
        let mut client = TcpStream::connect(addr).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client
            .write_all(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert!(reply.ends_with("answered\n"), "{reply}");
        server.join().unwrap();
        drop(idle);
    }

    /// A Content-Length above MAX_BODY gets 413 at once, with no body read and no call to `respond`.
    /// This includes a number too large for usize.
    #[test]
    fn an_oversized_body_gets_413_at_once() {
        for length in [(MAX_BODY + 1).to_string(), "99999999999999999999999999".to_string()] {
            let request = format!("POST /answer HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n");
            let mut reply = Vec::new();
            exchange(request.as_bytes(), &mut reply, 1, |_, _, _| panic!("respond must not run")).unwrap();
            let reply = String::from_utf8(reply).unwrap();
            assert!(reply.starts_with("HTTP/1.1 413 Content Too Large"), "{length}: {reply}");
        }
    }
}
