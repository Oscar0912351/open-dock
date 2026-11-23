//! Lokal Discord IPC (`\\.\pipe\discord-ipc-0..9`) & OAuth2 token-håndtering.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

const MAX_FRAME_LEN: usize = 1_048_576;
const TOKEN_ENDPOINT: &str = "https://discord.com/api/v10/oauth2/token";
pub const DEFAULT_CENTRAL_BASE: &str = "https://ossidev.dk";

pub fn central_base_url() -> String {
    std::env::var("OPEN_DOCK_CENTRAL_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_CENTRAL_BASE.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeError {
    NotFound,
    Broken(String),
    Protocol(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "Discord IPC pipe blev ikke fundet"),
            Self::Broken(msg) | Self::Protocol(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawParticipant {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub avatar: Option<String>,
    pub discriminator: Option<String>,
    pub muted: bool,
    pub deafened: bool,
    pub is_self: bool,
    /// `voice_state.session_id` — stabilt, unikt id for brugerens voice-session.
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedVoiceChannel {
    pub id: String,
    pub name: Option<String>,
    pub participants: Vec<RawParticipant>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelfUserProfile {
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_hash: Option<String>,
    pub discriminator: Option<String>,
}

impl SelfUserProfile {
    pub fn from_user_value(user: &Value) -> Self {
        let user_id = non_empty_str(user.get("id"));
        let username = non_empty_str(user.get("username"));
        let display_name = non_empty_str(user.get("global_name")).or_else(|| username.clone());
        let avatar_hash = non_empty_str(user.get("avatar"));
        let discriminator = non_empty_str(user.get("discriminator"));
        Self {
            user_id,
            username,
            display_name,
            avatar_hash,
            discriminator,
        }
    }

    pub fn merge_from(&mut self, other: &Self) {
        if other.user_id.is_some() {
            self.user_id = other.user_id.clone();
        }
        if other.username.is_some() {
            self.username = other.username.clone();
        }
        if other.display_name.is_some() {
            self.display_name = other.display_name.clone();
        }
        if other.avatar_hash.is_some() {
            self.avatar_hash = other.avatar_hash.clone();
        }
        if other.discriminator.is_some() {
            self.discriminator = other.discriminator.clone();
        }
    }

    pub fn merged_with(mut self, fallback: &Self) -> Self {
        if self.user_id.is_none() {
            self.user_id = fallback.user_id.clone();
        }
        if self.username.is_none() {
            self.username = fallback.username.clone();
        }
        if self.display_name.is_none() {
            self.display_name = fallback.display_name.clone();
        }
        if self.avatar_hash.is_none() {
            self.avatar_hash = fallback.avatar_hash.clone();
        }
        if self.discriminator.is_none() {
            self.discriminator = fallback.discriminator.clone();
        }
        self
    }
}

pub struct DiscordIpcPipe {
    file: File,
    pub self_profile: SelfUserProfile,
}

impl std::fmt::Debug for DiscordIpcPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordIpcPipe")
            .field("self_profile", &self.self_profile)
            .finish()
    }
}

pub fn encode_frame(opcode: u32, payload: &Value) -> Result<Vec<u8>, String> {
    let json_bytes =
        serde_json::to_vec(payload).map_err(|err| format!("JSON-fejl i Discord-frame: {err}"))?;
    if json_bytes.len() > MAX_FRAME_LEN {
        return Err("Discord IPC payload overstiger 1 MiB".into());
    }
    let len = u32::try_from(json_bytes.len())
        .map_err(|_| "Discord IPC payload-længde overflow".to_string())?;
    let mut buf = Vec::with_capacity(8 + json_bytes.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&json_bytes);
    Ok(buf)
}

pub fn read_frame_from_reader<R: Read>(reader: &mut R) -> Result<(u32, Value), PipeError> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).map_err(|err| match err.kind() {
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe => {
            PipeError::Broken("Discord lukkede IPC-forbindelsen".into())
        }
        _ => PipeError::Broken(format!("Kunne ikke læse IPC-header: {err}")),
    })?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if len > MAX_FRAME_LEN {
        return Err(PipeError::Protocol(format!(
            "Discord IPC frame for stor: {len} bytes"
        )));
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|err| PipeError::Broken(format!("Kunne ikke læse IPC-body: {err}")))?;
    let val = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice::<Value>(&body)
            .map_err(|err| PipeError::Protocol(format!("Ugyldig JSON fra Discord IPC: {err}")))?
    };
    Ok((opcode, val))
}

impl DiscordIpcPipe {
    pub fn connect_any() -> Result<Self, PipeError> {
        #[cfg(windows)]
        {
            for idx in 0..10 {
                let path = format!(r"\\.\pipe\discord-ipc-{idx}");
                match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(file) => {
                        return Ok(Self {
                            file,
                            self_profile: SelfUserProfile::default(),
                        });
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                    Err(err) if err.raw_os_error() == Some(231) => continue,
                    Err(_) => continue,
                }
            }
            Err(PipeError::NotFound)
        }
        #[cfg(not(windows))]
        {
            Err(PipeError::NotFound)
        }
    }

    pub fn close(&mut self) {
        let _ = self.write_frame(OP_CLOSE, &json!({}));
    }

    pub fn write_frame(&mut self, opcode: u32, payload: &Value) -> Result<(), PipeError> {
        let frame = encode_frame(opcode, payload).map_err(PipeError::Protocol)?;
        self.file
            .write_all(&frame)
            .and_then(|_| self.file.flush())
            .map_err(|err| PipeError::Broken(format!("Discord IPC skrivning fejlede: {err}")))
    }

    pub fn send_command(
        &mut self,
        cmd: &str,
        args: Value,
        evt: Option<&str>,
    ) -> Result<String, PipeError> {
        let nonce = Uuid::new_v4().to_string();
        let mut obj = json!({
            "cmd": cmd,
            "args": args,
            "nonce": nonce,
        });
        if let (Some(e), Some(map)) = (evt, obj.as_object_mut()) {
            map.insert("evt".to_string(), Value::String(e.to_string()));
        }
        self.write_frame(OP_FRAME, &obj)?;
        Ok(nonce)
    }

    pub fn try_read_frame(&mut self) -> Result<Option<(u32, Value)>, PipeError> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::System::Pipes::PeekNamedPipe;

            let mut avail: u32 = 0;
            let handle = HANDLE(self.file.as_raw_handle() as _);
            let res = unsafe { PeekNamedPipe(handle, None, 0, None, Some(&mut avail), None) };
            if let Err(err) = res {
                return Err(PipeError::Broken(format!("Discord IPC pipe afbrudt: {err}")));
            }
            if avail < 8 {
                return Ok(None);
            }
            read_frame_from_reader(&mut self.file).map(Some)
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn read_frame_timeout(&mut self, timeout: Duration) -> Result<(u32, Value), PipeError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some((op, val)) = self.try_read_frame()? {
                if op == OP_PING {
                    let _ = self.write_frame(OP_PONG, &val);
                    continue;
                }
                if op == OP_CLOSE {
                    let msg = val
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Discord lukkede IPC-forbindelsen");
                    return Err(PipeError::Broken(msg.to_string()));
                }
                return Ok((op, val));
            }
            if Instant::now() >= deadline {
                return Err(PipeError::Broken("Timeout fra Discord IPC".into()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_for_nonce(
        &mut self,
        nonce: &str,
        timeout: Duration,
    ) -> Result<Value, PipeError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(PipeError::Broken(
                    "Tidsudløb mens der blev ventet på svar fra Discord".into(),
                ));
            }
            let (_, val) = self.read_frame_timeout(remaining)?;
            if val.get("nonce").and_then(|v| v.as_str()) == Some(nonce) {
                return Ok(val);
            }
        }
    }
}

pub fn connect_and_handshake(client_id: &str) -> Result<DiscordIpcPipe, String> {
    let cid = client_id.trim();
    if cid.is_empty() {
        return Err("Client ID må ikke være tomt".into());
    }
    #[cfg(windows)]
    {
        let mut any_pipe = false;
        let mut last_err = String::new();
        for idx in 0..10 {
            let path = format!(r"\\.\pipe\discord-ipc-{idx}");
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            any_pipe = true;
            let mut pipe = DiscordIpcPipe {
                file,
                self_profile: SelfUserProfile::default(),
            };
            let handshake = json!({
                "v": 1,
                "client_id": cid,
            });
            if let Err(err) = pipe.write_frame(OP_HANDSHAKE, &handshake) {
                last_err = err.to_string();
                continue;
            }
            match pipe.read_frame_timeout(Duration::from_secs(5)) {
                Ok((_, val)) => {
                    if val.get("evt").and_then(|v| v.as_str()) == Some("READY") {
                        if let Some(user) = val.get("data").and_then(|d| d.get("user")) {
                            pipe.self_profile = SelfUserProfile::from_user_value(user);
                        }
                        return Ok(pipe);
                    }
                    if let Some(err_msg) = extract_rpc_error(&val) {
                        last_err = err_msg;
                    }
                }
                Err(err) => {
                    last_err = err.to_string();
                }
            }
        }
        if !any_pipe {
            Err("Discord kører ikke, eller Discords lokale IPC er utilgængelig".into())
        } else if last_err.is_empty() {
            Err("Kunne ikke gennemføre handshake med Discord IPC".into())
        } else {
            Err(last_err)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = cid;
        Err("Discord IPC er kun understøttet på Windows".into())
    }
}

pub fn extract_rpc_error(val: &Value) -> Option<String> {
    if val.get("evt").and_then(|v| v.as_str()) != Some("ERROR") {
        return None;
    }
    let data = val.get("data")?;
    let msg = data
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Ukendt Discord RPC-fejl");
    Some(msg.to_string())
}

pub fn request_authorize_code(
    client_id: &str,
) -> Result<(String, bool, SelfUserProfile, DiscordIpcPipe), String> {
    let mut pipe = connect_and_handshake(client_id)?;
    let initial_profile = pipe.self_profile.clone();

    let write_scopes = json!(["rpc", "rpc.voice.read", "rpc.voice.write"]);
    let nonce = pipe
        .send_command(
            "AUTHORIZE",
            json!({
                "client_id": client_id.trim(),
                "scopes": write_scopes,
                "prompt": "consent",
            }),
            None,
        )
        .map_err(|e| e.to_string())?;

    let resp = pipe
        .wait_for_nonce(&nonce, Duration::from_secs(60))
        .map_err(|e| e.to_string())?;

    if let Some(err_msg) = extract_rpc_error(&resp) {
        if err_msg.to_ascii_lowercase().contains("scope") {
            let read_scopes = json!(["rpc", "rpc.voice.read"]);
            let retry_nonce = pipe
                .send_command(
                    "AUTHORIZE",
                    json!({
                        "client_id": client_id.trim(),
                        "scopes": read_scopes,
                        "prompt": "consent",
                    }),
                    None,
                )
                .map_err(|e| e.to_string())?;
            let retry_resp = pipe
                .wait_for_nonce(&retry_nonce, Duration::from_secs(60))
                .map_err(|e| e.to_string())?;
            if let Some(retry_err) = extract_rpc_error(&retry_resp) {
                pipe.close();
                return Err(retry_err);
            }
            let code = match retry_resp
                .get("data")
                .and_then(|d| d.get("code"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                Some(c) => c.to_string(),
                None => {
                    pipe.close();
                    return Err("Discord returnerede ingen autorisationskode".to_string());
                }
            };
            return Ok((code, false, initial_profile, pipe));
        }
        pipe.close();
        return Err(err_msg);
    }

    let code = match resp
        .get("data")
        .and_then(|d| d.get("code"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(c) => c.to_string(),
        None => {
            pipe.close();
            return Err("Discord returnerede ingen autorisationskode".to_string());
        }
    };
    Ok((code, true, initial_profile, pipe))
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

pub fn build_form_body(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub async fn exchange_code_for_token(
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|err| format!("HTTP-klient fejl: {err}"))?;

    let trimmed_redirect = redirect_uri.trim();
    let mut fields = vec![
        ("client_id", client_id.trim()),
        ("client_secret", client_secret.trim()),
        ("grant_type", "authorization_code"),
        ("code", code.trim()),
    ];
    if !trimmed_redirect.is_empty() {
        fields.push(("redirect_uri", trimmed_redirect));
    }

    let resp = client
        .post(TOKEN_ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(build_form_body(&fields))
        .send()
        .await
        .map_err(|err| format!("Netværksfejl mod Discord OAuth2: {err}"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str::<TokenResponse>(&body_text)
            .map_err(|err| format!("Ugyldigt token-svar fra Discord: {err}"));
    }

    // Nogle apps uden redirect_uri i Developer Portal afviser feltet ved local IPC code exchange.
    if status.as_u16() == 400
        && !trimmed_redirect.is_empty()
        && body_text.to_ascii_lowercase().contains("redirect_uri")
    {
        let fallback_fields = [
            ("client_id", client_id.trim()),
            ("client_secret", client_secret.trim()),
            ("grant_type", "authorization_code"),
            ("code", code.trim()),
        ];
        let retry = client
            .post(TOKEN_ENDPOINT)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(build_form_body(&fallback_fields))
            .send()
            .await
            .map_err(|err| format!("Netværksfejl mod Discord OAuth2: {err}"))?;
        let retry_status = retry.status();
        let retry_text = retry.text().await.unwrap_or_default();
        if retry_status.is_success() {
            return serde_json::from_str::<TokenResponse>(&retry_text)
                .map_err(|err| format!("Ugyldigt token-svar fra Discord: {err}"));
        }
        return Err(format_oauth_error(retry_status.as_u16(), &retry_text));
    }

    Err(format_oauth_error(status.as_u16(), &body_text))
}

pub async fn refresh_access_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|err| format!("HTTP-klient fejl: {err}"))?;

    let fields = [
        ("client_id", client_id.trim()),
        ("client_secret", client_secret.trim()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.trim()),
    ];

    let resp = client
        .post(TOKEN_ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(build_form_body(&fields))
        .send()
        .await
        .map_err(|err| format!("Netværksfejl ved token-fornyelse: {err}"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        serde_json::from_str::<TokenResponse>(&body_text)
            .map_err(|err| format!("Ugyldigt refresh-svar fra Discord: {err}"))
    } else {
        Err(format_oauth_error(status.as_u16(), &body_text))
    }
}

pub async fn fetch_central_rpc_config(base_url: &str) -> Result<(String, String), String> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/api/discord-rpc/config");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|err| format!("HTTP-klient fejl: {err}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|err| format!("Kunne ikke kontakte OSSI Central: {err}"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if status.as_u16() == 404 || status.as_u16() == 503 {
        return Err("OSSI Central har ikke aktiveret Discord RPC-forbindelse endnu".into());
    }
    if !status.is_success() {
        return Err(format_central_error(status.as_u16(), &body_text));
    }

    let val: Value = serde_json::from_str(&body_text)
        .map_err(|err| format!("Ugyldigt svar fra OSSI Central: {err}"))?;
    let inner = val.get("data").filter(|v| v.is_object()).unwrap_or(&val);
    let enabled = inner
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return Err("OSSI Central har ikke aktiveret Discord RPC-forbindelse endnu".into());
    }
    let client_id = non_empty_str(inner.get("clientId"))
        .ok_or_else(|| "OSSI Central returnerede intet Discord Client ID".to_string())?;
    let redirect_uri = non_empty_str(inner.get("redirectUri"))
        .unwrap_or_else(|| "http://127.0.0.1".to_string());
    Ok((client_id, redirect_uri))
}

async fn post_central_token(base_url: &str, body: Value) -> Result<TokenResponse, String> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/api/discord-rpc/token");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|err| format!("HTTP-klient fejl: {err}"))?;

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|err| format!("Netværksfejl mod OSSI Central: {err}"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format_central_error(status.as_u16(), &body_text));
    }

    let val: Value = serde_json::from_str(&body_text)
        .map_err(|err| format!("Ugyldigt token-svar fra OSSI Central: {err}"))?;
    let inner = val.get("data").filter(|v| v.is_object()).unwrap_or(&val);
    serde_json::from_value::<TokenResponse>(inner.clone())
        .map_err(|err| format!("Ugyldigt token-format fra OSSI Central: {err}"))
}

pub async fn exchange_code_via_central(base_url: &str, code: &str) -> Result<TokenResponse, String> {
    post_central_token(
        base_url,
        json!({
            "grantType": "authorization_code",
            "code": code.trim(),
        }),
    )
    .await
}

pub async fn refresh_token_via_central(
    base_url: &str,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    post_central_token(
        base_url,
        json!({
            "grantType": "refresh_token",
            "refreshToken": refresh_token.trim(),
        }),
    )
    .await
}

fn format_central_error(status: u16, body: &str) -> String {
    if status == 404 {
        return "OSSI Central har ikke aktiveret Discord RPC-forbindelse endnu".into();
    }
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = non_empty_str(val.get("error")).or_else(|| non_empty_str(val.get("message"))) {
            return msg;
        }
    }
    format!("OSSI Central svarede med fejl (HTTP {status})")
}

fn format_oauth_error(status: u16, body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        if let Some(desc) = non_empty_str(val.get("error_description"))
            .or_else(|| non_empty_str(val.get("message")))
            .or_else(|| non_empty_str(val.get("error")))
        {
            return format!("Discord afviste OAuth2 ({status}): {desc}");
        }
    }
    format!("Discord OAuth2 fejlede (HTTP {status})")
}

pub fn non_empty_str(val: Option<&Value>) -> Option<String> {
    val.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

pub fn parse_voice_state_entry(
    entry: &Value,
    self_user_id: Option<&str>,
) -> Option<RawParticipant> {
    let user = entry.get("user")?;
    let id = non_empty_str(user.get("id"))?;
    let username = non_empty_str(user.get("username")).unwrap_or_else(|| "Discord".into());
    let display_name = non_empty_str(entry.get("nick"))
        .or_else(|| non_empty_str(user.get("global_name")))
        .unwrap_or_else(|| username.clone());
    let avatar = non_empty_str(user.get("avatar"));
    let discriminator = non_empty_str(user.get("discriminator"));

    let vs = entry.get("voice_state");
    let flag = |obj: Option<&Value>, key: &str| -> bool {
        obj.and_then(|o| o.get(key))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let deafened = flag(vs, "self_deaf") || flag(vs, "deaf");
    let muted = deafened
        || flag(vs, "self_mute")
        || flag(vs, "mute")
        || flag(vs, "suppress")
        || entry
            .get("mute")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let is_self = self_user_id.is_some_and(|sid| sid == id);
    let session_id = vs.and_then(|o| non_empty_str(o.get("session_id")));

    Some(RawParticipant {
        id,
        username,
        display_name,
        avatar,
        discriminator,
        muted,
        deafened,
        is_self,
        session_id,
    })
}

pub fn parse_voice_channel(
    data: &Value,
    self_user_id: Option<&str>,
) -> Option<ParsedVoiceChannel> {
    if data.is_null() {
        return None;
    }
    let id = non_empty_str(data.get("id"))?;
    let ch_type = data.get("type").and_then(|v| v.as_i64()).unwrap_or(2);
    let raw_name = non_empty_str(data.get("name"));
    // DM / Group DM (type 1 eller 3) kan returnere generisk/tomt kanalnavn; lad UI falde tilbage til "Privat call".
    let name = if ch_type == 1 || ch_type == 3 {
        raw_name.filter(|n| !n.eq_ignore_ascii_case("Unknown"))
    } else {
        raw_name
    };
    let participants = data
        .get("voice_states")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| parse_voice_state_entry(entry, self_user_id))
                .collect()
        })
        .unwrap_or_default();

    Some(ParsedVoiceChannel {
        id,
        name,
        participants,
    })
}

pub fn parse_voice_settings(data: &Value) -> Option<(bool, bool)> {
    let mute = data.get("mute").and_then(|v| v.as_bool());
    let deaf = data.get("deaf").and_then(|v| v.as_bool());
    match (mute, deaf) {
        (Some(m), Some(d)) => Some((m, d)),
        (Some(m), None) => Some((m, false)),
        (None, Some(d)) => Some((d, d)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_encode_decode_roundtrip() {
        let payload = json!({
            "cmd": "GET_SELECTED_VOICE_CHANNEL",
            "args": {},
            "nonce": "n-1",
        });
        let encoded = encode_frame(OP_FRAME, &payload).expect("encode");
        let mut cursor = Cursor::new(encoded);
        let (op, decoded) = read_frame_from_reader(&mut cursor).expect("decode");
        assert_eq!(op, OP_FRAME);
        assert_eq!(decoded, payload);
    }

    #[test]
    fn url_encode_form_fields() {
        let body = build_form_body(&[
            ("client_id", "123456"),
            ("redirect_uri", "http://127.0.0.1:8080/cb?x=1 2"),
            ("grant_type", "authorization_code"),
        ]);
        assert_eq!(
            body,
            "client_id=123456&redirect_uri=http%3A%2F%2F127.0.0.1%3A8080%2Fcb%3Fx%3D1%202&grant_type=authorization_code"
        );
    }

    #[test]
    fn parse_voice_channel_and_states() {
        let payload = json!({
            "id": "777",
            "name": "Gaming Lounge",
            "type": 2,
            "voice_states": [
                {
                    "nick": "ServerNick",
                    "mute": false,
                    "user": {
                        "id": "100",
                        "username": "friend1",
                        "global_name": "Friend Global",
                        "avatar": "hash1",
                        "discriminator": "0"
                    },
                    "voice_state": {
                        "session_id": "sess-friend",
                        "mute": false,
                        "deaf": false,
                        "self_mute": true,
                        "self_deaf": false,
                        "suppress": false
                    }
                },
                {
                    "user": {
                        "id": "200",
                        "username": "oscar",
                        "global_name": "Oscar",
                        "avatar": null,
                        "discriminator": "0"
                    },
                    "voice_state": {
                        "mute": false,
                        "deaf": false,
                        "self_mute": false,
                        "self_deaf": false,
                        "suppress": false
                    }
                }
            ]
        });

        let ch = parse_voice_channel(&payload, Some("200")).expect("channel");
        assert_eq!(ch.id, "777");
        assert_eq!(ch.name.as_deref(), Some("Gaming Lounge"));
        assert_eq!(ch.participants.len(), 2);

        assert_eq!(ch.participants[0].display_name, "ServerNick");
        assert!(ch.participants[0].muted);
        assert!(!ch.participants[0].deafened);
        assert!(!ch.participants[0].is_self);
        assert_eq!(ch.participants[0].session_id.as_deref(), Some("sess-friend"));

        assert_eq!(ch.participants[1].display_name, "Oscar");
        assert!(!ch.participants[1].muted);
        assert!(ch.participants[1].is_self);

        let dm = json!({
            "id": "888",
            "name": "",
            "type": 1,
            "voice_states": []
        });
        let parsed_dm = parse_voice_channel(&dm, None).expect("dm");
        assert_eq!(parsed_dm.name, None);
    }
}
