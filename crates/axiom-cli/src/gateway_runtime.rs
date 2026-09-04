use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Result};
use axiom_core::{atomic_write, AxiomConfig};
use axiom_engine::{ApprovalRequest, SkillApproval};
use serde::Deserialize;

use crate::chat::ChatSession;

const TELEGRAM_CHUNK_LIMIT: usize = 4000;
const MAX_SESSION_MAP_BYTES: u64 = 1024 * 1024;

pub(crate) async fn run_telegram_gateway(config_path: PathBuf) -> Result<()> {
    let config = AxiomConfig::load_from_path(&config_path)?;
    let token_env = config
        .gateway
        .telegram_bot_token_env
        .clone()
        .ok_or_else(|| {
            anyhow!("telegram is not configured yet. Run `axiom gateway setup` first.")
        })?;
    let token = crate::credentials::resolve_credential(&token_env)?.ok_or_else(|| {
        anyhow!("{token_env} is not set. Save it with `axiom gateway setup` and retry.")
    })?;
    let allowlist = config.gateway.telegram_allowed_chat_ids.clone();

    let client = TelegramClient::new(&token)?;
    let bot_name = client.get_me().await?;
    println!("Telegram gateway connected as @{bot_name}. Press Ctrl-C to stop.");
    if allowlist.is_empty() {
        println!("Warning: no allowed chat IDs set. Anyone who finds the bot can talk to it.");
        println!("Restrict it with `axiom gateway setup`.");
    }

    let mut offset: i64 = 0;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Telegram gateway stopped.");
                return Ok(());
            }
            updates = client.get_updates(offset) => {
                match updates {
                    Ok(batch) => {
                        for update in batch {
                            offset = offset.max(update.update_id.saturating_add(1));
                            if let Some((chat_id, text)) = update_text(&update)
                                .filter(|(chat_id, _)| chat_allowed(&allowlist, *chat_id))
                            {
                                if let Err(error) = answer_telegram_message(
                                    &client,
                                    &config_path,
                                    chat_id,
                                    &text,
                                )
                                .await
                                {
                                    eprintln!("telegram turn failed for chat {chat_id}: {error:#}");
                                }
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("telegram poll failed (retrying): {error:#}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn answer_telegram_message(
    client: &TelegramClient,
    config_path: &Path,
    chat_id: i64,
    text: &str,
) -> Result<()> {
    let mut session = load_gateway_session(config_path, &format!("telegram:{chat_id}"))?;
    let reply = respond_with_session(&mut session, text).await;
    for chunk in split_telegram_text(&reply) {
        client.send_message(chat_id, &chunk).await?;
    }
    Ok(())
}

pub(crate) async fn respond_with_session(session: &mut ChatSession, text: &str) -> String {
    match parse_bot_command(text) {
        BotCommand::Start | BotCommand::Help => HELP_TEXT.to_string(),
        BotCommand::Status => format!(
            "provider: {}\nmodel: {}",
            session.active_provider().unwrap_or("not configured"),
            session.active_model().unwrap_or("not configured"),
        ),
        BotCommand::Models { filter } => list_models_reply(session, filter.as_deref()).await,
        BotCommand::Model { id } => switch_model_reply(session, &id).await,
        BotCommand::Provider { name } => match session.set_provider(name.clone()) {
            Ok(active) => format!(
                "Provider switched to {active} with model {}.",
                session.active_model().unwrap_or("not configured")
            ),
            Err(error) => format!("Provider switch failed: {error:#}"),
        },
        BotCommand::Chat { text } => {
            let cards = match session.select_skill_cards(&text, 5) {
                Ok(cards) => cards,
                Err(error) => return format!("Skill lookup failed: {error:#}"),
            };
            let mut approval = BotApprover;
            match session
                .send_user_message_with_options(text, &cards, &mut approval, true)
                .await
            {
                Ok(turn) => turn.content,
                Err(error) => {
                    if let Some(hint) =
                        crate::credentials::credential_hint_for_error(&error.to_string())
                    {
                        format!("I hit an error: {error:#}\n{hint}")
                    } else {
                        format!("I hit an error: {error:#}\nTry /status or rephrase and retry.")
                    }
                }
            }
        }
    }
}

async fn list_models_reply(session: &ChatSession, filter: Option<&str>) -> String {
    let provider = match session.active_provider() {
        Some(provider) => provider.to_string(),
        None => return "No active provider. Use /provider <name> first.".to_string(),
    };
    match session.available_models(&provider).await {
        Ok(models) => {
            let query = filter.unwrap_or("").trim().to_ascii_lowercase();
            let mut visible: Vec<&str> = models
                .iter()
                .map(|model| model.id.as_str())
                .filter(|id| query.is_empty() || id.to_ascii_lowercase().contains(&query))
                .take(25)
                .collect();
            if visible.is_empty() {
                return format!("No models match '{query}'. Try /models without a filter.");
            }
            visible.sort_unstable();
            let mut reply = format!("Models on {provider} (showing {}):\n", visible.len());
            for id in visible {
                reply.push_str("- ");
                reply.push_str(id);
                reply.push('\n');
            }
            reply.push_str("Switch with: /model <id>");
            reply
        }
        Err(error) => format!("Could not fetch the live catalog: {error:#}"),
    }
}

async fn switch_model_reply(session: &mut ChatSession, id: &str) -> String {
    let provider = match session.active_provider() {
        Some(provider) => provider.to_string(),
        None => return "No active provider. Use /provider <name> first.".to_string(),
    };
    match session.available_models(&provider).await {
        Ok(models) if models.iter().any(|model| model.id == id) => match session.set_model(id) {
            Ok(active) => format!("Model switched to {active}."),
            Err(error) => format!("Model switch failed: {error:#}"),
        },
        Ok(models) => {
            let mut close: Vec<&str> = models
                .iter()
                .map(|model| model.id.as_str())
                .filter(|candidate| {
                    candidate
                        .to_ascii_lowercase()
                        .contains(&id.to_ascii_lowercase())
                })
                .take(8)
                .collect();
            close.sort_unstable();
            if close.is_empty() {
                format!("'{id}' is not in the {provider} catalog. See /models for exact IDs.")
            } else {
                format!(
                    "'{id}' is not an exact catalog ID. Did you mean one of these?\n- {}\nUse the full ID with /model.",
                    close.join("\n- ")
                )
            }
        }
        Err(_) => match session.set_model(id) {
            Ok(active) => format!(
                "Model switched to {active} (catalog unreachable, ID not verified — /models to confirm)."
            ),
            Err(error) => format!("Model switch failed: {error:#}"),
        },
    }
}

pub(crate) fn load_gateway_session(config_path: &Path, chat_key: &str) -> Result<ChatSession> {
    let map_path = gateway_session_map_path(config_path);
    let session_id = load_session_map(&map_path).ok().and_then(|map| {
        map.get(chat_key)
            .filter(|id| !id.trim().is_empty())
            .cloned()
    });
    if let Some(id) = session_id {
        if let Ok(session) = ChatSession::resume(config_path, &id) {
            return Ok(session);
        }
    }
    let session = ChatSession::load(config_path)?;
    let mut map = load_session_map(&map_path).unwrap_or_default();
    map.insert(chat_key.to_string(), session.session_id().to_string());
    let _ = save_session_map(&map_path, &map);
    Ok(session)
}

fn gateway_session_map_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("gateway-sessions.json")
}

fn load_session_map(path: &Path) -> Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    if path.metadata()?.len() > MAX_SESSION_MAP_BYTES {
        return Err(anyhow!("gateway session map exceeds size limit"));
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn save_session_map(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(map)?)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BotCommand {
    Start,
    Help,
    Status,
    Models { filter: Option<String> },
    Model { id: String },
    Provider { name: String },
    Chat { text: String },
}

pub(crate) fn parse_bot_command(text: &str) -> BotCommand {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return BotCommand::Chat {
            text: trimmed.to_string(),
        };
    }
    let mut parts = trimmed[1..].splitn(2, char::is_whitespace);
    let mut name = parts.next().unwrap_or("");
    if let Some((bare, _)) = name.split_once('@') {
        name = bare;
    }
    let args = parts.next().unwrap_or("").trim();
    match name {
        "start" => BotCommand::Start,
        "help" => BotCommand::Help,
        "status" => BotCommand::Status,
        "models" => BotCommand::Models {
            filter: (!args.is_empty()).then(|| args.to_string()),
        },
        "model" if !args.is_empty() => BotCommand::Model {
            id: args.to_string(),
        },
        "provider" if !args.is_empty() => BotCommand::Provider {
            name: args.to_string(),
        },
        _ => BotCommand::Chat {
            text: trimmed.to_string(),
        },
    }
}

pub(crate) fn chat_allowed(allowlist: &[String], chat_id: i64) -> bool {
    allowlist.is_empty() || allowlist.iter().any(|id| id.trim() == chat_id.to_string())
}

fn split_telegram_text(text: &str) -> Vec<String> {
    split_message_text(text, TELEGRAM_CHUNK_LIMIT)
}

pub(crate) fn split_message_text(text: &str, limit: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec!["(empty reply)".to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.split('\n') {
        if current.len() + line.len() + 1 > limit {
            if !current.trim().is_empty() {
                chunks.push(current.trim_end().to_string());
            }
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim_end().to_string());
    }
    if chunks.is_empty() {
        chunks.push(text.chars().take(limit).collect());
    }
    chunks
}

const HELP_TEXT: &str = "Axiom gateway bot.\n\
    Just write normally to chat.\n\
    /status — active provider and model\n\
    /models [filter] — live catalog search\n\
    /model <exact-id> — switch model\n\
    /provider <name> — switch provider\n\
    /help — this message";

pub(crate) struct BotApprover;

impl SkillApproval for BotApprover {
    fn approve(&mut self, _request: &ApprovalRequest) -> bool {
        false
    }
}

#[derive(Debug, Default, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    #[serde(default)]
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TgMe {
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TgUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
    #[serde(default)]
    channel_post: Option<TgMessage>,
}

#[derive(Debug, Default, Deserialize)]
struct TgMessage {
    #[serde(default)]
    chat: Option<TgChat>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TgChat {
    id: i64,
}

fn update_text(update: &TgUpdate) -> Option<(i64, String)> {
    let message = update.message.as_ref().or(update.channel_post.as_ref())?;
    let text = message.text.as_ref().map(|text| text.trim().to_string())?;
    if text.is_empty() {
        return None;
    }
    Some((message.chat.as_ref()?.id, text))
}

struct TelegramClient {
    http: reqwest::Client,
    base: String,
}

impl TelegramClient {
    fn new(token: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(75))
            .build()?;
        Ok(Self {
            http,
            base: format!("https://api.telegram.org/bot{token}"),
        })
    }

    async fn get_me(&self) -> Result<String> {
        let response: TgResponse<TgMe> = self
            .http
            .get(format!("{}/getMe", self.base))
            .send()
            .await?
            .json()
            .await?;
        if !response.ok {
            return Err(anyhow!(
                "telegram rejected the bot token: {}",
                response.description.unwrap_or_default()
            ));
        }
        Ok(response
            .result
            .and_then(|me| me.username)
            .unwrap_or_else(|| "unknown".to_string()))
    }

    async fn get_updates(&self, offset: i64) -> Result<Vec<TgUpdate>> {
        let response: TgResponse<Vec<TgUpdate>> = self
            .http
            .get(format!("{}/getUpdates", self.base))
            .query(&[
                ("timeout", "50".to_string()),
                ("offset", offset.to_string()),
                (
                    "allowed_updates",
                    "[\"message\",\"channel_post\"]".to_string(),
                ),
            ])
            .send()
            .await?
            .json()
            .await?;
        if !response.ok {
            return Err(anyhow!(
                "telegram getUpdates failed: {}",
                response.description.unwrap_or_default()
            ));
        }
        Ok(response.result.unwrap_or_default())
    }

    async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let response: TgResponse<serde_json::Value> = self
            .http
            .post(format!("{}/sendMessage", self.base))
            .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
            .send()
            .await?
            .json()
            .await?;
        if !response.ok {
            return Err(anyhow!(
                "telegram sendMessage failed: {}",
                response.description.unwrap_or_default()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slash_commands_with_bot_suffixes() {
        assert_eq!(parse_bot_command("/start"), BotCommand::Start);
        assert_eq!(parse_bot_command("/help@mybot"), BotCommand::Help);
        assert_eq!(parse_bot_command("/status"), BotCommand::Status);
        assert_eq!(
            parse_bot_command("/models nemotron"),
            BotCommand::Models {
                filter: Some("nemotron".to_string())
            }
        );
        assert_eq!(
            parse_bot_command("/models"),
            BotCommand::Models { filter: None }
        );
        assert_eq!(
            parse_bot_command("/model nvidia/nemotron-3.5"),
            BotCommand::Model {
                id: "nvidia/nemotron-3.5".to_string()
            }
        );
        assert_eq!(
            parse_bot_command("/provider groq"),
            BotCommand::Provider {
                name: "groq".to_string()
            }
        );
        assert_eq!(
            parse_bot_command("hello there"),
            BotCommand::Chat {
                text: "hello there".to_string()
            }
        );
        assert_eq!(
            parse_bot_command("/model"),
            BotCommand::Chat {
                text: "/model".to_string()
            }
        );
        assert_eq!(
            parse_bot_command("/unknown thing"),
            BotCommand::Chat {
                text: "/unknown thing".to_string()
            }
        );
    }

    #[test]
    fn allowlist_blocks_unknown_chats_only_when_set() {
        assert!(chat_allowed(&[], 123));
        assert!(chat_allowed(&["123".to_string()], 123));
        assert!(!chat_allowed(&["123".to_string()], 456));
        assert!(chat_allowed(&[" 123 ".to_string()], 123));
    }

    #[test]
    fn long_replies_split_on_line_boundaries() {
        let line = "x".repeat(500);
        let text = (0..10)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_telegram_text(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= TELEGRAM_CHUNK_LIMIT);
        }
        assert_eq!(split_telegram_text("  "), vec!["(empty reply)".to_string()]);
    }
}
