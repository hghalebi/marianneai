//! Multi-agent orchestration for MarianneAI email triage.
//! The orchestrator writes a blackboard context for each step so each specialist
//! can make a decision using shared state only, with no cross-agent mutable state.

use crate::compat::{WasmCompatSend, WasmCompatSync};
use crate::gmail::{GmailClient, GmailError};
use crate::queue::{QueueError, TaskQueueWriter};
use crate::types::{
    EmailClassification, EmailIntent, EmailResponseRecord, GlobalContext, MissionReport,
    ProcessedEmail, QueueBackendKind, TaskAssessment,
};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::{Client as HttpClient, header};
use rig::agent::AgentBuilder;
use rig::completion::Prompt;
use rig::providers::gemini;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Specialized Rig/Gemini agent alias used throughout the pipeline.
pub type EmailAgent = rig::agent::Agent<gemini::CompletionModel>;

/// The result of one specialist turn in the coordinator loop.
#[derive(Debug)]
pub enum TurnResult {
    /// Continue the current agent on another turn.
    KeepWorking {
        /// Human-readable reasoning for logs.
        thought: String,
        /// Updated context.
        new_context: GlobalContext,
    },
    /// Delegate control to another specialist.
    Delegate {
        /// Registered target agent name.
        target_agent: String,
        /// Instruction for the next specialist.
        instruction: String,
        /// Updated context.
        new_context: GlobalContext,
    },
    /// End the mission.
    FinalResult(String),
}

/// Specialist interface for the supervisor-worker pipeline.
#[async_trait]
pub trait Specialist: WasmCompatSend + WasmCompatSync {
    /// Stable routing name for the agent.
    fn name(&self) -> &str;

    /// Execute one turn.
    async fn run_turn(&self, ctx: GlobalContext) -> Result<TurnResult, AgentError>;
}

/// Multi-agent orchestrator modeled after the industrial document analyzer flow.
pub struct Orchestrator {
    agents: HashMap<String, Box<dyn Specialist>>,
    /// Hard stop for pathological loops; prevents infinite specialist handoffs.
    max_steps: u32,
}

impl Orchestrator {
    /// Create a new orchestrator with the default step limit.
    pub fn new(agents: HashMap<String, Box<dyn Specialist>>) -> Self {
        Self {
            agents,
            max_steps: 64,
        }
    }

    /// Run the mission until completion or step exhaustion.
    pub async fn run_mission(
        &self,
        mut context: GlobalContext,
        output_dir: PathBuf,
    ) -> Result<String, AgentError> {
        fs::create_dir_all(&output_dir)?;
        let mut current_agent_name = "supervisor".to_string();

        while context.step_count < self.max_steps {
            println!("STEP {} | agent={}", context.step_count, current_agent_name);
            if let Some(active_email) = context.current_email() {
                println!(
                    "  active_email index={} from={} subject={}",
                    context.current_email_index,
                    active_email.from.as_deref().unwrap_or("<unknown>"),
                    active_email.subject.as_deref().unwrap_or("(no subject)")
                );
            } else if context.mailbox_loaded {
                println!("  active_email none (all emails consumed)");
            } else {
                println!("  active_email not loaded yet");
            }

            let agent = match self.agents.get(&current_agent_name) {
                Some(agent) => agent,
                None => {
                    return Err(AgentError::UnknownAgent(current_agent_name));
                }
            };

            let result = match agent.run_turn(context.clone()).await {
                Ok(result) => result,
                Err(error) => {
                    context.conversation_history.push(format!(
                        "SYSTEM: {} failed: {}",
                        agent.name(),
                        error
                    ));
                    self.save_snapshot(&context, agent.name(), &output_dir)?;
                    return Err(error);
                }
            };

            match result {
                TurnResult::KeepWorking {
                    thought,
                    new_context,
                } => {
                    context = new_context;
                    println!("  action: keep_working: {thought}");
                    context.conversation_history.push(format!(
                        "SYSTEM: {} is still working: {}",
                        agent.name(),
                        thought
                    ));
                }
                TurnResult::Delegate {
                    target_agent,
                    instruction,
                    new_context,
                } => {
                    context = new_context;
                    println!("  action: delegate -> {target_agent} | instruction={instruction}");
                    context.conversation_history.push(format!(
                        "SYSTEM: {} delegated to {} with instruction: {}",
                        agent.name(),
                        target_agent,
                        instruction
                    ));
                    current_agent_name = target_agent;
                }
                TurnResult::FinalResult(summary) => {
                    let report = build_report(&context);
                    save_final_report(&report, &summary, &output_dir)?;
                    println!("  action: final_result summary={summary}");
                    return Ok(summary);
                }
            }

            if let Some(last) = context.conversation_history.last() {
                println!("  note: {last}");
            }

            context.step_count += 1;
            self.save_snapshot(&context, &current_agent_name, &output_dir)?;
        }

        Err(AgentError::MaxStepsReached(self.max_steps))
    }

    /// Persist a full context snapshot after each transition for replay and post-mortem debugging.
    fn save_snapshot(
        &self,
        context: &GlobalContext,
        current_agent: &str,
        output_dir: &Path,
    ) -> Result<(), AgentError> {
        let path = output_dir.join(format!("step_{:04}_context.json", context.step_count));
        let snapshot = serde_json::json!({
            "current_agent": current_agent,
            "timestamp": Utc::now(),
            "context": context,
        });
        fs::write(path, serde_json::to_string_pretty(&snapshot)?)?;
        Ok(())
    }
}

/// Top-level agent and orchestration errors.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("context error: {0}")]
    Context(String),

    #[error("unknown agent `{0}`")]
    UnknownAgent(String),

    #[error("mission exceeded max steps ({0})")]
    MaxStepsReached(u32),

    #[error("Gmail operation failed: {0}")]
    Gmail(#[from] GmailError),

    #[error("queue operation failed: {0}")]
    Queue(#[from] QueueError),

    #[error("model completion failed: {0}")]
    Prompt(#[from] rig::completion::PromptError),

    #[error("failed to parse structured JSON from {agent}: {source}. Response: {response}")]
    JsonResponse {
        agent: &'static str,
        source: serde_json::Error,
        response: String,
    },

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to call answer endpoint: {endpoint} - {source}")]
    AnswerEndpoint {
        endpoint: String,
        source: reqwest::Error,
    },

    #[error("answer endpoint returned no usable text")]
    AnswerEndpointEmpty,

    #[error("Datagouv MCP call failed: {endpoint} - {source}")]
    DatagouvMcp {
        endpoint: String,
        source: reqwest::Error,
    },

    #[error("Datagouv MCP returned an error: {tool}")]
    DatagouvToolError { tool: String, message: String },
}

/// Deterministic supervisor mirroring the industrial pipeline.
pub struct Supervisor;

#[async_trait]
impl Specialist for Supervisor {
    fn name(&self) -> &str {
        "supervisor"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        if !ctx.mailbox_loaded {
            return Ok(TurnResult::Delegate {
                target_agent: "mailbox_reader".to_string(),
                instruction: format!(
                    "Load up to {} Gmail messages matching `{}`.",
                    ctx.max_emails, ctx.query
                ),
                new_context: ctx,
            });
        }

        if ctx.current_email_index >= ctx.emails.len() {
            return Ok(TurnResult::Delegate {
                target_agent: "auditor".to_string(),
                instruction: "All available emails have been processed.".to_string(),
                new_context: ctx,
            });
        }

        if ctx.current_classification.is_none() {
            return Ok(TurnResult::Delegate {
                target_agent: "relevance_classifier".to_string(),
                instruction: "Classify whether the current email is relevant to MarianneAI."
                    .to_string(),
                new_context: ctx,
            });
        }

        if let Some(classification) = &ctx.current_classification
            && !classification.related_to_project
        {
            finalize_current_email(&mut ctx, "ignored");
            return Ok(TurnResult::Delegate {
                target_agent: "supervisor".to_string(),
                instruction: "The current email is not related to MarianneAI.".to_string(),
                new_context: ctx,
            });
        }

        if ctx.current_intent.is_none() {
            return Ok(TurnResult::Delegate {
                target_agent: "intent_extractor".to_string(),
                instruction: "Extract the sender intent from the current MarianneAI email."
                    .to_string(),
                new_context: ctx,
            });
        }

        if ctx.current_validation.is_none() {
            return Ok(TurnResult::Delegate {
                target_agent: "task_validator".to_string(),
                instruction:
                    "Decide whether the extracted intent is clear enough to become a queued task."
                        .to_string(),
                new_context: ctx,
            });
        }

        if ctx
            .current_validation
            .as_ref()
            .is_some_and(|assessment| assessment.should_queue)
            && ctx.current_queue_record.is_none()
        {
            return Ok(TurnResult::Delegate {
                target_agent: "queue_writer".to_string(),
                instruction: format!(
                    "Persist the validated task to the {:?} queue backend.",
                    ctx.queue_backend
                ),
                new_context: ctx,
            });
        }

        if ctx.current_validation.is_some() && ctx.current_response.is_none() {
            if ctx.answer_endpoint.is_none() && ctx.datagouv_mcp_endpoint.is_some() {
                return Ok(TurnResult::Delegate {
                    target_agent: "datagouv_responder".to_string(),
                    instruction:
                        "Prepare an answer for this related email using Datagouv MCP data tools."
                            .to_string(),
                    new_context: ctx,
                });
            }

            return Ok(TurnResult::Delegate {
                target_agent: "answer_responder".to_string(),
                instruction: "Prepare an answer for this related email.".to_string(),
                new_context: ctx,
            });
        }

        let disposition = if ctx
            .current_validation
            .as_ref()
            .is_some_and(|assessment| assessment.should_queue)
        {
            "queued"
        } else {
            "reviewed"
        };

        finalize_current_email(&mut ctx, disposition);
        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Continue with the next email.".to_string(),
            new_context: ctx,
        })
    }
}

/// Loads Gmail messages into the blackboard context.
pub struct MailboxReader {
    gmail: GmailClient,
}

impl MailboxReader {
    /// Create a new mailbox reader.
    pub fn new(gmail: GmailClient) -> Self {
        Self { gmail }
    }
}

#[async_trait]
impl Specialist for MailboxReader {
    fn name(&self) -> &str {
        "mailbox_reader"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let messages = self.gmail.fetch_messages(&ctx.query, ctx.max_emails)?;
        ctx.mailbox_loaded = true;
        ctx.emails = messages;
        println!(
            "MailboxReader: loaded {} email(s) for query `{}`",
            ctx.emails.len(),
            ctx.query
        );
        ctx.conversation_history.push(format!(
            "MailboxReader: loaded {} email(s) for query `{}`",
            ctx.emails.len(),
            ctx.query
        ));

        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Mailbox loaded.".to_string(),
            new_context: ctx,
        })
    }
}

/// LLM specialist that decides whether an email is about MarianneAI.
pub struct RelevanceClassifier {
    inner: EmailAgent,
}

impl RelevanceClassifier {
    /// Build the classifier agent.
    pub fn new(model: gemini::CompletionModel) -> Self {
        let preamble = r#"You classify inbound emails for MarianneAI.

Return JSON only with this exact shape:
{
  "related_to_project": true,
  "category": "feature_request",
  "confidence": 0.91,
  "public_data_relevance": true,
  "reasoning": "One short paragraph."
}

Categories should be short snake_case labels like:
feature_request, support_request, demo_request, partnership, bug_report, press, feedback, spam, unrelated.

An email is related if it concerns MarianneAI, its public-data mission, its product, its website, support, demos, pilots, partnerships, or requests to analyze public data."#;

        let inner = AgentBuilder::new(model)
            .preamble(preamble)
            .max_tokens(800)
            .build();
        Self { inner }
    }
}

#[async_trait]
impl Specialist for RelevanceClassifier {
    fn name(&self) -> &str {
        "relevance_classifier"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx
            .current_email()
            .cloned()
            .ok_or_else(|| AgentError::Context("no active email to classify".to_string()))?;

        let prompt = format!(
            "PROJECT\nname: {}\nabout: {}\nwebsite: {}\nrelevance_hints: {:?}\n\nEMAIL\nfrom: {:?}\nsubject: {:?}\ndate: {:?}\nbody:\n{}",
            ctx.project_profile.name,
            ctx.project_profile.about,
            ctx.project_profile.website,
            ctx.project_profile.relevance_hints,
            message.from,
            message.subject,
            message.date,
            truncate_text(&message.prompt_body(), 8_000),
        );

        let response = self.inner.prompt(&prompt).await?;
        let classification: EmailClassification =
            parse_structured_json("relevance_classifier", &response)?;
        println!(
            "RelevanceClassifier: from {:?}, subject {:?}, related={} category={} confidence={:.2}",
            message.from,
            message.subject,
            classification.related_to_project,
            classification.category,
            classification.confidence
        );

        ctx.conversation_history.push(format!(
            "RelevanceClassifier: evaluating email from {:?} subject={:?}",
            message.from, message.subject
        ));
        ctx.conversation_history.push(format!(
            "RelevanceClassifier: related={} category={} confidence={:.2}",
            classification.related_to_project, classification.category, classification.confidence
        ));
        ctx.current_classification = Some(classification);

        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Relevance classification complete.".to_string(),
            new_context: ctx,
        })
    }
}

/// LLM specialist that extracts sender intent from a relevant email.
pub struct IntentExtractor {
    inner: EmailAgent,
}

impl IntentExtractor {
    /// Build the intent extraction agent.
    pub fn new(model: gemini::CompletionModel) -> Self {
        let preamble = r#"You extract user intent from MarianneAI emails.

Return JSON only with this exact shape:
{
  "task_title": "Short title",
  "intent_summary": "Short paragraph",
  "requested_outcome": "What the sender wants",
  "task_description": "Queue-ready task description",
  "requester_identity": "email@example.com",
  "key_entities": ["entity"],
  "actionable": true,
  "missing_information": ["detail still missing"]
}

Focus on the sender's real intention, not your answer to them."#;

        let inner = AgentBuilder::new(model)
            .preamble(preamble)
            .max_tokens(1_000)
            .build();
        Self { inner }
    }
}

#[async_trait]
impl Specialist for IntentExtractor {
    fn name(&self) -> &str {
        "intent_extractor"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx.current_email().cloned().ok_or_else(|| {
            AgentError::Context("no active email to extract intent from".to_string())
        })?;
        let classification = ctx.current_classification.clone().ok_or_else(|| {
            AgentError::Context("intent extraction requires a classification".to_string())
        })?;

        let prompt = format!(
            "PROJECT ABOUT\n{}\n\nCLASSIFICATION\n{}\n\nEMAIL\nfrom: {:?}\nsubject: {:?}\nbody:\n{}",
            ctx.project_profile.about,
            serde_json::to_string_pretty(&classification)?,
            message.from,
            message.subject,
            truncate_text(&message.prompt_body(), 10_000),
        );

        let response = self.inner.prompt(&prompt).await?;
        let intent: EmailIntent = parse_structured_json("intent_extractor", &response)?;
        println!(
            "IntentExtractor: title={} actionable={}",
            intent.task_title, intent.actionable
        );

        ctx.conversation_history.push(format!(
            "IntentExtractor: message={:?} extraction_started",
            message.subject
        ));
        ctx.conversation_history.push(format!(
            "IntentExtractor: title={} actionable={}",
            intent.task_title, intent.actionable
        ));
        ctx.current_intent = Some(intent);

        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Intent extraction complete.".to_string(),
            new_context: ctx,
        })
    }
}

/// LLM quality gate that decides whether the extracted intent should enter the queue.
pub struct TaskValidator {
    inner: EmailAgent,
}

impl TaskValidator {
    /// Build the validator agent.
    pub fn new(model: gemini::CompletionModel) -> Self {
        let preamble = r#"You are the task-quality gate for MarianneAI.

Decide whether an inbound email is clear enough to become a real job queue item.

Return JSON only with this exact shape:
{
  "should_queue": true,
  "needs_human_review": false,
  "clarity_score": 82,
  "priority": "normal",
  "rationale": "Short paragraph",
  "missing_information": ["detail still missing"]
}

Set should_queue=true only when the request is relevant, sufficiently specific, and actionable without guessing the core job."#;

        let inner = AgentBuilder::new(model)
            .preamble(preamble)
            .max_tokens(900)
            .build();
        Self { inner }
    }
}

#[async_trait]
impl Specialist for TaskValidator {
    fn name(&self) -> &str {
        "task_validator"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx
            .current_email()
            .cloned()
            .ok_or_else(|| AgentError::Context("no active email to validate".to_string()))?;
        let classification = ctx.current_classification.clone().ok_or_else(|| {
            AgentError::Context("task validation requires a classification".to_string())
        })?;
        let intent = ctx.current_intent.clone().ok_or_else(|| {
            AgentError::Context("task validation requires extracted intent".to_string())
        })?;

        let prompt = format!(
            "VALID TASK CRITERIA\n{:?}\n\nEMAIL\nfrom: {:?}\nsubject: {:?}\n\nCLASSIFICATION\n{}\n\nINTENT\n{}",
            ctx.project_profile.valid_task_criteria,
            message.from,
            message.subject,
            serde_json::to_string_pretty(&classification)?,
            serde_json::to_string_pretty(&intent)?,
        );

        let response = self.inner.prompt(&prompt).await?;
        let assessment: TaskAssessment = parse_structured_json("task_validator", &response)?;
        println!(
            "TaskValidator: should_queue={} clarity={} priority={}",
            assessment.should_queue, assessment.clarity_score, assessment.priority
        );

        ctx.conversation_history.push(format!(
            "TaskValidator: evaluating message={:?} for clarity and actionability",
            message.subject
        ));
        ctx.conversation_history.push(format!(
            "TaskValidator: should_queue={} clarity_score={}",
            assessment.should_queue, assessment.clarity_score
        ));
        ctx.current_validation = Some(assessment);

        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Task validation complete.".to_string(),
            new_context: ctx,
        })
    }
}

/// Persists queue-worthy tasks to the configured backend.
pub struct QueueWriterAgent {
    writer: TaskQueueWriter,
}

impl QueueWriterAgent {
    /// Create the queue writer agent from the selected backend.
    pub fn new(writer: TaskQueueWriter) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl Specialist for QueueWriterAgent {
    fn name(&self) -> &str {
        "queue_writer"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx
            .current_email()
            .cloned()
            .ok_or_else(|| AgentError::Context("no active email to queue".to_string()))?;
        let classification = ctx.current_classification.clone().ok_or_else(|| {
            AgentError::Context("queue persistence requires classification".to_string())
        })?;
        let intent = ctx
            .current_intent
            .clone()
            .ok_or_else(|| AgentError::Context("queue persistence requires intent".to_string()))?;
        let assessment = ctx.current_validation.clone().ok_or_else(|| {
            AgentError::Context("queue persistence requires validation".to_string())
        })?;

        let record = match self
            .writer
            .enqueue(&message, &classification, &intent, &assessment)
            .await
        {
            Ok(record) => record,
            Err(error) => crate::types::QueuedJobRecord {
                backend: ctx.queue_backend,
                status: "persistence_failed".to_string(),
                external_id: None,
                location: None,
                queued_at: Utc::now(),
                error: Some(error.to_string()),
            },
        };

        ctx.conversation_history.push(format!(
            "QueueWriter: backend={:?} status={}",
            record.backend, record.status
        ));
        println!(
            "QueueWriter: intent={} backend={:?} status={}",
            intent.task_title, record.backend, record.status
        );
        ctx.current_queue_record = Some(record);

        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Queue persistence complete.".to_string(),
            new_context: ctx,
        })
    }
}

/// Builds response payloads and sends generated answers back to the sender.
pub struct AnswerResponder {
    http_client: HttpClient,
    gmail: GmailClient,
}

impl AnswerResponder {
    /// Create the responder stage.
    pub fn new() -> Self {
        Self {
            http_client: HttpClient::new(),
            gmail: GmailClient,
        }
    }
}

impl Default for AnswerResponder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Specialist for AnswerResponder {
    fn name(&self) -> &str {
        "answer_responder"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx
            .current_email()
            .cloned()
            .ok_or_else(|| AgentError::Context("no active email to respond to".to_string()))?;

        let classification = ctx
            .current_classification
            .clone()
            .ok_or_else(|| AgentError::Context("response requires classification".to_string()))?;
        let intent = ctx
            .current_intent
            .clone()
            .ok_or_else(|| AgentError::Context("response requires extracted intent".to_string()))?;
        let assessment = ctx
            .current_validation
            .clone()
            .ok_or_else(|| AgentError::Context("response requires validation".to_string()))?;

        let recipient = match self.gmail.infer_reply_recipient(&message) {
            Some(recipient) => recipient,
            None => {
                ctx.current_response = Some(EmailResponseRecord::failed(
                    "failed_no_recipient",
                    ctx.answer_endpoint.clone(),
                    message
                        .from
                        .unwrap_or_else(|| "<unknown sender>".to_string()),
                    build_reply_subject(message.subject.as_deref()),
                    "unable to infer reply recipient from the From header",
                ));

                finalize_current_email(&mut ctx, "response_skipped");
                return Ok(TurnResult::Delegate {
                    target_agent: "supervisor".to_string(),
                    instruction: "Response skipped due to missing sender address.".to_string(),
                    new_context: ctx,
                });
            }
        };

        let reply_subject = build_reply_subject(message.subject.as_deref());
        let instruction = build_answer_instruction(
            &ctx.project_profile,
            &message,
            &classification,
            &intent,
            &assessment,
            reply_subject.as_str(),
        );

        let response = if let Some(endpoint) = ctx.answer_endpoint.clone() {
            match generate_answer(
                &self.http_client,
                &endpoint,
                ctx.answer_endpoint_api_key.as_deref(),
                &instruction,
                ctx.answer_endpoint_timeout_seconds,
            )
            .await
            {
                Ok(generated) => {
                    if is_error_like_reply(&generated) {
                        let reason =
                            "answer endpoint returned an error-like payload instead of an answer";
                        println!(
                            "AnswerResponder: generated reply rejected by validation: {reason}"
                        );
                        ctx.conversation_history.push(
                            "AnswerResponder: generated payload rejected as error-like".to_string(),
                        );
                        let fallback =
                            build_funny_fallback_message(Some(reason), "answer endpoint");
                        send_with_fallback(
                            &self.gmail,
                            &recipient,
                            &reply_subject,
                            &message,
                            &fallback,
                            Some(&endpoint),
                            Some(reason),
                        )
                        .await?
                    } else {
                        println!(
                            "AnswerResponder: generated draft preview: {}",
                            preview_text(&generated, 240)
                        );
                        ctx.conversation_history.push(format!(
                            "AnswerResponder: generated draft preview={}",
                            preview_text(&generated, 400)
                        ));
                        send_with_fallback(
                            &self.gmail,
                            &recipient,
                            &reply_subject,
                            &message,
                            &generated,
                            Some(&endpoint),
                            None,
                        )
                        .await?
                    }
                }
                Err(error) => {
                    let reason = format!("answer endpoint failed: {error}");
                    println!("AnswerResponder fallback reason: {reason}");
                    let fallback = build_funny_fallback_message(Some(&reason), "answer endpoint");
                    ctx.conversation_history.push(format!(
                        "AnswerResponder: using fallback because generation failed: {reason}"
                    ));
                    send_with_fallback(
                        &self.gmail,
                        &recipient,
                        &reply_subject,
                        &message,
                        &fallback,
                        Some(&endpoint),
                        Some(&reason),
                    )
                    .await?
                }
            }
        } else {
            let fallback = build_funny_fallback_message(
                Some("No ANSWER_ENDPOINT_URL configured. Sending temporary busy response."),
                "answer endpoint",
            );
            println!("AnswerResponder using busy fallback: no ANSWER_ENDPOINT_URL configured");
            ctx.conversation_history.push(
                "AnswerResponder: no endpoint configured, using temporary busy reply".to_string(),
            );
            send_with_fallback(
                &self.gmail,
                &recipient,
                &reply_subject,
                &message,
                &fallback,
                None,
                Some("No answer endpoint configured"),
            )
            .await?
        };

        ctx.current_response = Some(response);

        let next_disposition = if ctx.current_response.as_ref().is_some_and(|record| {
            matches!(
                record.status.as_str(),
                "sent" | "sent_without_endpoint" | "sent_fallback"
            )
        }) {
            "responded"
        } else {
            "reviewed"
        };
        finalize_current_email(&mut ctx, next_disposition);
        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Response attempt complete.".to_string(),
            new_context: ctx,
        })
    }
}

/// Builds response payloads from Datagouv MCP and sends generated answers back to sender.
pub struct DatagouvResponder {
    datagouv_http_client: HttpClient,
    gmail: GmailClient,
}

#[derive(Debug, Clone)]
struct DatagouvQueryCandidate {
    /// Human-readable label used in logs.
    source: &'static str,
    /// Query string sent to Datagouv MCP.
    query: String,
}

impl DatagouvResponder {
    /// Create the Datagouv MCP responder.
    pub fn new() -> Self {
        Self {
            datagouv_http_client: HttpClient::new(),
            gmail: GmailClient,
        }
    }

    fn is_tool_call_enabled(&self, endpoint: &Option<String>) -> bool {
        endpoint.as_deref().is_some_and(|value| !value.is_empty())
    }

    fn response_subject(&self, subject: Option<&str>) -> String {
        build_reply_subject(subject)
    }

    fn query_variants(
        &self,
        profile: &crate::types::ProjectProfile,
        message: &crate::types::GmailMessage,
        classification: &EmailClassification,
        intent: &EmailIntent,
        assessment: &TaskAssessment,
        memory: &[String],
    ) -> Vec<DatagouvQueryCandidate> {
        let primary = build_datagouv_query(profile, message, classification, intent, assessment);
        let keyword_only = build_datagouv_fallback_query(profile, message, classification, intent);
        let focus =
            build_datagouv_focus_query(profile, message, classification, intent, assessment);
        let keyword_only_short =
            build_datagouv_keyword_only_query(profile, message, classification, intent);

        let mut candidates = vec![
            DatagouvQueryCandidate {
                source: "primary_structured_query",
                query: primary,
            },
            DatagouvQueryCandidate {
                source: "focus_query",
                query: focus,
            },
            DatagouvQueryCandidate {
                source: "shorter_keyword_query",
                query: keyword_only,
            },
            DatagouvQueryCandidate {
                source: "minimal_keywords_only",
                query: keyword_only_short,
            },
        ];

        for entry in memory.iter().take(3) {
            let learned = format!(
                "French public data datasets | learned_terms: {entry} | project: {}",
                normalize_search_text(&profile.name)
            );
            candidates.push(DatagouvQueryCandidate {
                source: "learned_memory",
                query: learned,
            });
        }

        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for candidate in candidates {
            if candidate.query.trim().is_empty() {
                continue;
            }

            let key = candidate.query.to_ascii_lowercase();
            if seen.insert(key) {
                deduped.push(candidate);
            }
        }

        deduped.truncate(8);
        deduped
    }

    async fn answer_with_retries(
        &self,
        endpoint: &str,
        tool_name: &str,
        timeout_seconds: u64,
        variants: Vec<DatagouvQueryCandidate>,
        ctx: &mut GlobalContext,
    ) -> Result<(String, DatagouvQueryCandidate), String> {
        let mut last_error: Option<String> = None;
        let mut join_set = tokio::task::JoinSet::new();
        let datagouv_http_client = self.datagouv_http_client.clone();
        let endpoint = endpoint.to_string();
        let tool_name = tool_name.to_string();

        for variant in variants {
            let query = variant.query.clone();
            println!(
                "DatagouvResponder: launching {} variant => {}",
                variant.source, query
            );
            ctx.conversation_history.push(format!(
                "DatagouvResponder: launching query variant [{}] {}",
                variant.source, query
            ));

            let source = variant.source;
            let client = datagouv_http_client.clone();
            let endpoint = endpoint.clone();
            let tool_name = tool_name.clone();

            join_set.spawn(async move {
                (
                    source,
                    query.clone(),
                    query_datagouv_mcp(&client, &endpoint, &tool_name, timeout_seconds, &query)
                        .await,
                )
            });
        }

        while let Some(finished) = join_set.join_next().await {
            let (source, query, reply) = match finished {
                Ok(attempt) => attempt,
                Err(error) => {
                    let reason = format!("Datagouv variant task failed to run: {error}");
                    last_error = Some(reason.clone());
                    ctx.conversation_history
                        .push(format!("DatagouvResponder: {reason}"));
                    println!("DatagouvResponder: {reason}");
                    continue;
                }
            };

            match reply {
                Ok(candidate)
                    if !is_empty_datagouv_result(&candidate)
                        && !is_error_like_reply(&candidate) =>
                {
                    let summary = truncate_text(&candidate, 160);
                    println!(
                        "DatagouvResponder: source {source} succeeded with usable result: {summary}"
                    );
                    ctx.conversation_history.push(format!(
                        "DatagouvResponder: source {} returned usable result ({summary})",
                        source
                    ));
                    join_set.abort_all();
                    return Ok((candidate, DatagouvQueryCandidate { source, query }));
                }
                Ok(candidate) => {
                    let reason = format!(
                        "candidate response was empty/error-like for source {source}: {}",
                        truncate_text(&candidate, 160)
                    );
                    last_error = Some(reason.clone());
                    ctx.conversation_history.push(format!(
                        "DatagouvResponder: query variant [{}] returned unusable answer",
                        source
                    ));
                    println!("DatagouvResponder: {reason}");
                }
                Err(error) => {
                    let reason = format!("query variant [{}] failed with {error}", source);
                    last_error = Some(reason.clone());
                    ctx.conversation_history
                        .push(format!("DatagouvResponder: {reason}"));
                    println!("DatagouvResponder: {reason}");
                }
            }
        }

        let reason = last_error.unwrap_or_else(|| "no usable candidate was returned".to_string());
        Err(reason)
    }
}

impl Default for DatagouvResponder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Specialist for DatagouvResponder {
    fn name(&self) -> &str {
        "datagouv_responder"
    }

    async fn run_turn(&self, mut ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let message = ctx
            .current_email()
            .cloned()
            .ok_or_else(|| AgentError::Context("no active email to respond to".to_string()))?;

        let classification = ctx
            .current_classification
            .clone()
            .ok_or_else(|| AgentError::Context("response requires classification".to_string()))?;
        let intent = ctx
            .current_intent
            .clone()
            .ok_or_else(|| AgentError::Context("response requires extracted intent".to_string()))?;
        let assessment = ctx
            .current_validation
            .clone()
            .ok_or_else(|| AgentError::Context("response requires validation".to_string()))?;

        let recipient = match self.gmail.infer_reply_recipient(&message) {
            Some(recipient) => recipient,
            None => {
                ctx.current_response = Some(EmailResponseRecord::failed(
                    "failed_no_recipient",
                    ctx.datagouv_mcp_endpoint.clone(),
                    message
                        .from
                        .unwrap_or_else(|| "<unknown sender>".to_string()),
                    self.response_subject(message.subject.as_deref()),
                    "unable to infer reply recipient from the From header",
                ));

                finalize_current_email(&mut ctx, "response_skipped");
                return Ok(TurnResult::Delegate {
                    target_agent: "supervisor".to_string(),
                    instruction: "Response skipped due to missing sender address.".to_string(),
                    new_context: ctx,
                });
            }
        };

        let endpoint = ctx
            .datagouv_mcp_endpoint
            .clone()
            .filter(|value| !value.is_empty());
        if !self.is_tool_call_enabled(&endpoint) {
            ctx.current_response = Some(EmailResponseRecord::failed(
                "datagouv_skipped",
                None,
                recipient,
                self.response_subject(message.subject.as_deref()),
                "datagouv_mcp endpoint is not configured",
            ));
            finalize_current_email(&mut ctx, "reviewed");
            return Ok(TurnResult::Delegate {
                target_agent: "supervisor".to_string(),
                instruction:
                    "Datagouv responder skipped because DATAGOUV_MCP_ENDPOINT is not configured."
                        .to_string(),
                new_context: ctx,
            });
        }

        let reply_subject = self.response_subject(message.subject.as_deref());
        let learned_memory = load_datagouv_memory_hints(ctx.datagouv_query_memory_path.as_deref());
        ctx.datagouv_query_memory_cache = learned_memory.clone();
        let query_variants = self.query_variants(
            &ctx.project_profile,
            &message,
            &classification,
            &intent,
            &assessment,
            &learned_memory,
        );
        let tool_name: String = if ctx.datagouv_mcp_tool.trim().is_empty() {
            "search_datasets".to_string()
        } else {
            ctx.datagouv_mcp_tool.clone()
        };

        let endpoint_ref = endpoint.as_deref().expect("endpoint filtered above");
        let reply_with_source = self
            .answer_with_retries(
                endpoint_ref,
                tool_name.as_str(),
                ctx.datagouv_mcp_timeout_seconds,
                query_variants,
                &mut ctx,
            )
            .await;

        let (reply_body, _chosen_variant) = match reply_with_source {
            Ok((body, chosen_variant)) => {
                let memory_path = ctx.datagouv_query_memory_path.as_deref();
                let memory_entry = build_datagouv_memory_entry(
                    &message,
                    &classification,
                    &intent,
                    &chosen_variant,
                    &body,
                );
                if let Err(memory_error) = append_datagouv_memory(memory_path, &memory_entry) {
                    ctx.conversation_history.push(format!(
                        "DatagouvResponder: failed to write memory entry: {memory_error}"
                    ));
                } else {
                    ctx.conversation_history.push(format!(
                        "DatagouvResponder: persisted successful query pattern from {} variant",
                        chosen_variant.source
                    ));
                }
                (body, chosen_variant)
            }
            Err(error) => {
                println!("DatagouvResponder all-query strategy failed: {error}");
                ctx.conversation_history.push(format!(
                    "DatagouvResponder: all-query variants failed or returned unusable answers ({error})"
                ));
                (
                    build_funny_fallback_message(
                        Some("Datagouv MCP did not return a useful result."),
                        "Datagouv MCP",
                    ),
                    DatagouvQueryCandidate {
                        source: "fallback_reply",
                        query: "n/a".to_string(),
                    },
                )
            }
        };

        println!(
            "DatagouvResponder response preview: {}",
            preview_text(&reply_body, 240)
        );

        let endpoint_name = endpoint.as_deref();
        let response = send_with_fallback(
            &self.gmail,
            &recipient,
            &reply_subject,
            &message,
            &reply_body,
            endpoint_name,
            None,
        )
        .await?;

        ctx.current_response = Some(response);
        let next_disposition = if ctx.current_response.as_ref().is_some_and(|record| {
            matches!(
                record.status.as_str(),
                "sent" | "sent_without_endpoint" | "sent_fallback"
            )
        }) {
            "responded"
        } else {
            "reviewed"
        };
        finalize_current_email(&mut ctx, next_disposition);
        Ok(TurnResult::Delegate {
            target_agent: "supervisor".to_string(),
            instruction: "Datagouv response attempt complete.".to_string(),
            new_context: ctx,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct AnswerInstruction {
    /// Subject that should be used for the outbound email.
    reply_subject: String,
    /// Project name from the active profile.
    project_name: String,
    /// Project mission statement.
    project_about: String,
    /// Project website.
    project_website: String,
    /// Normalized sender string used for context.
    sender: String,
    /// The original subject line to preserve context.
    original_subject: Option<String>,
    /// A short excerpt used by the endpoint for context grounding.
    original_message_excerpt: String,
    /// Classification summary from the prior stage.
    classification: EmailClassification,
    /// Parsed intent from the prior stage.
    intent: EmailIntent,
    /// Task validation record from the prior stage.
    validation: TaskAssessment,
    /// Hard constraints for the endpoint prompt.
    response_guidelines: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnswerEndpointResponse {
    Structured {
        #[serde(default)]
        answer: Option<String>,
        #[serde(rename = "response")]
        #[serde(default)]
        response: Option<String>,
        #[serde(rename = "message")]
        #[serde(default)]
        message: Option<String>,
        #[serde(rename = "text")]
        #[serde(default)]
        text: Option<String>,
        #[serde(rename = "body")]
        #[serde(default)]
        body: Option<String>,
    },
    Plain(String),
}

fn build_answer_instruction(
    profile: &crate::types::ProjectProfile,
    message: &crate::types::GmailMessage,
    classification: &EmailClassification,
    intent: &EmailIntent,
    assessment: &TaskAssessment,
    subject: &str,
) -> AnswerInstruction {
    let reply_subject = subject.to_string();
    let body_excerpt = truncate_text(&message.prompt_body(), 6_000);
    let response_guidelines = vec![
        "Answer directly as MarianneAI support staff.".to_string(),
        "Keep tone concise and professional with a friendly touch.".to_string(),
        "If context is missing, request exactly what is needed next.".to_string(),
        "Preserve any project-relevant context and mention expected turnaround.".to_string(),
    ];

    AnswerInstruction {
        reply_subject,
        project_name: profile.name.clone(),
        project_about: profile.about.clone(),
        project_website: profile.website.clone(),
        classification: classification.clone(),
        sender: message
            .from
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string()),
        original_subject: message.subject.clone(),
        original_message_excerpt: body_excerpt,
        intent: intent.clone(),
        validation: assessment.clone(),
        response_guidelines,
    }
}

fn build_funny_fallback_message(reason: Option<&str>, source: &str) -> String {
    let reason = reason.unwrap_or(
        "Our automated answer pipeline is temporarily unavailable. Please retry in a few minutes.",
    );
    format!(
        "Hi there,\n\nThanks for reaching out about MarianneAI. Great timing — we are currently making {source} do a friendly obstacle course.\n\n{source} returned an unusual response, so the automatic pipeline is taking a short coffee break.\nCurrent status: {reason}\n\nPlease try again in a few minutes, and we will respond properly.\n\nIf this is urgent, resend with subject [urgent] and we will keep the ticket open.\n\nKind regards,\nMarianneAI Support"
    )
}

async fn send_with_fallback(
    gmail: &GmailClient,
    recipient: &str,
    subject: &str,
    message: &crate::types::GmailMessage,
    body: &str,
    endpoint: Option<&str>,
    fallback_reason: Option<&str>,
) -> Result<EmailResponseRecord, AgentError> {
    let busy_message = build_funny_fallback_message(None, "answering service");
    match gmail.send_reply(
        recipient,
        subject,
        body,
        Some(message.thread_id.as_str()),
        None,
    ) {
        Ok(sent_message) => {
            println!(
                "Sent reply to {} with subject {} (message {:?}, thread {:?})\n  preview: {}",
                recipient,
                subject,
                sent_message.message_id,
                sent_message.thread_id,
                preview_text(body, 220)
            );
            Ok(EmailResponseRecord {
                status: if endpoint.is_some() {
                    "sent".to_string()
                } else {
                    "sent_without_endpoint".to_string()
                },
                endpoint: endpoint.map(std::string::ToString::to_string),
                attempted_at: Utc::now(),
                sent_message_id: sent_message.message_id,
                sent_thread_id: sent_message.thread_id,
                recipient: recipient.to_string(),
                subject: subject.to_string(),
                response_preview: Some(preview_text(body, 400)),
                error: None,
            })
        }
        Err(primary_error) => {
            println!("Reply send failed for {}: {}", recipient, primary_error);
            if endpoint.is_some() && body != busy_message {
                let reason = fallback_reason.unwrap_or("generated content could not be delivered");
                let fallback_body =
                    build_funny_fallback_message(Some(reason), "email sending layer");
                match gmail.send_reply(
                    recipient,
                    subject,
                    &fallback_body,
                    Some(message.thread_id.as_str()),
                    None,
                ) {
                    Ok(sent_message) => Ok(EmailResponseRecord {
                        status: "sent_fallback".to_string(),
                        endpoint: endpoint.map(std::string::ToString::to_string),
                        attempted_at: Utc::now(),
                        sent_message_id: sent_message.message_id,
                        sent_thread_id: sent_message.thread_id,
                        recipient: recipient.to_string(),
                        subject: subject.to_string(),
                        response_preview: Some(preview_text(&fallback_body, 400)),
                        error: Some(format!(
                            "primary send failed ({primary_error}), fallback sent instead"
                        )),
                    })
                    .inspect(|record| {
                        println!(
                            "Sent fallback reply to {} with subject {} (message {:?}, thread {:?})",
                            recipient, subject, record.sent_message_id, record.sent_thread_id,
                        );
                        println!("  fallback preview: {}", preview_text(&fallback_body, 220));
                    }),
                    Err(fallback_error) => Err(AgentError::from(fallback_error)),
                }
            } else {
                Err(AgentError::from(primary_error))
            }
        }
    }
}

fn build_datagouv_query(
    profile: &crate::types::ProjectProfile,
    message: &crate::types::GmailMessage,
    classification: &EmailClassification,
    intent: &EmailIntent,
    assessment: &TaskAssessment,
) -> String {
    let subject = normalize_subject_query(
        message
            .subject
            .as_deref()
            .unwrap_or("General MarianneAI inquiry"),
    );
    let subject = normalize_search_text(&subject);
    let project_name = normalize_search_text(&profile.name);
    let _project_about = normalize_search_text(&profile.about);
    let category = normalize_search_text(&classification.category);
    let requested_outcome = normalize_search_text(&intent.requested_outcome);
    let intent_summary = normalize_search_text(&intent.intent_summary);
    let key_entities = intent
        .key_entities
        .iter()
        .map(|entity| normalize_search_text(entity))
        .filter(|entity| !entity.trim().is_empty())
        .collect::<Vec<_>>()
        .join(", ");

    let entities = if key_entities.is_empty() {
        "public data".to_string()
    } else {
        key_entities
    };
    let outcome = if requested_outcome.is_empty() {
        "search french public data by theme".to_string()
    } else {
        requested_outcome
    };
    let summary = if intent_summary.is_empty() {
        normalize_search_text(&intent.task_description)
    } else {
        intent_summary
    };
    let context = if summary.is_empty() {
        "civic data analysis".to_string()
    } else {
        summary
    };
    let source = normalize_search_text(&profile.website);
    let keyword_bundle = compose_query_keywords(&[
        subject.as_str(),
        category.as_str(),
        outcome.as_str(),
        context.as_str(),
        entities.as_str(),
    ]);

    format!(
        "Professional public-data search for MarianneAI | project: {project_name} | keywords: {keyword_bundle} | focus: {subject} | source: {source} | clarity={}",
        assessment.clarity_score
    )
}

fn build_datagouv_focus_query(
    profile: &crate::types::ProjectProfile,
    message: &crate::types::GmailMessage,
    classification: &EmailClassification,
    intent: &EmailIntent,
    assessment: &TaskAssessment,
) -> String {
    let subject = normalize_subject_query(
        message
            .subject
            .as_deref()
            .unwrap_or("General MarianneAI inquiry"),
    );
    let project_name = normalize_search_text(&profile.name);
    let category = normalize_search_text(&classification.category);
    let outcome = normalize_search_text(&intent.requested_outcome);
    let title = normalize_search_text(&intent.task_title);
    let context = normalize_search_text(&intent.task_description);
    let key_terms = compose_query_keywords(&[
        subject.as_str(),
        category.as_str(),
        outcome.as_str(),
        title.as_str(),
        context.as_str(),
    ]);
    let confidence = if assessment.clarity_score > 0 {
        format!("clarity={}", assessment.clarity_score)
    } else {
        "clarity=unknown".to_string()
    };

    if key_terms.is_empty() {
        format!("French public data | project: {project_name} | {confidence}")
    } else {
        format!("French public data | project: {project_name} | terms: {key_terms} | {confidence}")
    }
}

fn build_datagouv_keyword_only_query(
    _profile: &crate::types::ProjectProfile,
    message: &crate::types::GmailMessage,
    _classification: &EmailClassification,
    intent: &EmailIntent,
) -> String {
    let subject =
        normalize_subject_query(message.subject.as_deref().unwrap_or("French public data"));
    let subject = normalize_search_text(&subject);
    let entities = intent
        .key_entities
        .iter()
        .map(|entity| compose_query_keywords(&[normalize_search_text(entity).as_str()]))
        .collect::<Vec<_>>()
        .join(" ");
    let keywords = compose_query_keywords(&[subject.as_str(), entities.as_str()]);

    if keywords.is_empty() {
        "French public data".to_string()
    } else {
        format!("French public data | keywords: {keywords}")
    }
}

fn build_datagouv_memory_entry(
    message: &crate::types::GmailMessage,
    classification: &EmailClassification,
    intent: &EmailIntent,
    chosen: &DatagouvQueryCandidate,
    response: &str,
) -> String {
    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let subject = sanitize_for_memory(
        message
            .subject
            .as_deref()
            .unwrap_or("General MarianneAI inquiry"),
    );
    let category = sanitize_for_memory(&classification.category);
    let terms = sanitize_for_memory(&extract_memory_terms(&chosen.query));
    let query = sanitize_for_memory(&chosen.query);
    let response_excerpt = sanitize_for_memory(response);
    let task_hint = sanitize_for_memory(&intent.task_title);
    format!(
        "- [{timestamp}] source={source} | subject={subject} | category={category} | terms={terms} | query={query} | intent={task_hint} | response={response_excerpt}",
        source = chosen.source
    )
}

fn append_datagouv_memory(path: Option<&str>, entry: &str) -> Result<(), AgentError> {
    let Some(path) = path.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };

    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(AgentError::Io)?;

    if existing.trim().is_empty() {
        writeln!(file, "# Datagouv Query Memory")?;
        writeln!(file)?;
    }

    writeln!(file, "{}", entry)?;
    Ok(())
}

fn load_datagouv_memory_hints(path: Option<&str>) -> Vec<String> {
    let Some(path) = path.filter(|value| !value.trim().is_empty()) else {
        return Vec::new();
    };

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut hints = Vec::new();
    for line in content.lines() {
        if let Some(terms) = parse_memory_terms(line) {
            hints.push(terms);
        }
    }

    if hints.is_empty() {
        return Vec::new();
    }

    hints.truncate(6);
    hints
}

fn parse_memory_terms(line: &str) -> Option<String> {
    if !line.starts_with("- [") || !line.contains("terms=") {
        return None;
    }

    let marker = "terms=";
    let start = line.find(marker)? + marker.len();
    let terms = line[start..].split(" | ").next().unwrap_or_default().trim();
    if terms.is_empty() {
        None
    } else {
        Some(terms.to_string())
    }
}

fn sanitize_for_memory(input: &str) -> String {
    sanitize_query_text(input)
        .replace('|', " - ")
        .trim()
        .to_string()
}

fn extract_memory_terms(query: &str) -> String {
    let normalized = compose_query_keywords(&[query]);
    if normalized.is_empty() {
        "public data".to_string()
    } else {
        normalized
    }
}

fn build_datagouv_fallback_query(
    profile: &crate::types::ProjectProfile,
    message: &crate::types::GmailMessage,
    _classification: &EmailClassification,
    intent: &EmailIntent,
) -> String {
    let subject = normalize_subject_query(
        message
            .subject
            .as_deref()
            .unwrap_or("General MarianneAI inquiry"),
    );
    let subject = normalize_search_text(&subject);
    let entities = intent
        .key_entities
        .iter()
        .map(|entity| normalize_search_text(entity))
        .filter(|entity| !entity.trim().is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let about = normalize_search_text(&profile.about);
    let project = normalize_search_text(&profile.name);
    let task_focus = if !subject.is_empty() {
        subject
    } else {
        normalize_search_text(&intent.task_title)
    };

    let compact_keywords =
        compose_query_keywords(&[task_focus.as_str(), entities.as_str(), about.as_str()]);

    if compact_keywords.is_empty() {
        format!("French public data datasets | project: {project}")
    } else {
        format!("French public data datasets | {compact_keywords} | project: {project}")
    }
}

fn compose_query_keywords(parts: &[&str]) -> String {
    let stop_words = get_search_stop_words();
    let mut seen = HashSet::new();
    let mut keywords = Vec::new();

    for part in parts {
        for token in part
            .split_whitespace()
            .map(|token| token.trim())
            .filter(|token| !token.is_empty())
        {
            let token = token.to_lowercase();
            if stop_words.contains(token.as_str()) || token.len() <= 2 {
                continue;
            }
            if seen.insert(token.clone()) {
                keywords.push(token);
            }
            if keywords.len() >= 20 {
                break;
            }
        }
        if keywords.len() >= 20 {
            break;
        }
    }

    keywords.join(" ")
}

fn normalize_subject_query(subject: &str) -> String {
    let mut cleaned = subject.trim().to_string();
    let prefixes = [
        "re:",
        "fw:",
        "fwd:",
        "aw:",
        "tr:",
        "sv:",
        "ref:",
        "marianneai:",
    ];
    loop {
        let lowered = cleaned.to_ascii_lowercase();
        let Some(prefix) = prefixes.iter().find(|prefix| lowered.starts_with(*prefix)) else {
            break;
        };

        cleaned = cleaned[prefix.len()..].trim_start().to_string();
        cleaned = cleaned.trim_start().to_string();
    }

    cleaned
        .trim()
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_search_text(input: &str) -> String {
    sanitize_query_text(&transliterate_accents(input))
}

fn transliterate_accents(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'æ' | 'ā' => {
                output.push('a');
            }
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                output.push('A');
            }
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => output.push('e'),
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ė' | 'Ę' => output.push('E'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => output.push('i'),
            'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'Į' => output.push('I'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => output.push('o'),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => output.push('O'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => output.push('u'),
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' => output.push('U'),
            'ç' => output.push('c'),
            'Ç' => output.push('C'),
            'œ' => output.push_str("oe"),
            'Œ' => output.push_str("OE"),
            'ß' => output.push_str("ss"),
            'ÿ' => output.push('y'),
            'Ÿ' => output.push('Y'),
            _ => output.push(ch),
        }
    }

    sanitize_query_text(&output)
}

fn sanitize_query_text(input: &str) -> String {
    input
        .replace(['\n', '\r'], " ")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric()
                || c.is_whitespace()
                || matches!(c, '-' | '_' | '/' | '.' | ':' | '+' | '&')
            {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(std::string::ToString::to_string)
        .filter(|word: &String| {
            !matches!(
                word.as_str(),
                "i" | "me"
                    | "you"
                    | "the"
                    | "a"
                    | "an"
                    | "and"
                    | "or"
                    | "of"
                    | "to"
                    | "for"
                    | "with"
                    | "in"
                    | "on"
                    | "at"
                    | "that"
                    | "this"
                    | "from"
                    | "about"
                    | "please"
                    | "could"
                    | "would"
                    | "should"
                    | "thank"
                    | "thanks"
                    | "kindly"
                    | "cordially"
                    | "bonjour"
                    | "merci"
            )
        })
        .take(32)
        .collect::<Vec<_>>()
        .join(" ")
}

fn get_search_stop_words() -> &'static HashSet<&'static str> {
    const WORDS: [&str; 43] = [
        "and", "or", "the", "a", "an", "of", "to", "for", "with", "in", "on", "at", "that", "this",
        "from", "about", "it", "its", "i", "me", "we", "you", "they", "them", "his", "her", "hers",
        "is", "are", "was", "were", "be", "this", "not", "no", "as", "if", "how", "why", "when",
        "where", "who", "which",
    ];

    static WORD_SET: std::sync::LazyLock<HashSet<&'static str>> =
        std::sync::LazyLock::new(|| WORDS.iter().copied().collect());

    &WORD_SET
}

fn is_empty_datagouv_result(answer: &str) -> bool {
    let normalized = answer.trim().to_ascii_lowercase();
    normalized.contains("no datasets found for query:")
        || normalized.contains("no results for")
        || normalized.contains("no matching dataset")
        || normalized == "no output returned by datagouv mcp tool."
}

fn is_error_like_reply(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    normalized == "error"
        || normalized.contains("error:")
        || normalized.contains("exception")
        || normalized.contains("traceback")
        || normalized.contains("request failed")
        || normalized.contains("failed to")
        || normalized.contains("service unavailable")
        || normalized.contains("couldn't")
        || normalized.contains("could not")
        || normalized.contains("forbidden")
        || normalized.contains("not found")
        || normalized.contains("invalid")
        || normalized.contains("timeout")
        || normalized == "null"
        || normalized == "undefined"
        || normalized.contains("mcp returned an error")
}

async fn query_datagouv_mcp(
    client: &HttpClient,
    endpoint: &str,
    tool_name: &str,
    timeout_seconds: u64,
    query: &str,
) -> Result<String, AgentError> {
    let init_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "roots": {
                    "listChanged": false,
                },
                "sampling": {}
            },
            "clientInfo": {
                "name": "email-interface",
                "version": "0.1.0"
            }
        }
    });

    let init_response = client
        .post(endpoint)
        .json(&init_request)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .send()
        .await
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?
        .error_for_status()
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?;

    let session_id = init_response
        .headers()
        .get("mcp-session-id")
        .or_else(|| init_response.headers().get("Mcp-Session-Id"))
        .and_then(|value| value.to_str().ok().map(str::to_string));

    let init_body = init_response
        .text()
        .await
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let init_value =
        parse_mcp_response_payload(endpoint, "initialize", &init_body).map_err(|source| {
            AgentError::DatagouvToolError {
                tool: source.0,
                message: source.1,
            }
        })?;

    if let Some(error) = init_value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("initialize failed");
        return Err(AgentError::DatagouvToolError {
            tool: "initialize".to_string(),
            message: message.to_string(),
        });
    }

    let _ = client
        .post(endpoint)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .header(header::ACCEPT, "application/json, text/event-stream")
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .header("Mcp-Session-Id", session_id.as_deref().unwrap_or(""))
        .send()
        .await
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?;

    let call_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": {
                "query": query,
            }
        }
    });

    let call_response = client
        .post(endpoint)
        .json(&call_request)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .header("Mcp-Session-Id", session_id.as_deref().unwrap_or(""))
        .send()
        .await
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?
        .error_for_status()
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?;

    let call_body = call_response
        .text()
        .await
        .map_err(|source| AgentError::DatagouvMcp {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let payload = parse_mcp_response_payload(endpoint, tool_name, &call_body)
        .map_err(|(tool, message)| AgentError::DatagouvToolError { tool, message })?;

    if let Some(answer) = payload
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|first| first.get("text"))
        .and_then(Value::as_str)
    {
        return Ok(answer.to_string());
    }

    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("tool call failed");
        return Err(AgentError::DatagouvToolError {
            tool: tool_name.to_string(),
            message: message.to_string(),
        });
    }

    let raw = payload.to_string();
    if raw.trim().is_empty() {
        Ok("No output returned by Datagouv MCP tool.".to_string())
    } else {
        Ok(payload.to_string())
    }
}

fn parse_mcp_response_payload(
    endpoint: &str,
    tool_name: &str,
    body: &str,
) -> Result<Value, (String, String)> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err((
            tool_name.to_string(),
            format!("Datagouv MCP returned empty body from {endpoint}."),
        ));
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }

    let mut extracted_chunks = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.trim();
        if !line.starts_with("data:") {
            continue;
        }

        let data = line.trim_start_matches("data:").trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }

        match serde_json::from_str::<Value>(data) {
            Ok(value) => return Ok(value),
            Err(_) => extracted_chunks.push(data.to_string()),
        }
    }

    if extracted_chunks.is_empty() {
        return Err((
            tool_name.to_string(),
            format!(
                "Datagouv MCP returned non-JSON payload for {tool_name} at {endpoint}: {}",
                truncate_text(body, 240)
            ),
        ));
    }

    for chunk in extracted_chunks {
        if let Ok(value) = serde_json::from_str::<Value>(&chunk) {
            return Ok(value);
        }
    }

    Err((
        tool_name.to_string(),
        format!(
            "Datagouv MCP payload parse failure for {tool_name} at {endpoint}: {}",
            truncate_text(body, 240)
        ),
    ))
}

async fn generate_answer(
    client: &HttpClient,
    endpoint: &str,
    api_key: Option<&str>,
    instruction: &AnswerInstruction,
    timeout_seconds: u64,
) -> Result<String, AgentError> {
    let mut request = client
        .post(endpoint)
        .header(header::CONTENT_TYPE, "application/json")
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .json(instruction);

    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.header("x-api-key", api_key);
    }

    let response = request
        .send()
        .await
        .map_err(|source| AgentError::AnswerEndpoint {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let response = response
        .error_for_status()
        .map_err(|source| AgentError::AnswerEndpoint {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let payload_text = response
        .text()
        .await
        .map_err(|source| AgentError::AnswerEndpoint {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let candidate = extract_json_object(payload_text.trim())
        .unwrap_or(payload_text.trim())
        .to_string();

    if let Ok(structured) = serde_json::from_str::<AnswerEndpointResponse>(&candidate) {
        if let AnswerEndpointResponse::Plain(value) = structured {
            return if value.trim().is_empty() {
                Err(AgentError::AnswerEndpointEmpty)
            } else {
                Ok(value)
            };
        }

        if let AnswerEndpointResponse::Structured {
            answer,
            response,
            message,
            text,
            body,
        } = structured
        {
            let answer = answer
                .or(response)
                .or(message)
                .or(text)
                .or(body)
                .unwrap_or_default();
            if answer.trim().is_empty() {
                return Err(AgentError::AnswerEndpointEmpty);
            }
            return Ok(answer);
        }
    }

    if !candidate.trim().is_empty() {
        return Ok(candidate);
    }

    Err(AgentError::AnswerEndpointEmpty)
}

fn preview_text(input: &str, limit: usize) -> String {
    truncate_text(input, limit)
}

fn build_reply_subject(subject: Option<&str>) -> String {
    let base = subject.unwrap_or("(no subject)");
    if base.to_ascii_lowercase().starts_with("re:") {
        base.to_string()
    } else {
        format!("Re: {base}")
    }
}

/// Final report generator and mission terminator.
pub struct Auditor;

#[async_trait]
impl Specialist for Auditor {
    fn name(&self) -> &str {
        "auditor"
    }

    async fn run_turn(&self, ctx: GlobalContext) -> Result<TurnResult, AgentError> {
        let related = ctx
            .reviewed_emails
            .iter()
            .filter(|email| email.classification.related_to_project)
            .count();
        let queued = ctx
            .reviewed_emails
            .iter()
            .filter(|email| {
                email
                    .queued_job
                    .as_ref()
                    .is_some_and(|record| record.status == "queued")
            })
            .count();
        let review_needed = ctx
            .reviewed_emails
            .iter()
            .filter(|email| {
                email
                    .assessment
                    .as_ref()
                    .is_some_and(|assessment| assessment.needs_human_review)
            })
            .count();

        let summary = format!(
            "Processed {} email(s): {} related to MarianneAI, {} persisted to the {:?} queue backend, {} still need human review.",
            ctx.reviewed_emails.len(),
            related,
            queued,
            ctx.queue_backend,
            review_needed
        );

        Ok(TurnResult::FinalResult(summary))
    }
}

fn build_report(context: &GlobalContext) -> MissionReport {
    let related_count = context
        .reviewed_emails
        .iter()
        .filter(|email| email.classification.related_to_project)
        .count();
    let queueable_count = context
        .reviewed_emails
        .iter()
        .filter(|email| {
            email
                .assessment
                .as_ref()
                .is_some_and(|assessment| assessment.should_queue)
        })
        .count();
    let persisted_count = context
        .reviewed_emails
        .iter()
        .filter(|email| {
            email
                .queued_job
                .as_ref()
                .is_some_and(|record| record.status == "queued")
        })
        .count();
    let response_attempt_count = context
        .reviewed_emails
        .iter()
        .filter(|email| email.response.is_some())
        .count();
    let responded_count = context
        .reviewed_emails
        .iter()
        .filter(|email| {
            email.response.as_ref().is_some_and(|response| {
                matches!(
                    response.status.as_str(),
                    "sent" | "sent_fallback" | "sent_without_endpoint"
                )
            })
        })
        .count();

    MissionReport {
        objective: context.objective.clone(),
        query: context.query.clone(),
        queue_backend: context.queue_backend,
        total_loaded: context.emails.len(),
        related_count,
        queueable_count,
        persisted_count,
        responded_count,
        response_attempt_count,
        reviewed_emails: context.reviewed_emails.clone(),
        conversation_history: context.conversation_history.clone(),
    }
}

fn save_final_report(
    report: &MissionReport,
    summary: &str,
    output_dir: &Path,
) -> Result<(), AgentError> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let report_path = output_dir.join(format!("final_report_{}.json", timestamp));
    let summary_path = output_dir.join("triage_summary.txt");

    fs::write(report_path, serde_json::to_string_pretty(report)?)?;
    fs::write(summary_path, summary)?;

    Ok(())
}

fn finalize_current_email(ctx: &mut GlobalContext, disposition: &str) {
    if let Some(message) = ctx.current_email().cloned() {
        let classification = ctx
            .current_classification
            .clone()
            .unwrap_or(EmailClassification {
                related_to_project: false,
                category: "unclassified".to_string(),
                confidence: 0.0,
                public_data_relevance: false,
                reasoning: "No classification was produced.".to_string(),
            });

        ctx.reviewed_emails.push(ProcessedEmail {
            message,
            classification,
            intent: ctx.current_intent.clone(),
            assessment: ctx.current_validation.clone(),
            queued_job: ctx.current_queue_record.clone(),
            response: ctx.current_response.clone(),
            disposition: disposition.to_string(),
        });
    }

    ctx.advance_email();
}

fn parse_structured_json<T>(agent_name: &'static str, response: &str) -> Result<T, AgentError>
where
    T: DeserializeOwned,
{
    let candidate = extract_json_object(response).unwrap_or(response).trim();
    serde_json::from_str(candidate).map_err(|source| AgentError::JsonResponse {
        agent: agent_name,
        source,
        response: response.to_string(),
    })
}

fn extract_json_object(input: &str) -> Option<&str> {
    let start = input.find('{')?;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in input[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match character {
            '\\' if in_string => {
                escaped = true;
            }
            '"' => {
                in_string = !in_string;
            }
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let end = start + index + character.len_utf8();
                    return Some(&input[start..end]);
                }
            }
            _ => {}
        }
    }

    None
}

fn truncate_text(input: &str, limit: usize) -> String {
    let mut output = input.chars().take(limit).collect::<String>();
    if input.chars().count() > limit {
        output.push_str("\n[truncated]");
    }
    output
}

/// Build all specialists once and return a map keyed by routing name.
pub fn build_agents(
    model: gemini::CompletionModel,
    queue_backend: QueueBackendKind,
) -> Result<HashMap<String, Box<dyn Specialist>>, AgentError> {
    let writer = TaskQueueWriter::from_backend(queue_backend)?;
    let mut agents: HashMap<String, Box<dyn Specialist>> = HashMap::new();
    agents.insert("supervisor".to_string(), Box::new(Supervisor));
    agents.insert(
        "mailbox_reader".to_string(),
        Box::new(MailboxReader::new(GmailClient)),
    );
    agents.insert(
        "relevance_classifier".to_string(),
        Box::new(RelevanceClassifier::new(model.clone())),
    );
    agents.insert(
        "intent_extractor".to_string(),
        Box::new(IntentExtractor::new(model.clone())),
    );
    agents.insert(
        "task_validator".to_string(),
        Box::new(TaskValidator::new(model.clone())),
    );
    agents.insert(
        "queue_writer".to_string(),
        Box::new(QueueWriterAgent::new(writer)),
    );
    agents.insert(
        "answer_responder".to_string(),
        Box::new(AnswerResponder::new()),
    );
    agents.insert(
        "datagouv_responder".to_string(),
        Box::new(DatagouvResponder::new()),
    );
    agents.insert("auditor".to_string(), Box::new(Auditor));
    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::{
        DatagouvQueryCandidate, DatagouvResponder, append_datagouv_memory,
        build_datagouv_memory_entry, build_datagouv_query, extract_json_object,
        load_datagouv_memory_hints, normalize_subject_query, parse_mcp_response_payload,
        sanitize_query_text, truncate_text,
    };
    use crate::types::{
        EmailClassification, EmailIntent, GmailMessage, ProjectProfile, TaskAssessment,
    };
    use std::env::temp_dir;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn extracts_first_json_object() {
        let response = "Here is the result:\n{\"ok\":true}\nThanks";
        assert_eq!(extract_json_object(response), Some("{\"ok\":true}"));
    }

    #[test]
    fn parses_mcp_json_response_body() {
        let payload = r#"{"jsonrpc":"2.0","result":{"content":[{"text":"ok"}]}}"#;
        let value =
            parse_mcp_response_payload("https://mcp.example.com/mcp", "search_datasets", payload)
                .expect("expected valid json payload");
        assert_eq!(
            value.get("result"),
            Some(&serde_json::json!({"content":[{"text":"ok"}]}))
        );
    }

    #[test]
    fn parses_mcp_sse_response_body() {
        let payload = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"ok\"}]}}\n";
        let value =
            parse_mcp_response_payload("https://mcp.example.com/mcp", "search_datasets", payload)
                .expect("expected valid sse payload");
        assert_eq!(
            value.get("result"),
            Some(&serde_json::json!({"content":[{"text":"ok"}]}))
        );
    }

    #[test]
    fn truncates_long_bodies() {
        let text = "abcdef";
        assert_eq!(truncate_text(text, 3), "abc\n[truncated]");
    }

    #[test]
    fn normalizes_reply_subject_prefixes() {
        let normalized =
            normalize_subject_query("Re: FW: marianneai: Budget public dataset review");
        assert_eq!(normalized, "Budget public dataset review");
        assert!(!normalized.to_lowercase().starts_with("re:"));
        assert!(!normalized.to_lowercase().contains("fw:"));
    }

    #[test]
    fn sanitizes_subject_like_text_for_query() {
        let sanitized = sanitize_query_text(
            "Could you please provide, with data, the 2024 transport subsidies?? Merci, user.",
        );
        assert!(sanitized.contains("provide"));
        assert!(sanitized.contains("transport"));
        assert!(sanitized.contains("subsidies"));
        assert!(!sanitized.contains("please"));
        assert!(!sanitized.contains("merci"));
    }

    #[test]
    fn datagouv_query_is_structured_and_professional() {
        let profile = ProjectProfile::default();
        let message = GmailMessage {
            message_id: "mid-1".to_string(),
            thread_id: "tid-1".to_string(),
            from: Some("user@example.com".to_string()),
            to: Some("agent@example.com".to_string()),
            subject: Some("Re: FW: marianneai: Need public housing statistics by city".to_string()),
            date: None,
            snippet: Some("Please share".to_string()),
            plain_text_body: Some(
                "Could you please provide exact numbers for each city from your dataset?"
                    .to_string(),
            ),
            html_body: None,
            label_ids: vec![],
        };
        let classification = EmailClassification {
            related_to_project: true,
            category: "dataset request".to_string(),
            confidence: 0.98,
            public_data_relevance: true,
            reasoning: "clear".to_string(),
        };
        let intent = EmailIntent {
            task_title: "Public housing trend".to_string(),
            intent_summary: "find latest housing dataset for social aid".to_string(),
            requested_outcome: "Retrieve municipal housing allocation datasets".to_string(),
            task_description: "analyse municipal housing".to_string(),
            requester_identity: Some("user@example.com".to_string()),
            key_entities: vec!["housing".to_string(), "social aid".to_string()],
            actionable: true,
            missing_information: vec![],
        };
        let assessment = TaskAssessment {
            should_queue: true,
            needs_human_review: false,
            clarity_score: 89,
            priority: "high".to_string(),
            rationale: "Clear intent".to_string(),
            missing_information: vec![],
        };

        let query = build_datagouv_query(&profile, &message, &classification, &intent, &assessment);

        assert!(query.contains("Professional public-data search for MarianneAI"));
        assert!(query.contains("focus: Need public housing statistics by city"));
        assert!(query.contains("keywords:"));
        assert!(query.contains("housing"));
        assert!(query.contains("social"));
        assert!(query.contains("clarity=89"));
        assert!(!query.contains("please"));
        assert!(!query.contains("exact numbers"));
    }

    #[test]
    fn datagouv_query_variants_include_primary_and_keyword_fallbacks() {
        let responder = DatagouvResponder::new();
        let profile = ProjectProfile::default();
        let message = GmailMessage {
            message_id: "mid-2".to_string(),
            thread_id: "tid-2".to_string(),
            from: Some("user@example.com".to_string()),
            to: Some("agent@example.com".to_string()),
            subject: Some(
                "Re: marianneai: Need a stable list of companies in France by city".to_string(),
            ),
            date: None,
            snippet: Some("Need french company dataset".to_string()),
            plain_text_body: None,
            html_body: None,
            label_ids: vec![],
        };
        let classification = EmailClassification {
            related_to_project: true,
            category: "dataset request".to_string(),
            confidence: 0.92,
            public_data_relevance: true,
            reasoning: "clear request".to_string(),
        };
        let intent = EmailIntent {
            task_title: "French company list".to_string(),
            intent_summary: "Collect company data by french location".to_string(),
            requested_outcome: "retrieve a clean dataset for French companies".to_string(),
            task_description: "find french company data".to_string(),
            requester_identity: Some("user@example.com".to_string()),
            key_entities: vec!["company".to_string(), "France".to_string()],
            actionable: true,
            missing_information: vec![],
        };
        let assessment = TaskAssessment {
            should_queue: true,
            needs_human_review: false,
            clarity_score: 91,
            priority: "high".to_string(),
            rationale: "Actionable".to_string(),
            missing_information: vec![],
        };
        let memory = vec!["companys list".to_string(), "siret".to_string()];

        let variants = responder.query_variants(
            &profile,
            &message,
            &classification,
            &intent,
            &assessment,
            &memory,
        );

        let has_primary = variants
            .iter()
            .any(|variant| variant.source == "primary_structured_query");
        let has_focus = variants
            .iter()
            .any(|variant| variant.source == "focus_query");
        let has_short = variants
            .iter()
            .any(|variant| variant.source == "shorter_keyword_query");
        let has_minimal = variants
            .iter()
            .any(|variant| variant.source == "minimal_keywords_only");
        let has_memory = variants
            .iter()
            .any(|variant| variant.source == "learned_memory");

        assert!(has_primary);
        assert!(has_focus);
        assert!(has_short);
        assert!(has_minimal);
        assert!(has_memory);
    }

    #[test]
    fn datagouv_memory_parse_round_trip_works() {
        let _profile = ProjectProfile::default();
        let message = GmailMessage {
            message_id: "mid-3".to_string(),
            thread_id: "tid-3".to_string(),
            from: Some("user@example.com".to_string()),
            to: Some("agent@example.com".to_string()),
            subject: Some("marianneai: company data".to_string()),
            date: None,
            snippet: Some("Need company stats".to_string()),
            plain_text_body: None,
            html_body: None,
            label_ids: vec![],
        };
        let classification = EmailClassification {
            related_to_project: true,
            category: "dataset request".to_string(),
            confidence: 0.9,
            public_data_relevance: true,
            reasoning: "clear request".to_string(),
        };
        let intent = EmailIntent {
            task_title: "Company data".to_string(),
            intent_summary: "Get public company reference data".to_string(),
            requested_outcome: "Find company information".to_string(),
            task_description: "Get company list".to_string(),
            requester_identity: Some("user@example.com".to_string()),
            key_entities: vec!["company".to_string(), "SIREN".to_string()],
            actionable: true,
            missing_information: vec![],
        };
        let response = "Useful output from datagouv";
        let chosen_variant = DatagouvQueryCandidate {
            source: "minimal_keywords_only",
            query: "French public data".to_string(),
        };
        let entry = build_datagouv_memory_entry(
            &message,
            &classification,
            &intent,
            &chosen_variant,
            response,
        );
        assert!(entry.contains("source=minimal_keywords_only"));
        assert!(entry.contains("company"));

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before epoch")
            .as_nanos();
        let path = temp_dir().join(format!("datagouv-query-memory-test-{nanos}.md"));
        let path = path.to_str().expect("utf8 path");
        append_datagouv_memory(Some(path), &entry).expect("writing memory entry");
        let hints = load_datagouv_memory_hints(Some(path));
        assert!(!hints.is_empty());
        assert!(hints[0].contains("french public data"));
        fs::remove_file(path).expect("remove test memory file");
    }
}
