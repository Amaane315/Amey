# Amey Discord Bot

A powerful Discord bot built with Rust using the Serenity library. This bot provides role management, nickname management, and utility commands.

## Features

- **Moderation Commands**
  - `!create_role <role_name>` - Create a new server role
  - `!assign_role <@user> <role_name>` - Assign a role to a user
  - `!remove_role <@user> <role_name>` - Remove a role from a user
  - `!set_nickname <@user> <nickname>` - Set a user's nickname
  - `!set_all_amey` - Set all members' nicknames to "Amey"

- **Utility Commands**
  - `!ping` - Check if the bot is responsive
  - `!help` - Show all available commands
  - `!serverinfo` - Get information about the current server
  - `!userinfo <@user>` - Get information about a user

## Requirements

- Rust (1.70+)
- A Discord bot token (get from [Discord Developer Portal](https://discord.com/developers/applications))

## Setup

### 1. Clone and build the project

```bash
cd Amey
cargo build --release
```

### 2. Create a Discord bot

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Click "New Application" and give it a name
3. Go to "Bot" tab and click "Add Bot"
4. Under the TOKEN section, click "Copy" to copy your bot token
5. Save this token safely

### 3. Set the bot token and Ollama key

Create a `.env` file in the project directory or set the environment variables:

```bash
export DISCORD_TOKEN="your_bot_token_here"
export OLLAMA_API_KEY="your_ollama_api_key_here"
export OLLAMA_MODEL="gemma3:4b"
```

**Note:** The bot uses Ollama Gemma 3 4B model for AI responses. Make sure your Ollama API key is valid.

### 4. Invite the bot to your server

1. In Developer Portal, go to OAuth2 > URL Generator
2. Select scopes: `bot`
3. Select permissions:
   - `Send Messages`
   - `Manage Roles`
   - `Manage Nicknames`
   - `Read Messages/View Channels`
4. Copy the generated URL and open it in your browser

### 5. Run the bot

```bash
# Set your token and Gemini key first
export DISCORD_TOKEN="your_token_here"
export GEMINI_API_KEY="your_gemini_api_key_here"

# Run the bot in development mode
cargo run
```

You should see output like:
```
✅ Amey is connected and ready!
```

## AI Features

The bot now includes Ollama AI integration for intelligent chat responses:

- **Smart Replies**: Responds to any chat message with AI-generated responses using Gemma 3 4B model
- **Conversation Memory**: Remembers recent chat history per channel (up to 12 turns)
- **Memory Management**: Use `!memory_show` to view saved history and `!memory_clear` to reset

## Commands Reference

| Command | Usage | Example |
|---------|-------|---------|
| `!ping` | Check bot status | `!ping` → `Pong! 🏓` |
| `!help` | Show all commands | `!help` |
| `!serverinfo` | Get server details | `!serverinfo` |
| `!userinfo` | Get user details | `!userinfo @user` |
| `!create_role` | Create a role | `!create_role Admins` |
| `!assign_role` | Assign role to user | `!assign_role @user Admin` |
| `!remove_role` | Remove role from user | `!remove_role @user Admin` |
| `!set_nickname` | Set user's nickname | `!set_nickname @user NewName` |
| `!set_all_amey` | Set all to "Amey" | `!set_all_amey` |
| `!memory_show` | Show saved chat memory | `!memory_show` |
| `!memory_clear` | Clear saved chat memory | `!memory_clear` |

## Bot Permissions Required

Make sure your bot has these permissions in your server:
- ✅ Send Messages
- ✅ Manage Roles
- ✅ Manage Nicknames
- ✅ Read Messages/View Channels

## Troubleshooting

### Bot not responding to commands?
- Check that `DISCORD_TOKEN` is set correctly
- Ensure MESSAGE_CONTENT intent is enabled in Developer Portal
- Verify the bot has permissions in the channel

### "Role not found" error?
- Make sure the role name is exactly as it appears in your server
- Check that the bot's role is higher than the target role in role hierarchy

### Ollama API not working?
- Verify `OLLAMA_API_KEY` is set correctly in `.env`
- Check that your Ollama API key is valid and has credits
- Ensure the API key is not expired
- The bot will fall back to basic responses if Ollama is unavailable

## Development

To modify the bot:

1. Edit `src/main.rs`
2. Test changes: `cargo check`
3. Build: `cargo build --release`
4. Run: `cargo run`

## Dependencies

- **serenity** - Discord API wrapper
- **tokio** - Async runtime

## License

MIT License

## Support

For issues or questions, check the [Serenity documentation](https://docs.rs/serenity/) or [Discord.py documentation](https://discord.py.readthedocs.io/).
