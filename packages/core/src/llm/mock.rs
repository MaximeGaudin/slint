//! A stand-in provider, for tests: a local TCP server that answers every request with the same
//! canned HTTP response, or accepts the connection and never answers at all. No external network
//! is touched — everything runs over 127.0.0.1.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(crate) struct MockServer {
    pub address: std::net::SocketAddr,
    pub requests: Arc<AtomicUsize>,
    /// How many requests were being served at once, at the busiest moment.
    pub max_in_flight: Arc<AtomicUsize>,
}

impl MockServer {
    /// Starts the server. When `hang` is set, connections are accepted and never answered, which
    /// is what a wedged provider looks like from the outside.
    pub fn start(response: String, hang: bool) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let in_flight = Arc::new(AtomicUsize::new(0));

        let (requests_shared, max_in_flight_shared, in_flight_shared) =
            (requests.clone(), max_in_flight.clone(), in_flight.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let response = response.clone();
                let (requests, max_in_flight, in_flight) = (
                    requests_shared.clone(),
                    max_in_flight_shared.clone(),
                    in_flight_shared.clone(),
                );
                std::thread::spawn(move || {
                    use std::io::Write;

                    requests.fetch_add(1, Ordering::SeqCst);
                    let serving = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(serving, Ordering::SeqCst);

                    let mut stream = stream;
                    let _ = drain_request(&mut stream);

                    if hang {
                        std::thread::sleep(Duration::from_secs(120));
                    } else {
                        // Slow enough that requests sent without a limiter overlap, and the
                        // counter can see how many the client had in flight at once.
                        std::thread::sleep(Duration::from_millis(100));
                        // Count the request as served before the bytes leave, so a client allowed
                        // to send the next one is never counted twice.
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        let _ = stream.write_all(response.as_bytes());
                    }
                });
            }
        });

        MockServer {
            address,
            requests,
            max_in_flight,
        }
    }

    /// A reply the Ollama adapter reads: content "[]" is a clean skill.
    pub fn ollama_reply() -> String {
        let body = r#"{"model":"llama3.2","created_at":"2024-01-01T00:00:00Z","message":{"role":"assistant","content":"[]"},"done":true}"#;
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// A status-only reply, the way a rate limit or an outage arrives.
    pub fn status(status: u16, reason: &str, retry_after: Option<u64>) -> String {
        let header = retry_after
            .map(|seconds| format!("retry-after: {seconds}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 {status} {reason}\r\n{header}content-length: 2\r\nconnection: close\r\n\r\n{{}}"
        )
    }
}

/// Reads one HTTP request so the response that follows lands where the client expects it.
fn drain_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    use std::io::Read;

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..at]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")?
                        .trim()
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(0);
            if buffer.len() - at - 4 >= length {
                return Ok(());
            }
        }
    }
}
