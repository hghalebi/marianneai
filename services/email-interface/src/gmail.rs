//! Gmail ingestion through the Google Workspace CLI (`gws`).
//! The reader uses `gws` JSON mode so the pipeline receives strict structured
//! payloads and can continue reliably without HTML scraping.

use crate::types::GmailMessage;
use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use thiserror::Error;

// Includes write/send scopes so queueing and future response workflows can be
// enabled without re-authenticating.
const GMAIL_SCOPE_FIX: &str = "gws auth login --scopes https://www.googleapis.com/auth/gmail.modify,https://www.googleapis.com/auth/gmail.send";
const GMAIL_CLI_ENV_VARS: [&str; 2] = ["GWS_BIN", "GMAIL_CLI_COMMAND"];

/// Thin wrapper around the `gws` Gmail commands.
#[derive(Debug, Clone, Default)]
pub struct GmailClient;

/// Returns the CLI command name/path used for all `gws` invocations.
pub fn gmail_cli_command() -> String {
    GMAIL_CLI_ENV_VARS
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_else(|| "gws".to_string())
}

/// Returns the default `gws` credentials directory for current user.
pub fn gws_config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        let mut dir = PathBuf::from(home);
        dir.push(".config/gws");
        dir
    })
}

impl GmailClient {
    /// Load recent Gmail messages matching the supplied query, bounded to avoid API
    /// request-time and payload-size issues for oversized requests.
    pub fn fetch_messages(
        &self,
        query: &str,
        max_results: u32,
    ) -> Result<Vec<GmailMessage>, GmailError> {
        let safe_max_results = max_results.clamp(1, 25);
        let params = json!({
            "userId": "me",
            "q": query,
            "maxResults": safe_max_results,
            "includeSpamTrash": false
        });

        let list_value = run_gws_json(&[
            "gmail",
            "users",
            "messages",
            "list",
            "--params",
            &params.to_string(),
            "--format",
            "json",
        ])?;

        let list_response: ListMessagesResponse =
            serde_json::from_value(list_value).map_err(GmailError::DeserializeResponse)?;

        let mut messages = Vec::new();
        for message_ref in list_response.messages.unwrap_or_default() {
            messages.push(self.fetch_message(&message_ref.id)?);
        }

        Ok(messages)
    }

    fn fetch_message(&self, message_id: &str) -> Result<GmailMessage, GmailError> {
        let params = json!({
            "userId": "me",
            "id": message_id,
            "format": "full"
        });

        let value = run_gws_json(&[
            "gmail",
            "users",
            "messages",
            "get",
            "--params",
            &params.to_string(),
            "--format",
            "json",
        ])?;

        let raw: RawMessage =
            serde_json::from_value(value).map_err(GmailError::DeserializeResponse)?;
        raw.into_message()
    }

    /// Send a response email using the Gmail send API.
    pub fn send_reply(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
        from: Option<&str>,
    ) -> Result<SentMessage, GmailError> {
        let raw_message = compose_raw_message(to, subject, body, thread_id, from);
        let encoded = URL_SAFE_NO_PAD.encode(raw_message.as_bytes());
        let params = json!({ "userId": "me" });
        let mut body = serde_json::Map::new();
        body.insert("raw".to_string(), json!(encoded));
        if let Some(thread_id) = thread_id {
            body.insert("threadId".to_string(), json!(thread_id));
        }
        let body = Value::Object(body);

        let value = run_gws_json(&[
            "gmail",
            "users",
            "messages",
            "send",
            "--params",
            &params.to_string(),
            "--format",
            "json",
            "--json",
            &body.to_string(),
        ])?;
        let response: SentMessageBody =
            serde_json::from_value(value).map_err(GmailError::DeserializeResponse)?;

        Ok(response.into())
    }

    /// Try to infer a reply address from the `From` header.
    pub fn infer_reply_recipient(&self, message: &GmailMessage) -> Option<String> {
        parse_email_address(message.from.as_deref())
    }
}

#[derive(Debug, Error)]
pub enum GmailError {
    #[error("failed to spawn Gmail CLI command: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("gws returned invalid UTF-8 on stdout")]
    StdoutUtf8(#[from] std::string::FromUtf8Error),

    #[error("gws did not return valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("failed to deserialize Gmail response: {0}")]
    DeserializeResponse(serde_json::Error),

    #[error("gws returned no JSON output")]
    EmptyOutput,

    #[error("{0}")]
    CommandFailed(String),

    #[error("failed to decode Gmail message body: {0}")]
    Base64(#[from] base64::DecodeError),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListMessagesResponse {
    messages: Option<Vec<MessageReference>>,
}

#[derive(Debug, Deserialize)]
struct MessageReference {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessage {
    id: String,
    thread_id: String,
    snippet: Option<String>,
    label_ids: Option<Vec<String>>,
    payload: Option<MessagePart>,
}

impl RawMessage {
    fn into_message(self) -> Result<GmailMessage, GmailError> {
        let payload = self.payload.unwrap_or_default();
        let from = payload.header_value("From");
        let to = payload.header_value("To");
        let subject = payload.header_value("Subject");
        let date = payload.header_value("Date");

        let mut collector = BodyCollector::default();
        collector.collect(&payload)?;

        Ok(GmailMessage {
            message_id: self.id,
            thread_id: self.thread_id,
            from,
            to,
            subject,
            date,
            snippet: self.snippet,
            plain_text_body: collector.plain_text,
            html_body: collector.html,
            label_ids: self.label_ids.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SentMessageBody {
    id: Option<String>,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    label_ids: Option<Vec<String>>,
}

/// Result from the Gmail send API, mapped to concise fields for the responder stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentMessage {
    /// Gmail-assigned id of the sent message.
    pub message_id: Option<String>,
    /// Gmail thread id after send.
    pub thread_id: Option<String>,
    /// Labels returned by the API.
    pub label_ids: Vec<String>,
}

impl From<SentMessageBody> for SentMessage {
    fn from(value: SentMessageBody) -> Self {
        Self {
            message_id: value.id,
            thread_id: value.thread_id,
            label_ids: value.label_ids.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePart {
    mime_type: Option<String>,
    headers: Option<Vec<MessageHeader>>,
    parts: Option<Vec<MessagePart>>,
    body: Option<MessageBody>,
}

impl MessagePart {
    fn header_value(&self, name: &str) -> Option<String> {
        self.headers.as_ref().and_then(|headers| {
            headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
                .map(|header| header.value.clone())
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MessageHeader {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageBody {
    data: Option<String>,
}

#[derive(Debug, Default)]
struct BodyCollector {
    plain_text: Option<String>,
    html: Option<String>,
}

impl BodyCollector {
    fn collect(&mut self, part: &MessagePart) -> Result<(), GmailError> {
        if let Some(data) = part.body.as_ref().and_then(|body| body.data.as_deref()) {
            let decoded = decode_gmail_body(data)?;
            match part.mime_type.as_deref() {
                Some("text/plain") if self.plain_text.is_none() => {
                    self.plain_text = Some(decoded);
                }
                Some("text/html") if self.html.is_none() => {
                    self.html = Some(decoded);
                }
                _ => {}
            }
        }

        if let Some(parts) = &part.parts {
            // Gmail often nests the human-readable body in multipart/alternative branches.
            for child in parts {
                self.collect(child)?;
            }
        }

        Ok(())
    }
}

fn decode_gmail_body(data: &str) -> Result<String, GmailError> {
    // Gmail APIs emit URL-safe base64 segments with variable line breaks; normalize
    // before decoding so both line-broken and URL-safe payloads are accepted.
    let mut normalized = data.trim().replace('\n', "");
    let padding = (4 - normalized.len() % 4) % 4;
    normalized.push_str(&"=".repeat(padding));
    let bytes = URL_SAFE.decode(normalized)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn run_gws_json(args: &[&str]) -> Result<Value, GmailError> {
    let program = gmail_cli_command();

    let spec = CommandSpec {
        program,
        base_args: Vec::new(),
    };

    match spec.run(args) {
        Ok(value) => Ok(value),
        Err(GmailError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(GmailError::Spawn(std::io::Error::new(
                error.kind(),
                format!(
                    "`{}` command not found. Install Google Workspace CLI and retry.",
                    spec.program
                ),
            )))
        }
        Err(other) => Err(other),
    }
}

#[derive(Debug, Clone)]
struct CommandSpec {
    program: String,
    base_args: Vec<String>,
}

impl CommandSpec {
    fn run(&self, args: &[&str]) -> Result<Value, GmailError> {
        let output = Command::new(&self.program)
            .args(&self.base_args)
            .args(args)
            .output()?;

        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;

        if !output.status.success() {
            return Err(GmailError::CommandFailed(summarize_failure(
                &self.program,
                &self.base_args,
                args,
                stdout.trim(),
                stderr.trim(),
            )));
        }

        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Err(GmailError::EmptyOutput);
        }

        serde_json::from_str(trimmed).map_err(GmailError::InvalidJson)
    }
}

fn summarize_failure(
    program: &str,
    base_args: &[String],
    args: &[&str],
    stdout: &str,
    stderr: &str,
) -> String {
    let rendered_command = std::iter::once(program.to_string())
        .chain(base_args.iter().cloned())
        .chain(args.iter().map(|arg| arg.to_string()))
        .collect::<Vec<_>>()
        .join(" ");

    if stdout.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
        || stdout.contains("insufficient authentication scopes")
    {
        return format!(
            "{} failed because the active gws credentials do not include Gmail scopes. Re-run `{}`.",
            rendered_command, GMAIL_SCOPE_FIX
        );
    }

    if let Ok(value) = serde_json::from_str::<Value>(stdout)
        && let Some(message) = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
    {
        return format!("{} failed: {}", rendered_command, message);
    }

    if !stderr.is_empty() {
        return format!("{} failed: {}", rendered_command, stderr);
    }

    if !stdout.is_empty() {
        return format!("{} failed: {}", rendered_command, stdout);
    }

    format!("{} failed with no diagnostic output", rendered_command)
}

fn compose_raw_message(
    to: &str,
    subject: &str,
    body: &str,
    thread_id: Option<&str>,
    from: Option<&str>,
) -> String {
    let subject = if subject.to_ascii_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    };

    let mut headers = vec![
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=\"UTF-8\"".to_string(),
        "Content-Transfer-Encoding: 7bit".to_string(),
        format!("To: {to}"),
        format!("Subject: {subject}"),
        format!("Date: {}", Utc::now().to_rfc2822()),
        "X-Mailer: email-interface".to_string(),
    ];

    if let Some(from) = from.filter(|value| !value.trim().is_empty()) {
        headers.push(format!("From: {from}"));
    }

    if let Some(thread_id) = thread_id {
        headers.push(format!("References: {thread_id}"));
    }

    let safe_body = sanitize_ascii_body(body);
    let mut message = headers.join("\r\n");
    message.push_str("\r\n\r\n");
    message.push_str(&safe_body);
    message.push('\n');
    message
}

fn sanitize_ascii_body(body: &str) -> String {
    body.chars()
        .map(|value| {
            if value.is_ascii_graphic()
                || value == '\n'
                || value == '\r'
                || value == '\t'
                || value == ' '
            {
                value
            } else if value == '\u{00a0}' {
                ' '
            } else {
                '?'
            }
        })
        .collect()
}

fn parse_email_address(header: Option<&str>) -> Option<String> {
    let value = header?.trim();
    if value.is_empty() {
        return None;
    }

    let candidate = if let Some((_, after)) = value.split_once('<') {
        after.split('>').next().unwrap_or(after).trim().to_string()
    } else {
        value.to_string()
    };

    if candidate.contains('@') && !candidate.contains(' ') {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_raw_message, decode_gmail_body, parse_email_address, summarize_failure};

    #[test]
    fn decodes_base64url_gmail_body() {
        let decoded = decode_gmail_body("SGVsbG8td29ybGQ").expect("body should decode");
        assert_eq!(decoded, "Hello-world");
    }

    #[test]
    fn summarizes_missing_scopes() {
        let message = summarize_failure(
            "gws",
            &[],
            &["gmail", "users", "messages", "list"],
            r#"{"error":{"message":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}}"#,
            "",
        );
        assert!(message.contains("gws auth login"));
    }

    #[test]
    fn builds_reply_message() {
        let raw = compose_raw_message(
            "alice@example.com",
            "Update request",
            "Thanks for contacting us.",
            Some("thread-123"),
            Some("agent@example.com"),
        );
        assert!(raw.contains("To: alice@example.com"));
        assert!(raw.contains("Subject: Re: Update request"));
        assert!(raw.contains("Thanks for contacting us."));
        assert!(raw.contains("References: thread-123"));
    }

    #[test]
    fn parses_from_address_in_angle_brackets() {
        assert_eq!(
            parse_email_address(Some("Alice Example <alice@example.com>")),
            Some("alice@example.com".to_string())
        );
    }

    #[test]
    fn parses_plain_from_address() {
        assert_eq!(
            parse_email_address(Some("alice@example.com")),
            Some("alice@example.com".to_string())
        );
    }
}
