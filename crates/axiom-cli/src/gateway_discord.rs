use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Result};
use axiom_core::AxiomConfig;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;

use crate::gateway_runtime::{load_gateway_session, respond_with_session};

const DISCORD_CHUNK_LIMIT: usize = 2000;
const DISCORD_REST_BASE: &str = "https://discord.com/api/v10";
const DISCORD_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const DISCORD_INTENTS: u64 = 1 | 512 | 4096 | 32768;

pub(crate) async fn run_discord_gateway(config_path: PathBuf) -> Result<()> {
    let config = AxiomConfig::load_from_path(&config_path)?;
    let token_env = config
        .gateway
        .discord_bot_token_env
        .clone()
        .ok_or_else(|| {
            anyhow!("discord is not configured yet. Run `axiom gateway setup` first.")
        })?;
    let token = crate::credentials::resolve_credential(&token_env)?.ok_or_else(|| {
        anyhow!("{token_env} is not set. Save it with `axiom gateway setup` and retry.")
    })?;
    let allowlist = config.gateway.discord_allowed_guild_ids.clone();

    let me = DiscordRest::new(&token).current_username().await?;
    println!("Discord gateway connecting as @{me}. Press Ctrl-C to stop.");
    println!("Note: the MESSAGE CONTENT privileged intent must be enabled for this bot in the Discord developer portal, or message text arrives empty.");
    if allowlist.is_empty() {
        println!(
            "Warning: no allowed server IDs set. Anyone who can reach the bot can talk to it."
        );
        println!("Restrict it with `axiom gateway setup`.");
    }

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Discord gateway stopped.");
                return Ok(());
            }
            outcome = run_connection(&config_path, &token, &allowlist) => {
                if let Err(error) = outcome {
                    eprintln!("discord connection dropped (reconnecting): {error:#}");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn run_connection(config_path: &PathBuf, token: &str, allowlist: &[String]) -> Result<()> {
    let (stream, _) = tokio_tungstenite::connect_async(DISCORD_GATEWAY_URL).await?;
    let (mut sink, mut source) = stream.split();
    let hello: GwEnvelope = read_envelope(&mut source).await?;
    if hello.op != 10 {
        return Err(anyhow!("discord gateway sent op {} before hello", hello.op));
    }
    let interval_ms = hello
        .d
        .get("heartbeat_interval")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("discord hello missed heartbeat_interval"))?;
    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({
            "op": 2,
            "d": {
                "token": token,
                "intents": DISCORD_INTENTS,
                "properties": { "os": "linux", "browser": "axiom", "device": "axiom" },
            }
        })
        .to_string()
        .into(),
    ))
    .await?;

    let sequence: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));
    let heartbeat_sequence = Arc::clone(&sequence);
    let mut heartbeat = tokio::time::interval(Duration::from_millis(interval_ms));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let heartbeat_task = tokio::spawn(async move {
        loop {
            heartbeat.tick().await;
            let payload = {
                let guard = heartbeat_sequence.lock().expect("heartbeat sequence");
                serde_json::json!({ "op": 1, "d": *guard })
            };
            if sink
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    payload.to_string().into(),
                ))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let rest = DiscordRest::new(token);
    let mut bot_id = String::new();
    let outcome = async {
        while let Some(message) = source.next().await {
            let text = match message? {
                tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
                tokio_tungstenite::tungstenite::Message::Close(_) => break,
                _ => continue,
            };
            let envelope: GwEnvelope = serde_json::from_str(&text)?;
            if let Some(seq) = envelope.s {
                *sequence.lock().expect("gateway sequence") = Some(seq);
            }
            match (envelope.op, envelope.t.as_deref()) {
                (0, Some("READY")) => {
                    bot_id = envelope
                        .d
                        .get("user")
                        .and_then(|user| user.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                (0, Some("MESSAGE_CREATE")) => {
                    let incoming: DiscordMessage = serde_json::from_value(envelope.d)?;
                    if incoming.author.bot || incoming.author.id == bot_id {
                        continue;
                    }
                    if incoming.content.trim().is_empty() {
                        continue;
                    }
                    let scope = incoming
                        .guild_id
                        .as_deref()
                        .unwrap_or(incoming.author.id.as_str());
                    if !allowlist.is_empty() && !allowlist.iter().any(|id| id.trim() == scope) {
                        continue;
                    }
                    let mut session = load_gateway_session(
                        config_path,
                        &format!("discord:{}", incoming.channel_id),
                    )?;
                    let reply = respond_with_session(&mut session, incoming.content.trim()).await;
                    for chunk in
                        crate::gateway_runtime::split_message_text(&reply, DISCORD_CHUNK_LIMIT)
                    {
                        if let Err(error) = rest
                            .send_channel_message(&incoming.channel_id, &chunk)
                            .await
                        {
                            eprintln!("discord reply failed: {error:#}");
                            break;
                        }
                    }
                }
                (7, _) | (9, _) => break,
                _ => {}
            }
        }
        Ok(())
    }
    .await;
    heartbeat_task.abort();
    outcome
}

async fn read_envelope<S>(source: &mut S) -> Result<GwEnvelope>
where
    S: futures_util::StreamExt<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    let hello_text = tokio::time::timeout(Duration::from_secs(15), source.next())
        .await?
        .ok_or_else(|| anyhow!("discord gateway closed before hello"))??;
    let text = match hello_text {
        tokio_tungstenite::tungstenite::Message::Text(text) => text.to_string(),
        other => return Err(anyhow!("unexpected discord gateway frame: {other:?}")),
    };
    Ok(serde_json::from_str(&text)?)
}

#[derive(Debug, Deserialize)]
struct GwEnvelope {
    op: i32,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<i64>,
    #[serde(default)]
    t: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordAuthor {
    id: String,
    #[serde(default)]
    bot: bool,
}

#[derive(Debug, Deserialize)]
struct DiscordMessage {
    #[serde(default)]
    channel_id: String,
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    author: DiscordAuthor,
    #[serde(default)]
    content: String,
}

impl Default for DiscordAuthor {
    fn default() -> Self {
        Self {
            id: String::new(),
            bot: false,
        }
    }
}

struct DiscordRest {
    http: reqwest::Client,
    token: String,
}

impl DiscordRest {
    fn new(token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            token: token.to_string(),
        }
    }

    async fn current_username(&self) -> Result<String> {
        let response = self
            .http
            .get(format!("{DISCORD_REST_BASE}/users/@me"))
            .header("Authorization", format!("Bot {}", self.token))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "discord rejected the bot token (HTTP {})",
                response.status()
            ));
        }
        let body: Value = response.json().await?;
        Ok(body
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    async fn send_channel_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let response = self
            .http
            .post(format!(
                "{DISCORD_REST_BASE}/channels/{channel_id}/messages"
            ))
            .header("Authorization", format!("Bot {}", self.token))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("discord send failed (HTTP {})", response.status()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_bot_and_empty_messages() {
        let own = serde_json::json!({
            "channel_id": "7",
            "author": { "id": "9", "bot": true },
            "content": "/status"
        });
        let message: DiscordMessage = serde_json::from_value(own).expect("bot message");
        assert!(message.author.bot);

        let empty = serde_json::json!({
            "channel_id": "7",
            "author": { "id": "9" },
            "content": "   "
        });
        let message: DiscordMessage = serde_json::from_value(empty).expect("empty message");
        assert!(message.content.trim().is_empty());
    }

    #[test]
    fn parses_guild_and_dm_message_shapes() {
        let guild = serde_json::json!({
            "channel_id": "7",
            "guild_id": "42",
            "author": { "id": "9" },
            "content": "hi"
        });
        let message: DiscordMessage = serde_json::from_value(guild).expect("guild message");
        assert_eq!(message.guild_id.as_deref(), Some("42"));

        let dm = serde_json::json!({
            "channel_id": "8",
            "author": { "id": "9" },
            "content": "hi"
        });
        let message: DiscordMessage = serde_json::from_value(dm).expect("dm");
        assert!(message.guild_id.is_none());
    }

    #[test]
    fn discord_scope_prefers_guild_over_author() {
        let guild = serde_json::json!({
            "channel_id": "7",
            "guild_id": "42",
            "author": { "id": "9" },
            "content": "hi"
        });
        let message: DiscordMessage = serde_json::from_value(guild).expect("guild message");
        let scope = message
            .guild_id
            .as_deref()
            .unwrap_or(message.author.id.as_str());
        assert_eq!(scope, "42");
    }
}
