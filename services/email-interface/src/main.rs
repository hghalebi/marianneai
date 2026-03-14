//! CLI entrypoint for the MarianneAI email interface.
//! It wires runtime arguments into the shared multi-agent pipeline and writes
//! progress snapshots plus the final report under the configured output directory.

use clap::Parser;
use email_interface::agentic::{AgentError, Orchestrator, build_agents};
use email_interface::gmail;
use email_interface::types::{GlobalContext, QueueBackendKind};
use reqwest::Url;
use rig::client::{CompletionClient, ProviderClient};
use rig::providers::gemini;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";
const DEFAULT_GEMINI_FALLBACK_MODEL: &str = "gemini-1.5-flash";

/// Multi-agent Gmail triage for MarianneAI.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "Read Gmail, classify MarianneAI-related emails, extract intent, and queue valid tasks."
)]
struct Args {
    /// Gmail search query passed directly to `gws gmail users messages list`.
    #[arg(long, default_value = "newer_than:14d")]
    query: String,

    /// Maximum number of emails to read during this run. The Gmail stage also
    /// clamps values to a safe API window.
    #[arg(long, default_value_t = 10)]
    max_emails: u32,

    /// Directory where snapshots and the final report will be written.
    #[arg(long, default_value = "./output/email-triage")]
    output_dir: PathBuf,

    /// Queue backend to use for valid tasks. If omitted, `QUEUE_BACKEND` is used.
    #[arg(long, value_enum)]
    queue_backend: Option<QueueBackendKind>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let queue_backend = args
        .queue_backend
        .unwrap_or_else(QueueBackendKind::from_env);

    ensure_runtime_access()?;
    ensure_gemini_key()?;

    let model_name = std::env::var("GEMINI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GEMINI_MODEL.to_string());
    let fallback_model = std::env::var("GEMINI_FALLBACK_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GEMINI_FALLBACK_MODEL.to_string());
    let client = gemini::Client::from_env();
    let completion_model = pick_gemini_model(
        &client,
        &model_name,
        &fallback_model,
        std::env::var("USE_GEMINI_FALLBACK").ok(),
    );
    let agents = build_agents(completion_model, queue_backend)?;

    let objective = format!(
        "Read recent Gmail messages, find the ones related to MarianneAI, extract sender intent, and queue valid tasks to {:?}.",
        queue_backend
    );

    let mut context = GlobalContext::new(objective, args.query, args.max_emails, queue_backend);
    context.answer_endpoint = std::env::var("ANSWER_ENDPOINT_URL").ok();
    context.answer_endpoint_api_key = std::env::var("ANSWER_ENDPOINT_API_KEY").ok();
    context.answer_endpoint_timeout_seconds = std::env::var("ANSWER_ENDPOINT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    let force_remote_mcp = env_value("DATAGOUV_MCP_FORCE_REMOTE")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"));
    let configured_mcp_endpoint = env_value("DATAGOUV_MCP_ENDPOINT");
    let local_mcp_endpoint = resolve_datagouv_mcp_endpoint();
    context.datagouv_mcp_endpoint = select_datagouv_mcp_endpoint(
        configured_mcp_endpoint,
        local_mcp_endpoint,
        force_remote_mcp,
    );
    context.datagouv_mcp_tool = std::env::var("DATAGOUV_MCP_TOOL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "search_datasets".to_string());
    context.datagouv_mcp_timeout_seconds = std::env::var("DATAGOUV_MCP_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    context.datagouv_query_memory_path = env_value("DATAGOUV_QUERY_MEMORY_PATH").or_else(|| {
        Some(
            args.output_dir
                .join("datagouv-query-memory.md")
                .to_string_lossy()
                .to_string(),
        )
    });
    log_access_state(&context);

    let orchestrator = Orchestrator::new(agents);

    let summary = orchestrator.run_mission(context, args.output_dir).await?;
    println!("{}", summary);
    Ok(())
}

fn log_access_state(ctx: &GlobalContext) {
    if ctx.answer_endpoint.is_none() {
        println!(
            "Hint: ANSWER_ENDPOINT_URL is unset. Related emails will use Datagouv MCP responder if DATAGOUV_MCP_ENDPOINT is set; otherwise a temporary fallback message is sent."
        );
    } else if let Some(endpoint) = &ctx.answer_endpoint {
        if Url::parse(endpoint).is_err() {
            println!(
                "Invalid ANSWER_ENDPOINT_URL value '{endpoint}'. Verify this is a valid HTTP(S) URL."
            );
        } else {
            println!(
                "Answer endpoint configured at {endpoint} with timeout {}s.",
                ctx.answer_endpoint_timeout_seconds
            );
        }
    }

    if let Some(endpoint) = &ctx.datagouv_mcp_endpoint {
        if Url::parse(endpoint).is_err() {
            println!(
                "Invalid DATAGOUV_MCP_ENDPOINT value '{endpoint}'. Verify this is a valid MCP URL."
            );
        } else {
            println!(
                "Datagouv MCP endpoint configured: {endpoint} (tool: {}, timeout: {}s).",
                ctx.datagouv_mcp_tool, ctx.datagouv_mcp_timeout_seconds
            );
        }
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn select_datagouv_mcp_endpoint(
    configured_endpoint: Option<String>,
    local_endpoint: Option<String>,
    force_remote: bool,
) -> Option<String> {
    if force_remote {
        return configured_endpoint;
    }

    if let Some(local_endpoint) = local_endpoint {
        if let Some(configured_endpoint) = configured_endpoint {
            if is_local_datagouv_endpoint(&configured_endpoint) {
                println!(
                    "Datagouv MCP endpoint points to local instance; using {configured_endpoint}."
                );
                return Some(configured_endpoint);
            }

            println!(
                "Datagouv MCP local instance is available at {local_endpoint}; overriding configured endpoint ({configured_endpoint}) to use local tools."
            );
            return Some(local_endpoint);
        }

        println!(
            "Datagouv MCP local instance detected. Using local tools endpoint: {local_endpoint}."
        );
        return Some(local_endpoint);
    }

    configured_endpoint
}

fn is_local_datagouv_endpoint(endpoint: &str) -> bool {
    endpoint.contains("://127.0.0.1")
        || endpoint.contains("://localhost")
        || endpoint.contains("://[::1]")
}

fn resolve_datagouv_mcp_endpoint() -> Option<String> {
    let host = std::env::var("DATAGOUV_MCP_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = std::env::var("DATAGOUV_MCP_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(8000);

    if is_local_datagouv_mcp_available(&host, port) {
        let endpoint = format!("http://{host}:{port}/mcp");
        println!("Datagouv MCP auto-detected locally at {endpoint} (host={host}, port={port}).");
        Some(endpoint)
    } else {
        println!(
            "Hint: DATAGOUV_MCP_ENDPOINT is unset and local MCP at {host}:{port} is not reachable. Start it with scripts/run_datagouv_mcp_local.sh or set DATAGOUV_MCP_ENDPOINT for hosted mode."
        );
        None
    }
}

fn is_local_datagouv_mcp_available(host: &str, port: u16) -> bool {
    let endpoint = format!("{host}:{port}");
    let socket_addrs = match endpoint.to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(_) => return false,
    };

    socket_addrs.into_iter().any(|socket_addr: SocketAddr| {
        TcpStream::connect_timeout(&socket_addr, Duration::from_millis(400)).is_ok()
    })
}

fn ensure_runtime_access() -> Result<(), AgentError> {
    let cli_command = gmail::gmail_cli_command();
    if !command_exists(&cli_command) {
        return Err(AgentError::Context(format!(
            "Missing Google Workspace CLI command `{cli_command}`. Install `gws` or set GWS_BIN or GMAIL_CLI_COMMAND to a valid command."
        )));
    }

    let has_google_client_id = std::env::var("GOOGLE_WORKSPACE_CLI_CLIENT_ID").is_ok();
    let has_google_client_secret = std::env::var("GOOGLE_WORKSPACE_CLI_CLIENT_SECRET").is_ok();
    let gws_config_dir = gmail::gws_config_dir();
    let client_secret_file = gmail::gws_config_dir()
        .map(|dir| dir.join("client_secret.json"))
        .filter(|path| path.exists());
    let config_dir_exists = gws_config_dir.as_ref().is_some_and(|dir| dir.exists());

    if (!has_google_client_id || !has_google_client_secret) && client_secret_file.is_none() {
        return Err(AgentError::Context(
            "OAuth client config not found. Set GOOGLE_WORKSPACE_CLI_CLIENT_ID and GOOGLE_WORKSPACE_CLI_CLIENT_SECRET, or write them in ~/.config/gws/client_secret.json."
                .to_string(),
        ));
    }

    if !config_dir_exists {
        let fallback_dir = gws_config_dir.unwrap_or_else(|| PathBuf::from("~/.config/gws"));
        println!(
            "WARNING: {} has no `~/.config/gws` directory. Re-run: gws auth login --scopes https://www.googleapis.com/auth/gmail.modify,https://www.googleapis.com/auth/gmail.send",
            fallback_dir.display(),
        );
        println!(
            "If you already configured credentials, set GWS_BIN/GMAIL_CLI_COMMAND to that binary."
        );
    }

    if !has_google_client_id {
        println!(
            "Hint: set GOOGLE_WORKSPACE_CLI_CLIENT_ID before running automated Gmail operations."
        );
    }

    if !has_google_client_secret {
        println!(
            "Hint: set GOOGLE_WORKSPACE_CLI_CLIENT_SECRET before running automated Gmail operations."
        );
    }

    if has_google_client_id && has_google_client_secret {
        println!("OAuth credentials are provided via environment variables.");
    } else if config_dir_exists {
        println!(
            "OAuth credentials file exists at ~/.config/gws; using cached profile if present."
        );
    }

    println!("Verified Gmail CLI command: {cli_command}");
    Ok(())
}

fn pick_gemini_model(
    client: &gemini::Client,
    primary: &str,
    fallback: &str,
    forced_fallback: Option<String>,
) -> gemini::CompletionModel {
    let primary_model = primary.trim();
    let fallback_model = fallback.trim();
    if primary_model.is_empty() {
        return client.completion_model(DEFAULT_GEMINI_MODEL);
    }

    if let Some(force_fallback) = forced_fallback
        .as_deref()
        .map(str::trim)
        .filter(|flag| !flag.is_empty())
    {
        let force =
            force_fallback.eq_ignore_ascii_case("true") || force_fallback.eq_ignore_ascii_case("1");
        if force {
            println!("USE_GEMINI_FALLBACK enabled; using fallback model {fallback_model}.");
            if !fallback_model.is_empty() {
                return client.completion_model(fallback_model);
            }
        }
    }

    if !fallback_model.is_empty() && fallback_model != primary_model {
        println!(
            "Primary Gemini model configured as '{primary_model}'. If quota errors happen, set USE_GEMINI_FALLBACK=true to force '{fallback_model}'."
        );
    }

    client.completion_model(primary_model)
}

fn command_exists(program: &str) -> bool {
    if program.contains('/') && std::path::Path::new(program).is_file() {
        return true;
    }

    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|entry| {
                    let candidate = entry.join(program);
                    if cfg!(windows) {
                        candidate.with_extension("exe").exists()
                    } else {
                        candidate.exists()
                    }
                })
                .find(|exists| *exists)
        })
        .unwrap_or(false)
}

/// Fail fast before creating model clients so missing secrets are reported
/// before pipeline startup begins.
fn ensure_gemini_key() -> Result<(), AgentError> {
    if std::env::var("GEMINI_API_KEY").is_ok() {
        Ok(())
    } else {
        Err(AgentError::Context(
            "GEMINI_API_KEY is required to run the Rig specialists".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::select_datagouv_mcp_endpoint;

    #[test]
    fn keeps_configured_local_when_set_explicitly() {
        let configured = Some("http://127.0.0.1:8001/mcp".to_string());
        let local = Some("http://127.0.0.1:8000/mcp".to_string());
        let actual = select_datagouv_mcp_endpoint(configured.clone(), local, false);
        assert_eq!(actual, configured);
    }

    #[test]
    fn prefers_local_over_remote_by_default() {
        let configured = Some("https://mcp.data.gouv.fr/mcp".to_string());
        let local = Some("http://127.0.0.1:8000/mcp".to_string());
        let actual = select_datagouv_mcp_endpoint(configured, local.clone(), false);
        assert_eq!(actual, local);
    }

    #[test]
    fn force_remote_keeps_configured_value() {
        let configured = Some("https://mcp.data.gouv.fr/mcp".to_string());
        let local = Some("http://127.0.0.1:8000/mcp".to_string());
        let actual = select_datagouv_mcp_endpoint(configured.clone(), local, true);
        assert_eq!(actual, configured);
    }
}
