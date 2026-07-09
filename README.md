# 🤖 Little Bobby TaBots

[![Rust](https://img.shields.io/badge/Language-Rust-orange.svg)](https://www.rust-lang.org/)
[![Docker](https://img.shields.io/badge/Container-Docker%20Compose-blue.svg)](https://www.docker.com/)
[![Security Audit](https://img.shields.io/badge/Security-100%25%20Vulnerability%20Free-success.svg)](https://rustsec.org/)

**Little Bobby TaBots** is a highly optimized, lightweight, self-hosted Discord music bot written in Rust. It is designed to run locally or inside a Docker container with minimal memory and CPU footprints. 

*Name origin: A nod to the classic XKCD "Robert'); DROP TABLE Students;--" comic.*

---

## ✨ Features

*   **Native Audio Decoding**: Uses `symphonia` natively inside the Rust binary to decode Ogg, MP3, FLAC, and AAC, avoiding heavy external background decoding loops.
*   **Discord Voice E2EE**: Built on `songbird 0.6.0` to support Discord's mandatory end-to-end voice encryption (DAVE protocol), eliminating websocket timeouts and disconnects.
*   **Vulnerability-Free**: Uses `native-tls` (OpenSSL) backend to keep compile-time dependencies 100% free of security advisories. Fully audited via `cargo audit`.
*   **Streamed Playback**: Downloads nothing to disk. Audio is piped directly from `yt-dlp` via `ffmpeg` to memory buffers, leaving zero temp file waste.
*   **Slash Commands**: Supports full modern slash interaction registry with guild-level instant registration.

---

## 📋 Commands Reference

| Command | Description |
| :--- | :--- |
| `/play <query>` | Connects to your voice channel and plays/queues a song (searches YouTube/SoundCloud or accepts direct URLs). An explicit YouTube `/playlist?list=...` URL queues every resolvable video in playlist order. |
| `/pause` | Pauses playback of the current track. |
| `/resume` | Resumes playing the paused track. |
| `/skip` | Skips the current track and starts the next one in the queue. |
| `/queue` | Shows an embed list of the currently playing track and the upcoming playlist. |
| `/leave` | Stops playback, clears the queue, and disconnects the bot from the voice channel. |
| `/ping` | A diagnostics command to confirm the bot is active and responsive. |

---

## 🚀 Getting Started

### 1. Discord Bot Setup
1. Go to the [Discord Developer Portal](https://discord.com/developers/applications).
2. Create a new Application called **Little Bobby TaBots** (or your preferred name).
3. Under the **Bot** tab, create a bot user and copy the **Token**.
4. Scroll down under the **Bot** tab and ensure **Guild Voice States** intent is enabled under **Privileged Gateway Intents**.
5. Under the **Installation** tab, set scopes to `bot` and `applications.commands`. Under permissions, grant `Connect`, `Speak`, and `Send Messages`. 
6. Use the generated link to invite the bot to your Discord server.

### 2. Configuration
Create a `.env` file in the project root:
```env
DISCORD_TOKEN=your_copied_discord_bot_token_here
GUILD_ID=your_test_server_id_here
```
> [!NOTE]
> Setting `GUILD_ID` registers slash commands instantly in your test server on bot startup. Without it, commands are registered globally and can take up to an hour to populate.

---

## 🐳 Running with Docker Compose (Recommended)

The easiest way to run the bot is containerized via Docker Compose. The multi-stage build compiles a statically linked `musl` release binary and bundles it inside a lightweight Alpine container with `ffmpeg` and `yt-dlp` pre-configured.

1.  Start the bot container in the background:
    ```bash
    docker compose up -d
    ```
2.  Follow the live logs to confirm it successfully connects:
    ```bash
    docker compose logs -f
    ```
3.  Stop the bot container:
    ```bash
    docker compose down
    ```

---

## 🛠️ Running Locally (Without Docker)

To run the project directly from your shell, you will need the following installed:
*   [Rust toolchain (stable)](https://rustup.rs/)
*   [ffmpeg](https://ffmpeg.org/) (must be in your system `PATH`)
*   [yt-dlp](https://github.com/yt-dlp/yt-dlp) (must be in your system `PATH`)

1.  Compile and run the release binary:
    ```bash
    cargo run --release
    ```

---

## 🔒 Security Auditing

To run a security check on dependencies:
```bash
cargo install cargo-audit
cargo audit
```
*Current status:* **`error: 0 vulnerabilities found!`**
