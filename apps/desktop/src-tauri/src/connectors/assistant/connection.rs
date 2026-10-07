use super::{map_error, model::Result};
use crate::connectors::oauth::{OAuthAuthorizationRequest, OAuthPendingFlow};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub fn listener() -> Result<TcpListener> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| "unavailable")?;
    listener.set_nonblocking(true).map_err(|_| "unavailable")?;
    Ok(listener)
}
pub fn parse_http(bytes: &[u8], port: u16) -> Result<Zeroizing<String>> {
    if bytes.len() > 16384 {
        return Err("invalid_callback");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid_callback")?;
    let head = text.split_once("\r\n\r\n").ok_or("invalid_callback")?.0;
    let mut lines = head.split("\r\n");
    let line = lines.next().ok_or("invalid_callback")?;
    if line.len() > 8192 {
        return Err("invalid_callback");
    }
    let parts: Vec<_> = line.split(' ').collect();
    if parts.len() != 3
        || parts[0] != "GET"
        || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0")
        || !parts[1].starts_with("/oauth/callback?")
        || parts[1].contains('#')
    {
        return Err("invalid_callback");
    }
    let expected = format!("127.0.0.1:{port}");
    let mut host = None;
    let mut count = 0;
    let mut names = std::collections::BTreeSet::new();
    for line in lines {
        count += 1;
        if count > 32 || line.len() > 2048 {
            return Err("invalid_callback");
        }
        let (name, value) = line.split_once(':').ok_or("invalid_callback")?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !value
                .bytes()
                .all(|b| (0x20..=0x7e).contains(&b) || b == b'\t')
        {
            return Err("invalid_callback");
        }
        let name = name.to_ascii_lowercase();
        if !names.insert(name.clone()) {
            return Err("invalid_callback");
        }
        if name == "host" {
            host = Some(value.trim());
        }
        if name == "transfer-encoding" || name == "content-length" && value.trim() != "0" {
            return Err("invalid_callback");
        }
    }
    if host != Some(expected.as_str()) {
        return Err("invalid_callback");
    }
    Ok(Zeroizing::new(format!("http://{expected}{}", parts[1])))
}
fn read_request(stream: &mut TcpStream, deadline: Instant, port: u16) -> Result<Zeroizing<String>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("timeout");
    }
    stream
        .set_read_timeout(Some(remaining.min(Duration::from_secs(1))))
        .map_err(|_| "invalid_callback")?;
    let mut bytes = Zeroizing::new(Vec::new());
    let mut chunk = Zeroizing::new([0u8; 1024]);
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        if Instant::now() >= deadline {
            return Err("timeout");
        }
        stream
            .set_read_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1)),
            ))
            .map_err(|_| "invalid_callback")?;
        let n = stream
            .read(&mut chunk[..])
            .map_err(|_| "invalid_callback")?;
        if n == 0 || bytes.len() + n > 16384 {
            return Err("invalid_callback");
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    parse_http(&bytes, port)
}
fn page(stream: &mut TcpStream, valid: bool, deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return;
    }
    let body = if valid {
        "<!doctype html><title>GHOST</title><p>Authorization received. Return to GHOST for connection status.</p>"
    } else {
        "<!doctype html><title>GHOST</title><p>Authorization was not accepted. Return to GHOST.</p>"
    };
    let response=format!("HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{}",if valid{"200 OK"}else{"400 Bad Request"},body.len(),body);
    let _ = stream.set_write_timeout(Some(remaining.min(Duration::from_millis(250))));
    let _ = stream.write_all(response.as_bytes());
}
pub fn receive(
    listener: &TcpListener,
    pending: &OAuthPendingFlow,
    deadline: Instant,
) -> Result<Zeroizing<String>> {
    let port = listener.local_addr().map_err(|_| "unavailable")?.port();
    loop {
        if Instant::now() >= deadline {
            return Err("timeout");
        }
        match listener.accept() {
            Ok((mut stream, address)) => {
                if !address.ip().is_loopback() {
                    page(&mut stream, false, deadline);
                    continue;
                }
                match read_request(&mut stream, deadline, port) {
                    Ok(url) => match pending
                        .validate_callback(&url, Instant::now())
                        .map_err(map_error)
                    {
                        Ok(()) => {
                            page(&mut stream, true, deadline);
                            return Ok(url);
                        }
                        Err("consent_denied") => {
                            page(&mut stream, false, deadline);
                            return Err("consent_denied");
                        }
                        Err(_) => page(&mut stream, false, deadline),
                    },
                    Err(_) => page(&mut stream, false, deadline),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => return Err("unavailable"),
        }
    }
}
#[cfg(target_os = "macos")]
pub fn open_browser(app: &tauri::AppHandle, request: &OAuthAuthorizationRequest) -> Result<()> {
    let url = Zeroizing::new(request.url().to_owned());
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let queued = cancelled.clone();
    app.run_on_main_thread(move || {
        if queued.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let text = objc2_foundation::NSString::from_str(&url);
        let opened = objc2_foundation::NSURL::URLWithString(&text)
            .is_some_and(|url| objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url));
        let _ = send.send(opened);
    })
    .map_err(|_| "browser_failed")?;
    match receive.recv_timeout(Duration::from_secs(10)) {
        Ok(true) => Ok(()),
        _ => {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            Err("browser_failed")
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub fn open_browser(_: &tauri::AppHandle, _: &OAuthAuthorizationRequest) -> Result<()> {
    Err("unavailable")
}
