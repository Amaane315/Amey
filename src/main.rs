use reqwest::Client as HttpClient;
use reqwest::StatusCode;
use serde_json::Value;
use serenity::async_trait;
use serenity::builder::GetMessages;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::{Utc, DateTime};

use dotenvy::dotenv;

struct Handler;

struct MemoryKey;

impl TypeMapKey for MemoryKey {
    type Value = Arc<Mutex<HashMap<ChannelId, Vec<String>>>>;
}

struct ApiClientKey;

impl TypeMapKey for ApiClientKey {
    type Value = HttpClient;
}

const MAX_MEMORY_TURNS: usize = 12;

// Logging function for JSON output
fn log_event(event_type: &str, details: serde_json::Value) {
    let timestamp: DateTime<Utc> = Utc::now();
    let log_entry = serde_json::json!({
        "timestamp": timestamp.to_rfc3339(),
        "event_type": event_type,
        "details": details
    });
    println!("{}", serde_json::to_string_pretty(&log_entry).unwrap_or_default());
}

// Helper function to send error messages
async fn send_error(ctx: &Context, msg: &Message, error: &str) {
    log_event("bot_response", serde_json::json!({
        "type": "error",
        "channel_id": msg.channel_id.to_string(),
        "user_id": msg.author.id.to_string(),
        "username": msg.author.name,
        "message": error
    }));

    if let Err(why) = msg.channel_id.say(&ctx.http, error).await {
        eprintln!("Error sending error message: {:?}", why);
    }
}

// Helper function to send success messages
async fn send_success(ctx: &Context, msg: &Message, success: &str) {
    log_event("bot_response", serde_json::json!({
        "type": "success",
        "channel_id": msg.channel_id.to_string(),
        "user_id": msg.author.id.to_string(),
        "username": msg.author.name,
        "message": success
    }));

    if let Err(why) = msg.channel_id.say(&ctx.http, success).await {
        eprintln!("Error sending success message: {:?}", why);
    }
}

fn build_prompt(history: &[String], latest: &str) -> String {
    let mut prompt = String::from("You are a helpful and polite Discord AI assistant. Use the previous conversation context to respond naturally and remember earlier chat history.\n\n");
    for entry in history {
        prompt.push_str(entry);
        prompt.push_str("\n");
    }
    prompt.push_str(&format!("User: {}\nAssistant:", latest));
    prompt
}

async fn call_ollama(client: &HttpClient, endpoint: &str, api_key: &str, model: &str, prompt: &str) -> Option<String> {
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false
    });

    // Retry logic with exponential backoff
    let mut attempts = 0;
    let max_attempts = 2;
    
    loop {
        attempts += 1;
        
        let response = match client
            .post(endpoint)
            .timeout(std::time::Duration::from_secs(30))
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await 
        {
            Ok(resp) => resp,
            Err(err) => {
                log_event("ollama_error", serde_json::json!({
                    "error_type": "request_failed",
                    "error": err.to_string(),
                    "attempt": attempts,
                    "max_attempts": max_attempts,
                    "endpoint": endpoint
                }));
                
                if attempts < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempts as u64)).await;
                    continue;
                }
                return None;
            }
        };

        if response.status() != StatusCode::OK {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            
            log_event("ollama_error", serde_json::json!({
                "error_type": "non_ok_status",
                "status": status.as_u16(),
                "response": text.clone(),
                "attempt": attempts,
                "endpoint": endpoint
            }));
            
            eprintln!("Ollama API error ({}): {}", status, text);
            
            if attempts < max_attempts {
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempts as u64)).await;
                continue;
            }
            return None;
        }

        match response.json::<Value>().await {
            Ok(json) => {
                if let Some(output) = json.get("response").and_then(|v| v.as_str()) {
                    return Some(output.to_string());
                } else {
                    log_event("ollama_error", serde_json::json!({
                        "error_type": "invalid_response_format",
                        "response": json,
                        "attempt": attempts
                    }));
                    return None;
                }
            }
            Err(err) => {
                log_event("ollama_error", serde_json::json!({
                    "error_type": "json_parse_failed",
                    "error": err.to_string(),
                    "attempt": attempts
                }));
                
                if attempts < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempts as u64)).await;
                    continue;
                }
                return None;
            }
        }
    }
}

async fn handle_ai_message(ctx: &Context, msg: &Message, content: &str) {
    let api_key = match env::var("OLLAMA_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
        _ => {
            let fallback = "I am listening! Set OLLAMA_API_KEY in .env to enable smarter AI replies.";
            send_success(ctx, msg, fallback).await;
            return;
        }
    };

    let endpoint = env::var("OLLAMA_API_ENDPOINT")
        .unwrap_or_else(|_| "https://ollama.com/api/generate".to_string());

    let model = env::var("OLLAMA_MODEL")
        .unwrap_or_else(|_| "gemma3:4b".to_string());

    let memory_map = {
        let data = ctx.data.read().await;
        data.get::<MemoryKey>().cloned()
    };
    let http_client = {
        let data = ctx.data.read().await;
        data.get::<ApiClientKey>().cloned()
    };

    if memory_map.is_none() || http_client.is_none() {
        send_success(ctx, msg, "AI memory is not initialized yet.").await;
        return;
    }

    let memory_map = memory_map.unwrap();
    let http_client = http_client.unwrap();

    let ai_response = {
        let mut memory = memory_map.lock().await;
        let history = memory.entry(msg.channel_id).or_default();

        let prompt = build_prompt(history, content);
        let ai_response = match call_ollama(&http_client, &endpoint, &api_key, &model, &prompt).await {
            Some(resp) => resp,
            None => {
                log_event("ollama_failed", serde_json::json!({
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "endpoint": endpoint.clone(),
                    "model": model.clone()
                }));
                let fallback = "⚠️ Ollama ka API abhi available nahi hai. Thoda wait karo or phir dobara try kar! 🔄";
                send_success(ctx, msg, fallback).await;
                return;
            }
        };

        history.push(format!("User: {}", content));
        history.push(format!("Assistant: {}", ai_response));
        if history.len() > MAX_MEMORY_TURNS {
            let excess = history.len() - MAX_MEMORY_TURNS;
            history.drain(0..excess);
        }

        log_event("ai_response", serde_json::json!({
            "channel_id": msg.channel_id.to_string(),
            "user_id": msg.author.id.to_string(),
            "username": msg.author.name,
            "user_message": content,
            "ai_response": ai_response.clone(),
            "memory_turns": history.len()
        }));

        ai_response
    };

    send_success(ctx, msg, &ai_response).await;
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Log incoming message
        log_event("message_received", serde_json::json!({
            "channel_id": msg.channel_id.to_string(),
            "user_id": msg.author.id.to_string(),
            "username": msg.author.name,
            "content": msg.content,
            "timestamp": msg.timestamp.to_string(),
            "mentions": msg.mentions.iter().map(|u| u.name.clone()).collect::<Vec<_>>()
        }));

        // Ignore bot's own messages
        if msg.author.bot {
            return;
        }

        let content = msg.content.trim();

        match content {
            "!ping" => {
                log_event("command_executed", serde_json::json!({
                    "command": "ping",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                send_success(&ctx, &msg, "Pong! 🏓").await;
            }
            "!help" => {
                log_event("command_executed", serde_json::json!({
                    "command": "help",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                let help_text = r#"**Available Commands:**
```
!ping - Check if the bot is responsive
!help - Show this help message
!create_role <role_name> - Create a new role
!assign_role <@user> <role_name> - Assign a role to a user
!remove_role <@user> <role_name> - Remove a role from a user
!set_nickname <@user> <nickname> - Set a user's nickname
!set_all_amey - Set all members' nicknames to Amey
!userinfo <@user> - Get information about a user
!serverinfo - Get information about the server
!memory_show - Show saved conversation memory for this channel
!memory_clear - Clear saved conversation memory for this channel
!clear_chat <@user> - Delete all messages from the mentioned user in this channel
```"#;
                send_success(&ctx, &msg, help_text).await;
            }
            "!serverinfo" => {
                log_event("command_executed", serde_json::json!({
                    "command": "serverinfo",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                if let Some(guild_id) = msg.guild_id {
                    if let Ok(guild) = guild_id.to_partial_guild(&ctx.http).await {
                        let info = format!(
                            "**Server Information:**\nName: {}\nOwner ID: {}",
                            guild.name, guild.owner_id
                        );
                        send_success(&ctx, &msg, &info).await;
                    } else {
                        send_error(&ctx, &msg, "Failed to fetch server information").await;
                    }
                } else {
                    send_error(&ctx, &msg, "This command only works in a server").await;
                }
            }
            _ if content.starts_with("!userinfo") => {
                log_event("command_executed", serde_json::json!({
                    "command": "userinfo",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "target_user": msg.mentions.first().map(|u| u.name.clone())
                }));
                if let Some(user) = msg.mentions.first() {
                    let info = format!(
                        "**User Information:**\nUsername: {}\nID: {}\nCreated: {}",
                        user.name, user.id, user.created_at()
                    );
                    send_success(&ctx, &msg, &info).await;
                } else {
                    send_error(&ctx, &msg, "Please mention a user").await;
                }
            }
            _ if content.starts_with("!create_role ") => {
                let role_name = content.trim_start_matches("!create_role ").to_string();
                if role_name.is_empty() {
                    send_error(&ctx, &msg, "Please provide a role name").await;
                    return;
                }

                if let Some(guild_id) = msg.guild_id {
                    match guild_id.create_role(&ctx.http, serenity::builder::EditRole::new().name(&role_name)).await {
                        Ok(role) => {
                            send_success(&ctx, &msg, &format!("✅ Created role: {}", role.name)).await;
                        }
                        Err(why) => {
                            eprintln!("Error creating role: {:?}", why);
                            send_error(&ctx, &msg, "Failed to create role").await;
                        }
                    }
                } else {
                    send_error(&ctx, &msg, "This command only works in a server").await;
                }
            }
            _ if content.starts_with("!assign_role ") => {
                let parts: Vec<&str> = content.split_whitespace().collect();
                log_event("command_executed", serde_json::json!({
                    "command": "assign_role",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "target_user": msg.mentions.first().map(|u| u.name.clone()),
                    "role_name": parts.get(2..).map(|p| p.join(" ")).unwrap_or_default()
                }));
                if parts.len() < 3 {
                    send_error(&ctx, &msg, "Usage: !assign_role <@user> <role_name>").await;
                    return;
                }

                let role_name = parts[2..].join(" ");
                if let Some(user) = msg.mentions.first() {
                    if let Some(guild_id) = msg.guild_id {
                        match guild_id.member(&ctx.http, user.id).await {
                            Ok(member) => {
                                if let Ok(roles) = guild_id.roles(&ctx.http).await {
                                    if let Some(role) = roles.values().find(|r| r.name == role_name) {
                                        match member.add_role(&ctx.http, role.id).await {
                                            Ok(_) => {
                                                send_success(&ctx, &msg, &format!("✅ Assigned role {} to {}", role_name, user.name)).await;
                                            }
                                            Err(why) => {
                                                eprintln!("Error adding role: {:?}", why);
                                                send_error(&ctx, &msg, "Failed to assign role").await;
                                            }
                                        }
                                    } else {
                                        send_error(&ctx, &msg, "Role not found").await;
                                    }
                                }
                            }
                            Err(why) => {
                                eprintln!("Error fetching member: {:?}", why);
                                send_error(&ctx, &msg, "Failed to fetch member").await;
                            }
                        }
                    }
                } else {
                    send_error(&ctx, &msg, "Please mention a user").await;
                }
            }
            _ if content.starts_with("!remove_role ") => {
                let parts: Vec<&str> = content.split_whitespace().collect();
                log_event("command_executed", serde_json::json!({
                    "command": "remove_role",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "target_user": msg.mentions.first().map(|u| u.name.clone()),
                    "role_name": parts.get(2..).map(|p| p.join(" ")).unwrap_or_default()
                }));
                if parts.len() < 3 {
                    send_error(&ctx, &msg, "Usage: !remove_role <@user> <role_name>").await;
                    return;
                }

                let role_name = parts[2..].join(" ");
                if let Some(user) = msg.mentions.first() {
                    if let Some(guild_id) = msg.guild_id {
                        match guild_id.member(&ctx.http, user.id).await {
                            Ok(member) => {
                                if let Ok(roles) = guild_id.roles(&ctx.http).await {
                                    if let Some(role) = roles.values().find(|r| r.name == role_name) {
                                        match member.remove_role(&ctx.http, role.id).await {
                                            Ok(_) => {
                                                send_success(&ctx, &msg, &format!("✅ Removed role {} from {}", role_name, user.name)).await;
                                            }
                                            Err(why) => {
                                                eprintln!("Error removing role: {:?}", why);
                                                send_error(&ctx, &msg, "Failed to remove role").await;
                                            }
                                        }
                                    } else {
                                        send_error(&ctx, &msg, "Role not found").await;
                                    }
                                }
                            }
                            Err(why) => {
                                eprintln!("Error fetching member: {:?}", why);
                                send_error(&ctx, &msg, "Failed to fetch member").await;
                            }
                        }
                    }
                } else {
                    send_error(&ctx, &msg, "Please mention a user").await;
                }
            }
            _ if content.starts_with("!set_nickname ") => {
                let parts: Vec<&str> = content.split_whitespace().collect();
                log_event("command_executed", serde_json::json!({
                    "command": "set_nickname",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "target_user": msg.mentions.first().map(|u| u.name.clone()),
                    "nickname": parts.get(2..).map(|p| p.join(" ")).unwrap_or_default()
                }));
                if parts.len() < 3 {
                    send_error(&ctx, &msg, "Usage: !set_nickname <@user> <nickname>").await;
                    return;
                }

                let nickname = parts[2..].join(" ");
                if let Some(user) = msg.mentions.first() {
                    if let Some(guild_id) = msg.guild_id {
                        match guild_id.member(&ctx.http, user.id).await {
                            Ok(mut member) => {
                                match member.edit(&ctx.http, serenity::builder::EditMember::new().nickname(&nickname)).await {
                                    Ok(_) => {
                                        send_success(&ctx, &msg, &format!("✅ Set nickname to {} for {}", nickname, user.name)).await;
                                    }
                                    Err(why) => {
                                        eprintln!("Error setting nickname: {:?}", why);
                                        send_error(&ctx, &msg, "Failed to set nickname").await;
                                    }
                                }
                            }
                            Err(why) => {
                                eprintln!("Error fetching member: {:?}", why);
                                send_error(&ctx, &msg, "Failed to fetch member").await;
                            }
                        }
                    }
                } else {
                    send_error(&ctx, &msg, "Please mention a user").await;
                }
            }
            "!set_all_amey" => {
                log_event("command_executed", serde_json::json!({
                    "command": "set_all_amey",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                if let Some(guild_id) = msg.guild_id {
                    match guild_id.members(&ctx.http, None, None).await {
                        Ok(members) => {
                            let mut count = 0;
                            for member in members {
                                if let Ok(mut m) = guild_id.member(&ctx.http, member.user.id).await {
                                    if m.edit(&ctx.http, serenity::builder::EditMember::new().nickname("Amey")).await.is_ok() {
                                        count += 1;
                                    }
                                }
                            }
                            send_success(&ctx, &msg, &format!("✅ Set {} members' nicknames to Amey", count)).await;
                        }
                        Err(why) => {
                            eprintln!("Error fetching members: {:?}", why);
                            send_error(&ctx, &msg, "Failed to fetch members").await;
                        }
                    }
                }
            }
            _ if content.starts_with("!clear_chat ") => {
                log_event("command_executed", serde_json::json!({
                    "command": "clear_chat",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name,
                    "target_user": msg.mentions.first().map(|u| u.name.clone())
                }));
                if let Some(user) = msg.mentions.first() {
                    // Check if user has permission to delete messages (server owner or admin)
                    let has_permission = if let Some(guild_id) = msg.guild_id {
                        if let Ok(guild) = guild_id.to_partial_guild(&ctx.http).await {
                            // Check if user is server owner
                            if msg.author.id == guild.owner_id {
                                true
                            } else {
                                // Check if user has admin role
                                match guild_id.member(&ctx.http, msg.author.id).await {
                                    Ok(member) => {
                                        // Check if user has any role with ADMINISTRATOR permission
                                        if let Ok(roles) = guild_id.roles(&ctx.http).await {
                                            member.roles.iter().any(|role_id| {
                                                roles.get(role_id)
                                                    .map(|role| role.permissions.administrator())
                                                    .unwrap_or(false)
                                            })
                                        } else {
                                            false
                                        }
                                    }
                                    Err(_) => false
                                }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !has_permission {
                        send_error(&ctx, &msg, "❌ Only server owner or admins can use this command!").await;
                        return;
                    }

                    // Get channel messages and filter by user
                    match msg.channel_id.messages(&ctx.http, GetMessages::new().limit(100)).await {
                        Ok(messages) => {
                            let user_messages: Vec<_> = messages.into_iter()
                                .filter(|m| m.author.id == user.id)
                                .collect();

                            if user_messages.is_empty() {
                                send_success(&ctx, &msg, &format!("No messages found from {} in this channel.", user.name)).await;
                                return;
                            }

                            let mut deleted_count = 0;
                            for message in user_messages {
                                if let Err(why) = msg.channel_id.delete_message(&ctx.http, message.id).await {
                                    eprintln!("Error deleting message {}: {:?}", message.id, why);
                                } else {
                                    deleted_count += 1;
                                }
                            }

                            log_event("messages_deleted", serde_json::json!({
                                "channel_id": msg.channel_id.to_string(),
                                "deleted_by": msg.author.id.to_string(),
                                "target_user": user.id.to_string(),
                                "deleted_count": deleted_count
                            }));

                            send_success(&ctx, &msg, &format!("✅ Deleted {} messages from {} in this channel.", deleted_count, user.name)).await;
                        }
                        Err(why) => {
                            eprintln!("Error fetching messages: {:?}", why);
                            send_error(&ctx, &msg, "Failed to fetch messages from this channel.").await;
                        }
                    }
                } else {
                    send_error(&ctx, &msg, "Usage: !clear_chat <@user>").await;
                }
            }
            "!memory_clear" => {
                log_event("command_executed", serde_json::json!({
                    "command": "memory_clear",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                let data = ctx.data.read().await;
                if let Some(memory_map) = data.get::<MemoryKey>() {
                    let mut memory = memory_map.lock().await;
                    memory.remove(&msg.channel_id);
                    send_success(&ctx, &msg, "✅ Memory cleared for this channel.").await;
                } else {
                    send_error(&ctx, &msg, "Memory storage not initialized.").await;
                }
            }
            "!memory_show" => {
                log_event("command_executed", serde_json::json!({
                    "command": "memory_show",
                    "channel_id": msg.channel_id.to_string(),
                    "user_id": msg.author.id.to_string(),
                    "username": msg.author.name
                }));
                let data = ctx.data.read().await;
                if let Some(memory_map) = data.get::<MemoryKey>() {
                    let memory = memory_map.lock().await;
                    if let Some(history) = memory.get(&msg.channel_id) {
                        let display = history.join("\n");
                        let summary = if display.len() > 1900 {
                            format!("{}...", &display[..1900])
                        } else {
                            display
                        };
                        send_success(&ctx, &msg, &format!("**Memory for this channel:**\n{}", summary)).await;
                    } else {
                        send_success(&ctx, &msg, "No saved memory for this channel.").await;
                    }
                } else {
                    send_error(&ctx, &msg, "Memory storage not initialized.").await;
                }
            }
            _ => {
                handle_ai_message(&ctx, &msg, content).await;
            }
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        log_event("bot_ready", serde_json::json!({
            "bot_name": ready.user.name,
            "bot_id": ready.user.id.to_string(),
            "guilds_count": ready.guilds.len(),
            "session_id": ready.session_id
        }));

        println!("✅ {} is connected and ready!", ready.user.name);
    }
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env if present.
    dotenv().ok();

    // Configure the client with your Discord bot token in the environment.
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");

    // Build our client.
    let mut client = Client::builder(
        &token,
        GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MEMBERS
            | GatewayIntents::GUILDS,
    )
    .event_handler(Handler)
    .await
    .expect("Error creating client");

    {
        let mut data = client.data.write().await;
        data.insert::<MemoryKey>(Arc::new(Mutex::new(HashMap::new())));
        data.insert::<ApiClientKey>(HttpClient::new());
    }

    // Start the client.
    if let Err(why) = client.start().await {
        eprintln!("Client error: {:?}", why);
    }
}
