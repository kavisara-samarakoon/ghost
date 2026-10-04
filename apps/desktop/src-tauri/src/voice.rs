//! One confirmed in-memory recording becomes literal transcript text; nothing dispatches it.
#[cfg(unix)]
mod storage;
#[cfg(test)]
mod tests;

use reqwest::blocking::{multipart, Client, Request};
use serde::Serialize;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::ipc::InvokeBody;

pub const MODEL: &str = "gpt-transcribe";
pub const ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
pub const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DURATION_MS: u32 = 30_000;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;
pub const CONFIRMED: &str = "send-to-openai";
const TRANSPORT_ERROR: &str = "transport";
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub struct VoiceLease;
impl VoiceLease {
    pub fn acquire() -> Result<Self, &'static str> {
        IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "busy")
    }
}
impl Drop for VoiceLease {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::Release);
    }
}

pub struct Audio {
    bytes: Vec<u8>,
    mime: &'static str,
    filename: &'static str,
    category: &'static str,
    duration_ms: u32,
}
fn format(mime: &str) -> Result<(&'static str, &'static str, &'static str), &'static str> {
    match mime {
        "audio/webm;codecs=opus" => Ok(("audio/webm;codecs=opus", "voice.webm", "webm")),
        "audio/webm" => Ok(("audio/webm", "voice.webm", "webm")),
        "audio/mp4" => Ok(("audio/mp4", "voice.mp4", "mp4")),
        "audio/ogg;codecs=opus" => Ok(("audio/ogg;codecs=opus", "voice.ogg", "ogg")),
        "audio/ogg" => Ok(("audio/ogg", "voice.ogg", "ogg")),
        _ => Err("format"),
    }
}
pub fn validate(
    window: &str,
    body: &InvokeBody,
    mime: Option<&str>,
    duration: Option<&str>,
    confirmation: Option<&str>,
) -> Result<Audio, &'static str> {
    if window != "main" {
        return Err("unavailable");
    }
    if confirmation != Some(CONFIRMED) {
        return Err("unavailable");
    }
    let InvokeBody::Raw(bytes) = body else {
        return Err("unavailable");
    };
    if bytes.is_empty() {
        return Err("recording");
    }
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err("too_large");
    }
    let (mime, filename, category) = format(mime.ok_or("format")?)?;
    let duration = duration.ok_or("duration")?;
    if duration.len() > 5
        || duration.is_empty()
        || !duration.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("duration");
    }
    let duration_ms = duration.parse::<u32>().map_err(|_| "duration")?;
    if duration_ms == 0 || duration_ms > MAX_DURATION_MS {
        return Err("duration");
    }
    Ok(Audio {
        bytes: bytes.clone(),
        mime,
        filename,
        category,
        duration_ms,
    })
}

#[derive(Serialize)]
pub struct VoiceResult {
    pub text: String,
    pub model: &'static str,
    pub audio_bytes: usize,
    pub duration_ms: u32,
    pub audit_recorded: bool,
}
#[derive(Clone, Copy, Serialize)]
pub struct AuditEvent {
    timestamp: u64,
    event: &'static str,
    model: &'static str,
    mime_category: &'static str,
    audio_bytes: usize,
    duration_ms: u32,
    result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript_bytes: Option<usize>,
}
impl AuditEvent {
    fn new(audio: &Audio, text: Option<&str>) -> Result<Self, &'static str> {
        let completed = text.is_some();
        Ok(Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "audit")?
                .as_secs(),
            event: if completed {
                "desktop.voice_transcription.completed"
            } else {
                "desktop.voice_transcription.confirmed"
            },
            model: MODEL,
            mime_category: audio.category,
            audio_bytes: audio.bytes.len(),
            duration_ms: audio.duration_ms,
            result: if completed { "completed" } else { "confirmed" },
            transcript_bytes: text.map(str::len),
        })
    }
}

fn credential(value: Option<String>) -> Result<String, &'static str> {
    let key = value.ok_or("credential")?;
    if key.is_empty() || key.len() > 1024 || !key.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err("credential");
    }
    Ok(key)
}
fn sanitize(text: &str, key: &str) -> Result<String, &'static str> {
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err("transcript");
    }
    // Cf plus terminal escapes and controls; ordinary Unicode is retained as literal text.
    static ESCAPES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static FORMATS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let escaped = ESCAPES
        .get_or_init(|| {
            regex::Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\))").unwrap()
        })
        .replace_all(text, "");
    let clean = FORMATS
        .get_or_init(|| regex::Regex::new(r"[\p{Cf}\p{Cc}&&[^\n\t]]").unwrap())
        .replace_all(&escaped, "");
    let clean = clean.replace(key, "[REDACTED]");
    let clean = clean.trim();
    if clean.is_empty() || clean.len() > MAX_TRANSCRIPT_BYTES {
        return Err("transcript");
    }
    Ok(clean.to_owned())
}

fn transcribe_with(
    audio: Audio,
    mut audit: impl FnMut(AuditEvent) -> Result<(), &'static str>,
    lookup: impl FnOnce() -> Option<String>,
    transport: impl FnOnce(&Audio, &str) -> Result<String, &'static str>,
) -> Result<VoiceResult, &'static str> {
    audit(AuditEvent::new(&audio, None)?).map_err(|_| "audit")?;
    let key = credential(lookup())?;
    let raw_text = transport(&audio, &key)?;
    let text = sanitize(&raw_text, &key)?;
    let audit_recorded = AuditEvent::new(&audio, Some(&text))
        .and_then(&mut audit)
        .is_ok();
    Ok(VoiceResult {
        text,
        model: MODEL,
        audio_bytes: audio.bytes.len(),
        duration_ms: audio.duration_ms,
        audit_recorded,
    })
}

fn client() -> Result<Client, &'static str> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| "service")
}
fn build_request(client: &Client, audio: &Audio, key: &str) -> Result<Request, &'static str> {
    let file = multipart::Part::bytes(audio.bytes.clone())
        .file_name(audio.filename)
        .mime_str(audio.mime)
        .map_err(|_| "format")?;
    let form = multipart::Form::new()
        .part("file", file)
        .text("model", MODEL)
        .text("response_format", "json");
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| "credential")?;
    authorization.set_sensitive(true);
    client
        .post(ENDPOINT)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .multipart(form)
        .build()
        .map_err(|_| "service")
}
fn parse_response(
    status: u16,
    length: Option<u64>,
    body: impl Read,
) -> Result<String, &'static str> {
    if !(200..300).contains(&status) {
        return Err("service");
    }
    if length.is_some_and(|size| size > MAX_RESPONSE_BYTES as u64) {
        return Err("transcript");
    }
    let mut bytes = Vec::new();
    body.take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TRANSPORT_ERROR)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("transcript");
    }
    let utf8 = std::str::from_utf8(&bytes).map_err(|_| "transcript")?;
    let value: serde_json::Value = serde_json::from_str(utf8).map_err(|_| "transcript")?;
    let text = value
        .as_object()
        .and_then(|value| value.get("text"))
        .and_then(|value| value.as_str())
        .ok_or("transcript")?;
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err("transcript");
    }
    Ok(text.to_owned())
}
fn send_openai_transcription(audio: &Audio, key: &str) -> Result<String, &'static str> {
    let client = client()?;
    let request = build_request(&client, audio, key)?;
    // Exactly one execute; no loops, redirects, or retries, including client default retries.
    let response = client.execute(request).map_err(|_| TRANSPORT_ERROR)?;
    parse_response(
        response.status().as_u16(),
        response.content_length(),
        response,
    )
}

#[cfg(unix)]
pub fn from_environment(audio: Audio) -> Result<VoiceResult, &'static str> {
    let home = storage::resolve_home()?;
    let store = storage::AuditStore::open(&home)?;
    transcribe_with(
        audio,
        |event| store.append(event),
        || std::env::var("OPENAI_API_KEY").ok(),
        send_openai_transcription,
    )
}
#[cfg(not(unix))]
pub fn from_environment(_: Audio) -> Result<VoiceResult, &'static str> {
    Err("unavailable")
}
