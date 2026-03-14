//! Queue persistence for queue-worthy email tasks.
//! Backends are intentionally thin: keep schema assumptions local, keep decision
//! logic in the pipeline specialists, and keep queue persistence failures observable.

use crate::types::{
    EmailClassification, EmailIntent, GmailMessage, QueueBackendKind, QueuedJobRecord,
    TaskAssessment,
};
use chrono::Utc;
use serde_json::{Value, json};
use thiserror::Error;
use tokio_postgres::NoTls;

/// Queue writer abstraction for PostgreSQL and disabled mode.
pub enum TaskQueueWriter {
    /// PostgreSQL writer for relational deployments.
    Postgres(PostgresQueueWriter),
    /// Local mode that records queue-worthiness for dry runs without external writes.
    Disabled,
}

impl TaskQueueWriter {
    /// Initialize the selected backend from environment variables.
    pub fn from_backend(kind: QueueBackendKind) -> Result<Self, QueueError> {
        match kind {
            QueueBackendKind::None => Ok(Self::Disabled),
            QueueBackendKind::Postgres => Ok(Self::Postgres(PostgresQueueWriter::from_env()?)),
        }
    }

    /// Persist a queue item and return the resulting record.
    pub async fn enqueue(
        &self,
        message: &GmailMessage,
        classification: &EmailClassification,
        intent: &EmailIntent,
        assessment: &TaskAssessment,
    ) -> Result<QueuedJobRecord, QueueError> {
        match self {
            Self::Postgres(writer) => {
                writer
                    .enqueue(message, classification, intent, assessment)
                    .await
            }
            // Keep local behavior deterministic: explicitly report that no backend was selected
            // instead of silently dropping queue candidates.
            Self::Disabled => Ok(QueuedJobRecord {
                backend: QueueBackendKind::None,
                status: "not_persisted".to_string(),
                external_id: None,
                location: None,
                queued_at: Utc::now(),
                error: Some("QUEUE_BACKEND=none".to_string()),
            }),
        }
    }
}

/// PostgreSQL-backed queue writer.
pub struct PostgresQueueWriter {
    connection_string: String,
    table_name: String,
}

impl PostgresQueueWriter {
    fn from_env() -> Result<Self, QueueError> {
        Ok(Self {
            connection_string: required_env("POSTGRES_DSN")?,
            table_name: sanitize_identifier(
                &std::env::var("POSTGRES_QUEUE_TABLE").unwrap_or_else(|_| "job_queue".to_string()),
            )?,
        })
    }

    async fn enqueue(
        &self,
        message: &GmailMessage,
        classification: &EmailClassification,
        intent: &EmailIntent,
        assessment: &TaskAssessment,
    ) -> Result<QueuedJobRecord, QueueError> {
        let (client, connection) = tokio_postgres::connect(&self.connection_string, NoTls).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        client.batch_execute(&self.ensure_table_sql()).await?;

        let payload = queue_payload(message, classification, intent, assessment);
        let insert = format!(
            "INSERT INTO {table} (
                source,
                source_message_id,
                source_thread_id,
                requester_email,
                subject,
                category,
                intent_summary,
                task_title,
                task_description,
                clarity_score,
                priority,
                needs_human_review,
                raw_payload
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT (source_message_id) DO UPDATE SET
                requester_email = EXCLUDED.requester_email,
                subject = EXCLUDED.subject,
                category = EXCLUDED.category,
                intent_summary = EXCLUDED.intent_summary,
                task_title = EXCLUDED.task_title,
                task_description = EXCLUDED.task_description,
                clarity_score = EXCLUDED.clarity_score,
                priority = EXCLUDED.priority,
                needs_human_review = EXCLUDED.needs_human_review,
                raw_payload = EXCLUDED.raw_payload
            RETURNING id::text",
            table = self.table_name
        );

        let row = client
            .query_one(
                &insert,
                &[
                    &"gmail",
                    &message.message_id,
                    &message.thread_id,
                    &intent.requester_identity,
                    &message
                        .subject
                        .clone()
                        .unwrap_or_else(|| "(no subject)".to_string()),
                    &classification.category,
                    &intent.intent_summary,
                    &intent.task_title,
                    &intent.task_description,
                    &(assessment.clarity_score as i32),
                    &assessment.priority,
                    &assessment.needs_human_review,
                    &payload,
                ],
            )
            .await?;

        let id: String = row.get(0);
        Ok(QueuedJobRecord {
            backend: QueueBackendKind::Postgres,
            status: "queued".to_string(),
            external_id: Some(id.clone()),
            location: Some(format!("{}#{}", self.table_name, id)),
            queued_at: Utc::now(),
            error: None,
        })
    }

    fn ensure_table_sql(&self) -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                id BIGSERIAL PRIMARY KEY,
                source TEXT NOT NULL,
                source_message_id TEXT NOT NULL UNIQUE,
                source_thread_id TEXT NOT NULL,
                requester_email TEXT,
                subject TEXT NOT NULL,
                category TEXT NOT NULL,
                intent_summary TEXT NOT NULL,
                task_title TEXT NOT NULL,
                task_description TEXT NOT NULL,
                clarity_score INTEGER NOT NULL,
                priority TEXT NOT NULL,
                needs_human_review BOOLEAN NOT NULL,
                raw_payload JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );",
            table = self.table_name
        )
    }
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("missing required environment variable `{0}`")]
    MissingEnv(&'static str),

    #[error("queue configuration is invalid: {0}")]
    InvalidConfig(String),

    #[error("Postgres operation failed: {0}")]
    Postgres(#[from] tokio_postgres::Error),
}

fn queue_payload(
    message: &GmailMessage,
    classification: &EmailClassification,
    intent: &EmailIntent,
    assessment: &TaskAssessment,
) -> Value {
    json!({
        "project": "marianneai",
        "source": "gmail",
        "message_id": message.message_id,
        "thread_id": message.thread_id,
        "sender": message.from,
        "subject": message.subject,
        "classification": classification,
        "intent": intent,
        "assessment": assessment,
        "queued_at": Utc::now(),
    })
}

fn required_env(name: &'static str) -> Result<String, QueueError> {
    std::env::var(name).map_err(|_| QueueError::MissingEnv(name))
}

fn sanitize_identifier(input: &str) -> Result<String, QueueError> {
    if input.is_empty() {
        return Err(QueueError::InvalidConfig(
            "queue table name cannot be empty".to_string(),
        ));
    }

    if input
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Ok(input.to_string())
    } else {
        Err(QueueError::InvalidConfig(format!(
            "queue table name `{}` contains unsupported characters",
            input
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_identifier;

    #[test]
    fn accepts_safe_table_names() {
        assert_eq!(
            sanitize_identifier("job_queue_2026").expect("identifier should be valid"),
            "job_queue_2026"
        );
    }

    #[test]
    fn rejects_unsafe_table_names() {
        assert!(sanitize_identifier("job-queue").is_err());
    }
}
