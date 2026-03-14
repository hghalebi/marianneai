//! Shared domain types for the email interface pipeline.
//! These types are intentionally serialization-friendly because every run snapshot
//! and report writes the full mission state for audit and reproducibility.

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

/// Supported queue persistence targets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueueBackendKind {
    /// Do not persist queue entries. Useful for dry runs or local validation.
    #[default]
    None,
    /// Persist queue entries to PostgreSQL on GCP.
    Postgres,
}

impl QueueBackendKind {
    /// Parse the backend from `QUEUE_BACKEND`, defaulting to `none`.
    pub fn from_env() -> Self {
        match std::env::var("QUEUE_BACKEND")
            .unwrap_or_else(|_| "none".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "postgres" | "postgresql" => Self::Postgres,
            _ => Self::None,
        }
    }
}

/// Project-level relevance profile for MarianneAI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProfile {
    /// Product name.
    pub name: String,
    /// High-level description.
    pub about: String,
    /// Public website.
    pub website: String,
    /// Phrases that usually indicate a relevant inbound email.
    pub relevance_hints: Vec<String>,
    /// Conditions for a task to be clear enough for queue insertion.
    pub valid_task_criteria: Vec<String>,
}

impl Default for ProjectProfile {
    /// Keep profile defaults in code so the system can be moved to a new project
    /// by implementing a single profile provider.
    fn default() -> Self {
        Self {
            name: "MarianneAI".to_string(),
            about: "L’IA citoyenne pour comprendre les données publiques.".to_string(),
            website: "https://marianneai.org/".to_string(),
            relevance_hints: vec![
                "questions about French public data".to_string(),
                "feature requests for MarianneAI".to_string(),
                "bug reports or operational problems".to_string(),
                "partnership, pilot, or demo inquiries".to_string(),
                "requests to analyze or explain public datasets".to_string(),
            ],
            valid_task_criteria: vec![
                "the sender intent is specific and actionable".to_string(),
                "there is enough detail to route work without guessing core requirements"
                    .to_string(),
                "the request relates to MarianneAI or its public-data mission".to_string(),
                "the email includes enough context to create a job queue item".to_string(),
            ],
        }
    }
}

/// A Gmail message normalized for the triage pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailMessage {
    /// Immutable Gmail message id.
    pub message_id: String,
    /// Gmail thread id.
    pub thread_id: String,
    /// Sender header.
    pub from: Option<String>,
    /// Recipient header.
    pub to: Option<String>,
    /// Subject line.
    pub subject: Option<String>,
    /// RFC 2822 date header.
    pub date: Option<String>,
    /// Gmail snippet.
    pub snippet: Option<String>,
    /// Plain text body, if available.
    pub plain_text_body: Option<String>,
    /// HTML body, if available.
    pub html_body: Option<String>,
    /// Gmail labels attached to the message.
    pub label_ids: Vec<String>,
}

impl GmailMessage {
    /// Returns a prompt-safe textual representation of the message body by
    /// preferring plain text, then HTML, then snippet as fallback.
    pub fn prompt_body(&self) -> String {
        if let Some(body) = self
            .plain_text_body
            .as_ref()
            .filter(|body| !body.trim().is_empty())
        {
            return body.clone();
        }

        self.html_body
            .clone()
            .or_else(|| self.snippet.clone())
            .unwrap_or_else(|| "No message body available.".to_string())
    }
}

/// Relevance classification for one email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailClassification {
    /// Whether the email is about MarianneAI or its mission.
    pub related_to_project: bool,
    /// High-level category such as `feature_request` or `demo_request`.
    pub category: String,
    /// Confidence score in the classification, `0.0..=1.0`.
    pub confidence: f32,
    /// Whether the message touches public-data workflows directly.
    pub public_data_relevance: bool,
    /// Short rationale for the decision.
    pub reasoning: String,
}

/// Extracted user intent for a relevant email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailIntent {
    /// Short queue-friendly task title.
    pub task_title: String,
    /// One-paragraph summary of what the sender wants.
    pub intent_summary: String,
    /// Clear statement of the requested outcome.
    pub requested_outcome: String,
    /// Queue-ready task description.
    pub task_description: String,
    /// Sender email or name if it can be inferred.
    pub requester_identity: Option<String>,
    /// Entities, datasets, or systems mentioned by the sender.
    pub key_entities: Vec<String>,
    /// Whether the request is actionable in its current form.
    pub actionable: bool,
    /// Missing details that block execution.
    pub missing_information: Vec<String>,
}

/// Final validity decision for queue insertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssessment {
    /// Whether a task should be inserted into the queue.
    pub should_queue: bool,
    /// Whether a human should review before action.
    pub needs_human_review: bool,
    /// Clarity score `0..=100`.
    pub clarity_score: u8,
    /// Suggested priority for the queue item.
    #[serde(deserialize_with = "deserialize_priority")]
    #[serde(default = "default_priority")]
    pub priority: String,
    /// Validation rationale.
    pub rationale: String,
    /// Missing information that prevented a stronger decision.
    pub missing_information: Vec<String>,
}

fn default_priority() -> String {
    "normal".to_string()
}

fn deserialize_priority<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(|value| value.unwrap_or_else(default_priority))
}

/// Persistence result for a queue-worthy email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedJobRecord {
    /// Target backend.
    pub backend: QueueBackendKind,
    /// Queue status such as `queued`, `not_persisted`, or `persistence_failed`.
    pub status: String,
    /// External identifier returned by the backing store.
    pub external_id: Option<String>,
    /// Human-readable location for the created record.
    pub location: Option<String>,
    /// Time when the queue write was attempted.
    pub queued_at: DateTime<Utc>,
    /// Any persistence error to surface in the report.
    pub error: Option<String>,
}

/// End-to-end result for one processed email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailResponseRecord {
    /// Stage status such as `skipped_no_endpoint`, `generated`, `sent`,
    /// `send_failed`, or `generation_failed`.
    pub status: String,
    /// Endpoint used to generate the response, when configured.
    pub endpoint: Option<String>,
    /// Time when the response pipeline attempted generation or sending.
    pub attempted_at: DateTime<Utc>,
    /// Gmail message id for the sent response, if available.
    pub sent_message_id: Option<String>,
    /// Gmail thread id for the sent response, if available.
    pub sent_thread_id: Option<String>,
    /// Recipient address used for the outbound response.
    pub recipient: String,
    /// Subject that was used for the outbound response.
    pub subject: String,
    /// Optional short preview of the generated answer for audit traces.
    pub response_preview: Option<String>,
    /// Error details if the generation or sending step failed.
    pub error: Option<String>,
}

impl EmailResponseRecord {
    /// Create a failure record with context for the same email review path.
    pub fn failed(
        status: impl Into<String>,
        endpoint: Option<String>,
        recipient: String,
        subject: String,
        error: impl Into<String>,
    ) -> Self {
        Self {
            status: status.into(),
            endpoint,
            attempted_at: Utc::now(),
            sent_message_id: None,
            sent_thread_id: None,
            recipient,
            subject,
            response_preview: None,
            error: Some(error.into()),
        }
    }

    /// Create a success-like record for reply attempts that do not result in a sent
    /// message (for example explicit operator skip).
    pub fn completed(
        status: impl Into<String>,
        endpoint: Option<String>,
        recipient: String,
        subject: String,
        response_preview: Option<String>,
    ) -> Self {
        Self {
            status: status.into(),
            endpoint,
            attempted_at: Utc::now(),
            sent_message_id: None,
            sent_thread_id: None,
            recipient,
            subject,
            response_preview,
            error: None,
        }
    }
}

/// End-to-end result for one processed email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedEmail {
    /// Original Gmail message.
    pub message: GmailMessage,
    /// Relevance decision.
    pub classification: EmailClassification,
    /// Extracted intent, when relevant.
    pub intent: Option<EmailIntent>,
    /// Queue validation decision, when relevant.
    pub assessment: Option<TaskAssessment>,
    /// Persistence result for queue-worthy tasks.
    pub queued_job: Option<QueuedJobRecord>,
    /// Auto-responder result for queue-worthy emails.
    pub response: Option<EmailResponseRecord>,
    /// Final disposition such as `ignored`, `reviewed`, or `queued`.
    pub disposition: String,
}

/// Shared blackboard state passed between specialists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalContext {
    /// High-level objective for the current mission.
    pub objective: String,
    /// Shared execution history.
    pub conversation_history: Vec<String>,
    /// Current step count.
    pub step_count: u32,
    /// MarianneAI relevance profile.
    pub project_profile: ProjectProfile,
    /// Gmail search query for mailbox loading.
    pub query: String,
    /// Maximum number of emails to process.
    pub max_emails: u32,
    /// Selected queue backend.
    pub queue_backend: QueueBackendKind,
    /// Whether Gmail messages have already been loaded into the context.
    pub mailbox_loaded: bool,
    /// Loaded Gmail messages.
    pub emails: Vec<GmailMessage>,
    /// Zero-based index of the active email.
    pub current_email_index: usize,
    /// Current classification result.
    pub current_classification: Option<EmailClassification>,
    /// Current intent extraction result.
    pub current_intent: Option<EmailIntent>,
    /// Current queue validation result.
    pub current_validation: Option<TaskAssessment>,
    /// Current queue persistence result.
    pub current_queue_record: Option<QueuedJobRecord>,
    /// Finalized email results.
    pub reviewed_emails: Vec<ProcessedEmail>,
    /// Response endpoint URL configured for this run.
    pub answer_endpoint: Option<String>,
    /// Optional API key for the configured response endpoint.
    pub answer_endpoint_api_key: Option<String>,
    /// Timeout in seconds for answer endpoint calls.
    pub answer_endpoint_timeout_seconds: u64,
    /// URL of the Datagouv MCP endpoint used for additional answer sourcing.
    pub datagouv_mcp_endpoint: Option<String>,
    /// MCP tool name to invoke for Datagouv lookup.
    pub datagouv_mcp_tool: String,
    /// Timeout in seconds for Datagouv MCP requests.
    pub datagouv_mcp_timeout_seconds: u64,
    /// Optional markdown path where Datagouv query successes are persisted.
    pub datagouv_query_memory_path: Option<String>,
    /// Cached learned query fragments from prior successful MCP queries.
    pub datagouv_query_memory_cache: Vec<String>,
    /// Current pending response generation result.
    pub current_response: Option<EmailResponseRecord>,
}

impl GlobalContext {
    /// Build a new mission context with an empty blackboard.
    pub fn new(
        objective: String,
        query: String,
        max_emails: u32,
        queue_backend: QueueBackendKind,
    ) -> Self {
        Self {
            objective,
            conversation_history: Vec::new(),
            step_count: 0,
            project_profile: ProjectProfile::default(),
            query,
            max_emails,
            queue_backend,
            mailbox_loaded: false,
            emails: Vec::new(),
            current_email_index: 0,
            current_classification: None,
            current_intent: None,
            current_validation: None,
            current_queue_record: None,
            answer_endpoint: None,
            answer_endpoint_api_key: None,
            answer_endpoint_timeout_seconds: 30,
            datagouv_mcp_endpoint: None,
            datagouv_mcp_tool: "search_datasets".to_string(),
            datagouv_mcp_timeout_seconds: 30,
            datagouv_query_memory_path: None,
            datagouv_query_memory_cache: Vec::new(),
            current_response: None,
            reviewed_emails: Vec::new(),
        }
    }

    /// Returns the current email, if there is one.
    pub fn current_email(&self) -> Option<&GmailMessage> {
        self.emails.get(self.current_email_index)
    }

    /// Advances to the next email and clears the current staged outputs.
    pub fn advance_email(&mut self) {
        self.current_email_index += 1;
        self.current_classification = None;
        self.current_intent = None;
        self.current_validation = None;
        self.current_queue_record = None;
        self.current_response = None;
    }
}

/// Final mission report written to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionReport {
    /// Objective supplied to the orchestrator.
    pub objective: String,
    /// Query used to load Gmail messages.
    pub query: String,
    /// Queue backend used during the mission.
    pub queue_backend: QueueBackendKind,
    /// Number of emails loaded from Gmail.
    pub total_loaded: usize,
    /// Number of emails classified as MarianneAI-related.
    pub related_count: usize,
    /// Number of emails that were clear enough to be queued.
    pub queueable_count: usize,
    /// Number of emails that were actually persisted successfully.
    pub persisted_count: usize,
    /// Number of emails for which a response was successfully sent.
    pub responded_count: usize,
    /// Number of emails that attempted response generation.
    pub response_attempt_count: usize,
    /// Finalized results per email.
    pub reviewed_emails: Vec<ProcessedEmail>,
    /// Execution history for debugging and review.
    pub conversation_history: Vec<String>,
}
