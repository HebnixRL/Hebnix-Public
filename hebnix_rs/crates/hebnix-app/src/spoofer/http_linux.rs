//! Explicit HTTP proxy. Only account HTTPS is inspected; other CONNECTs are
//! forwarded unchanged. No hosts redirects or PsyNet/WebSocket rewriting.
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crossbeam_channel::Sender;
use rustls::{ServerConnection, StreamOwned};

use super::{ca::Ca, proxy, rules::Rule};
use crate::messages::AppMsg;

pub struct HttpProxy {
    port: u16,
    running: Arc<AtomicBool>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl HttpProxy {
    pub fn start(
        ca: Arc<Ca>,
        rules: Arc<Vec<Box<dyn Rule>>>,
        tx: Sender<AppMsg>,
        ports: &[u16],
    ) -> Result<Self, String> {
        let listener = super::bind_local("HTTP proxy", ports)?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let config = proxy::build_server_config(Arc::new(proxy::CertResolver::with_default_host(
            ca,
            "api.epicgames.dev",
        )))?;
        // Pin upstream DNS so a stale hosts entry for the account host cannot loop back here.
        #[cfg(windows)]
        let pins = super::dns::resolve_a("api.epicgames.dev")
            .map(|ip| [("api.epicgames.dev".to_owned(), ip)].into())
            .unwrap_or_default();
        #[cfg(not(windows))]
        let pins = Default::default();
        let upstream = proxy::build_upstream_agent_pins(pins);
        let running = Arc::new(AtomicBool::new(true));
        let active = Arc::clone(&running);
        let worker = std::thread::Builder::new()
            .name("spoofer-http".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if !active.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let config = Arc::clone(&config);
                    let upstream = upstream.clone();
                    let rules = Arc::clone(&rules);
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        if let Err(error) = handle(stream, config, upstream, rules, tx) {
                            tracing::debug!("HTTP proxy connection ended: {error}");
                        }
                    });
                }
            })
            .map_err(|e| format!("Cannot start HTTP proxy: {e}"))?;
        Ok(Self {
            port,
            running,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
            && self
                .worker
                .lock()
                .is_ok_and(|w| w.as_ref().is_some_and(|w| !w.is_finished()))
    }

    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::AcqRel) {
            return;
        }
        let _ = TcpStream::connect((super::PROXY_HOST, self.port));
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle(
    mut client: TcpStream,
    config: Arc<rustls::ServerConfig>,
    upstream: ureq::Agent,
    rules: Arc<Vec<Box<dyn Rule>>>,
    tx: Sender<AppMsg>,
) -> Result<(), String> {
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    client
        .set_write_timeout(Some(std::time::Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    // Read only the header so TLS bytes following CONNECT stay in the socket.
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).map_err(|e| e.to_string())?;
        header.push(byte[0]);
        if header.len() > 65536 {
            return Err("Proxy request headers too large".into());
        }
    }
    let text = std::str::from_utf8(&header).map_err(|e| e.to_string())?;
    let mut first = text
        .lines()
        .next()
        .ok_or("Missing proxy request")?
        .split_whitespace();
    let method = first.next().ok_or("Missing method")?;
    let target = first.next().ok_or("Missing target")?;
    if method != "CONNECT" {
        let url = url::Url::parse(target).map_err(|e| e.to_string())?;
        if url.scheme() != "http" {
            return Err("Expected an HTTP URL".into());
        }
        url.host_str().ok_or("Missing HTTP host")?;
        let host = &url[url::Position::BeforeHost..url::Position::AfterPort];
        let mut replay = PrefixedStream {
            prefix: std::io::Cursor::new(header.clone()),
            stream: client,
        };
        return proxy::serve_one(&mut replay, &upstream, &rules, &tx, host);
    }
    let (host, port) = connect_target(target)?;
    if inspect_account(host, port) {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .map_err(|e| e.to_string())?;
        let conn = ServerConnection::new(config).map_err(|e| e.to_string())?;
        return proxy::serve_one(StreamOwned::new(conn, client), &upstream, &rules, &tx, host);
    }
    let mut server = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .map_err(|e| e.to_string())?;
    client.set_read_timeout(None).map_err(|e| e.to_string())?;
    let mut client_read = client.try_clone().map_err(|e| e.to_string())?;
    let mut server_write = server.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut client_read, &mut server_write);
        let _ = server_write.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

fn connect_target(target: &str) -> Result<(&str, u16), String> {
    let (host, port) = target
        .rsplit_once(':')
        .ok_or("CONNECT requires a host and port")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err("CONNECT host is empty".into());
    }
    Ok((host, port.parse().map_err(|_| "Invalid CONNECT port")?))
}

fn inspect_account(host: &str, port: u16) -> bool {
    port == 443 && host.eq_ignore_ascii_case("api.epicgames.dev")
}

struct PrefixedStream {
    prefix: std::io::Cursor<Vec<u8>>,
    stream: TcpStream,
}
impl Read for PrefixedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.prefix.read(buf)?;
        if n > 0 { Ok(n) } else { self.stream.read(buf) }
    }
}
impl Write for PrefixedStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn local_proxy() -> (TcpStream, std::thread::JoinHandle<Result<(), String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let worker = std::thread::spawn(move || {
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::server::ResolvesServerCertUsingSni::new()));
            handle(
                listener.accept().unwrap().0,
                Arc::new(config),
                proxy::build_upstream_agent_pins(Default::default()),
                Arc::new(Vec::new()),
                crossbeam_channel::unbounded().0,
            )
        });
        (client, worker)
    }

    #[test]
    fn connect_forwards_websocket_bytes_unchanged_without_interception() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = upstream.local_addr().unwrap();
        let echo = std::thread::spawn(move || {
            let mut stream = upstream.accept().unwrap().0;
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut frame = [0; 5];
            stream.read_exact(&mut frame).unwrap();
            assert_eq!(&frame, b"\x81\x03abc");
            stream.write_all(&frame).unwrap();
        });
        let (mut client, worker) = local_proxy();
        // Sending CONNECT and the payload together catches accidental overreads.
        let mut request =
            format!("CONNECT {address} HTTP/1.1\r\nHost: {address}\r\n\r\n").into_bytes();
        request.extend_from_slice(b"\x81\x03abc");
        client.write_all(&request).unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        assert_eq!(
            response,
            b"HTTP/1.1 200 Connection Established\r\n\r\n\x81\x03abc"
        );
        echo.join().unwrap();
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn plain_http_preserves_request_body_and_custom_upstream_port() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = upstream.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = upstream.accept().unwrap().0;
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let request = proxy::read_http_request(&mut stream).unwrap();
            assert_eq!(request.method, "POST");
            assert_eq!(request.path, "/echo?test=1");
            assert_eq!(request.body, b"hello");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let (mut client, worker) = local_proxy();
        write!(client, "POST http://{address}/echo?test=1 HTTP/1.1\r\nHost: {address}\r\nContent-Length: 5\r\n\r\nhello").unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with("\r\n\r\nok"));
        server.join().unwrap();
        worker.join().unwrap().unwrap();
    }
    #[test]
    fn psynet_and_other_websocket_hosts_are_never_inspected() {
        assert!(inspect_account("API.EPICGAMES.DEV", 443));
        for host in [
            "config.psynet.gg",
            "api.rlpp.psynet.gg",
            "ws.psynet.gg",
            "example.com",
        ] {
            assert!(!inspect_account(host, 443));
        }
        assert!(!inspect_account("api.epicgames.dev", 80));
    }
    #[test]
    fn connect_authorities_handle_ipv6_and_reject_bad_ports() {
        assert_eq!(connect_target("[::1]:443").unwrap(), ("::1", 443));
        assert!(connect_target("example.com").is_err());
        assert!(connect_target(":443").is_err());
        assert!(connect_target("example.com:bad").is_err());
    }
}
