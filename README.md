<p align="center">
  <img src="frontend/src/assets/twitch-drops-miner-logo.svg" width="128" alt="Twitch Drops Miner logo">
</p>

<h1 align="center">Twitch Drops Miner</h1>

<p align="center">Mine timed Twitch Drops without streaming video or audio.</p>

<p align="center">
  <a href="https://github.com/ohne-b/twitch-drops-miner?tab=License-1-ov-file"><img src="https://img.shields.io/badge/license-PolyForm_Noncommercial-9146ff" alt="License: PolyForm Noncommercial"></a>
</p>

Choose your games in the app or web dashboard, and the miner watches eligible channels and
claims your drops. It runs on your own hardware with one Twitch account per installation.

![Drops Miner dashboard with a Twitch profile and badge, Fortnite reward progress, channels and queued campaigns](.github/assets/dashboard-mining.png)

[Quick start](#quick-start) · [Dashboard](#using-the-dashboard) · [Updating](#updating) · [Troubleshooting](#troubleshooting)

> [!NOTE]
> This is a personal project for use on your own hardware and home network. VPS/cloud
> hosting and services for other users aren't supported. Twitch changes can break compatibility.

## Quick start

### Desktop app

Download the installer for your computer from [Releases](https://github.com/ohne-b/twitch-drops-miner/releases).

| System | Download |
| --- | --- |
| Windows 10/11 (64-bit) | `twitch-drops-miner-VERSION-windows-x64-setup.exe` |
| macOS 12 or newer (Intel and Apple silicon) | `twitch-drops-miner-VERSION-macos.dmg` |
| Linux (64-bit, Ubuntu 22.04 or newer) | `twitch-drops-miner-VERSION-linux-x64.AppImage` or `.deb` |

### Docker

With Docker Compose installed, save this as `compose.yaml` in a new folder:

```yaml
services:
  twitch-drops-miner:
    image: ghcr.io/ohne-b/twitch-drops-miner:latest
    container_name: twitch-drops-miner
    user: "1000:1000"
    ports:
      - "127.0.0.1:8080:8080"
    volumes:
      - ./data:/app/data
      - ./logs:/app/logs
    restart: unless-stopped
```

Create `data` and `logs` folders beside it. On Linux, make both writable by UID/GID
`1000:1000`. Start Docker, open a terminal in that folder, then run:

```bash
docker compose up -d
```

Open <http://127.0.0.1:8080> and follow [First login](#first-login).
Keep Docker running and the computer awake while mining.

The example allows access only from the same machine. For LAN access, bind an explicit
LAN address and enable [dashboard protection](#data-and-access).

| Registry | Image |
| --- | --- |
| GitHub Container Registry | `ghcr.io/ohne-b/twitch-drops-miner:latest` |
| [Docker Hub](https://hub.docker.com/r/ohneb/twitch-drops-miner) | `ohneb/twitch-drops-miner:latest` |

Both provide the same amd64/arm64 images. `latest` follows stable releases; replace it
with a release version to pin it. See [release notes](https://github.com/ohne-b/twitch-drops-miner/releases)
or the [changelog](CHANGELOG.md).

<details>
<summary>Install Docker</summary>

| System | Guide |
| --- | --- |
| Windows | [Docker Desktop](https://docs.docker.com/desktop/setup/install/windows-install/) |
| macOS | [Docker Desktop](https://docs.docker.com/desktop/setup/install/mac-install/) |
| Linux desktop | [Docker Desktop](https://docs.docker.com/desktop/setup/install/linux/) |
| Linux server | [Docker Engine](https://docs.docker.com/engine/install/) and [Compose](https://docs.docker.com/compose/install/linux/) |

</details>

## First login

1. Open **Settings > Twitch account**. Copy the code, select **Twitch Activate** and
   authorize on Twitch, then select **Done** in the dashboard.
2. Link your game accounts on [Twitch Drops campaigns](https://www.twitch.tv/drops/campaigns).
3. Open **Campaigns** and select **Mine** on a campaign to add its game to your priorities.
4. Use **Mining > Up next > Edit** to reorder games. Leave the miner running.

Your Twitch session survives restarts. Enter your Twitch password only on Twitch's
own authorization page.

> [!WARNING]
> Avoid watching Twitch manually on the same account while mining. Simultaneous viewing
> can interfere with drop progress.

## Using the dashboard

| Page | Use it to |
| --- | --- |
| **Mining** | Check confirmed progress, choose a channel and edit mining preferences from **Up next**. |
| **Campaigns > Available** | Search campaigns, inspect rewards and add games to mining. |
| **Campaigns > History** | Browse recorded claims, including rewards no longer in the catalog. |
| **Activity** | Find session events, warnings and errors; filter or follow new events. |
| **Settings** | View your profile and badges; manage dashboard access, connection and maintenance. |

### Telegram notifications

1. Open [@BotFather](https://t.me/BotFather), send `/newbot` and follow the prompts.
   Choose a username ending in `bot` and copy the token it gives you.
2. Open **your new bot** using BotFather's link and send `/start`. It does not need to reply.
3. In a private browser window, open `https://api.telegram.org/botYOUR_BOT_TOKEN/getUpdates`,
   replacing `YOUR_BOT_TOKEN` with your token. Keep the `bot` prefix.
4. Find the message you just sent. Copy the number under `message` → `chat` → `id`
   (for example, `"chat":{"id":123456789,...}`). If `result` is empty, send your bot `hi` and refresh.
5. In **Settings > Notifications**, enable claim alerts, paste the token and chat ID,
   select **Save**, then **Send test**.

Keep the token and the URL containing it private. If shared, revoke the token through BotFather.

## Updating

**Desktop:** use **Check for updates** in the tray menu or **Settings > Maintenance**.
Download the update, then select **Install and restart**.

**Docker:** Maintenance links to new releases. Install the image from the terminal:

Save your Compose file first and keep the previous image for rollback. If `image:` pins
a version, change it to the version you want.

Download the update, then stop the miner:

```bash
docker compose pull
docker compose stop
```

**Back up the entire `data/` directory while the miner is stopped**, then start the new image:

```bash
docker compose up -d
docker compose ps
docker compose logs --tail=100
```

Keep the service name, mounts, ownership and port binding unchanged. Restarting a container
alone does not install a new image.

<details>
<summary>Update an image built from a checkout</summary>

Back up `docker-compose.yml` before pulling changes:

```bash
git pull --ff-only
docker compose build
docker compose stop
```

Back up `data/`, then run `docker compose up -d --force-recreate` and check the logs.

</details>

<details>
<summary>Upgrading an older installation</summary>

Keep your project directory, service/container name and mounts. The executable is
`twitch-drops-miner`; update custom service commands if they still use the old name.
Older checkouts can update their remote:

```bash
git remote set-url origin https://github.com/ohne-b/twitch-drops-miner.git
```

Python-version settings, history, completed campaigns and dashboard protection remain
compatible. One fresh device-code login is required; old credentials stay untouched for rollback.
Earlier development builds labeled `1.3.2` need a manual upgrade to the current release series.
The historical image remains at `ghcr.io/ohne-b/twitch-miner:0.1.0` for rollback.

</details>

## Data and access

Desktop data stays separate from Docker. Find it through **Settings > Desktop > Open data folder**
or **Open log folder**. On Windows, app data is in `%LOCALAPPDATA%\app.twitch-drops-miner.local`.
Quit the app before backing it up.

Docker uses these mounted folders:

| Folder | Contents |
| --- | --- |
| `data/` → `/app/data` | Settings, Twitch credentials, dashboard sessions, history and claim recovery records. |
| `logs/` → `/app/logs` | Server logs (`TDM.*.log`). |

Keep these directories private. Run only one miner per data directory and stop it before backups.

Dashboard password protection is **off by default**. Enable it in **Settings > Dashboard
access** before allowing LAN or remote access. Use an HTTPS reverse proxy for remote access.
The dashboard password is separate from Twitch; mining continues while the dashboard is locked.

<details>
<summary>Reverse proxy and password settings</summary>

Set `PUBLIC_BASE_URL` under the Compose service's `environment`, preserving existing entries:

```yaml
environment:
  PUBLIC_BASE_URL: https://drops.example.com
```

Use the exact root URL you open in the browser, without a path, query, fragment or credentials.
Recreate the container with `docker compose up -d`. This setting controls the browser origin
and HTTPS cookies; your proxy must provide TLS and forward HTTP and Socket.IO connections.
It does not enable trust of forwarded client-IP headers.

Sessions last up to 30 days; **Remember me** keeps the browser cookie for that period.
Changing the password signs out other sessions. Disabling protection requires the current
password. Dashboard logout does not log the miner out of Twitch.

</details>

<details>
<summary>Forgot the dashboard password</summary>

Stop the miner and restrict network access. Back up and remove **only** `data/web_auth.json`,
restart, then set a new password. Keep all other data.

</details>

<details>
<summary>Clear cache and history</summary>

**Settings > Maintenance > Clear All Cache** removes cached campaign and channel data and
local claim history, then refreshes inventory. It keeps settings, Twitch credentials and
completed campaign records. Cleared history stays cleared; this cannot be undone.

</details>

## Troubleshooting

| Problem | What to try |
| --- | --- |
| Requests time out | In **Settings > Connection**, try **Connection Quality 3** if using 1 or 2. See timeout details below. |
| No campaigns appear | Clear display filters and refresh inventory. For catalog failures, wait and retry; relogging or clearing cache will not add missing feed entries. |
| Mining is idle | Select a game or enable other-game badges/emotes. Check live channels, game-account linking, reward dates, prerequisites and mining filters. |
| Progress is stuck | Compare with [Twitch inventory](https://www.twitch.tv/drops/inventory) and stop simultaneous manual viewing. **Dashboard connected** only describes the browser connection. |
| HTTP 401/403 errors | Reauthorize if Twitch reports an authentication failure. Public-page failures do not invalidate your saved login. |
| Cannot write data or logs | Check directory permissions for UID/GID `1000:1000` and that no second miner uses the data directory. |
| Proxy writes or live updates fail | Open the exact `PUBLIC_BASE_URL` and check that the proxy forwards Socket.IO connections. |

Read recent errors:

```bash
docker compose logs --since=30m --tail=200 twitch-drops-miner
```

Server diagnostics are in `logs/TDM.*.log`, not the Activity page.

<details>
<summary>Connection timeouts</summary>

**Connection Quality** defaults to **3**: 15 seconds to connect and 30 seconds per request.
Existing saved choices are kept. Higher values allow more time but also delay timeout errors.

</details>

<details>
<summary>Advanced diagnostics</summary>

If normal logs do not explain a failure, add `TDM_DIAGNOSTICS: "true"` to the service's
existing `environment` block, keeping its other settings. Then recreate only the miner:

```bash
docker compose up -d --no-deps twitch-drops-miner
```

For the server binary, use `--diagnostics` or `TDM_DIAGNOSTICS=true`.
It is off by default, even with `-v` or `-vv`; startup logs confirm when enabled.

1. Reproduce the problem. Diagnostics cannot recover earlier failures.
2. Review logs before sharing; they may contain account information. Never upload credentials
   or your data directory.
3. Remove the option or set it to `"false"`, then recreate the container or restart the binary.

</details>

## Other installation options

<a id="build-from-a-checkout"></a>
<details>
<summary>Build the Docker image from a checkout</summary>

The repository's [docker-compose.yml](docker-compose.yml) builds locally with the same
mounts, user and loopback binding. With Git and Docker installed:

```bash
git clone https://github.com/ohne-b/twitch-drops-miner.git
cd twitch-drops-miner
```

Create writable `data` and `logs` directories as in [Quick start](#quick-start), then run:

```bash
docker compose up -d --build
```

</details>

<a id="run-from-source"></a>
<details>
<summary>Run from source without Docker</summary>

Install [Rust through rustup](https://rustup.rs/), Node.js 24 and Git. Windows also needs
Visual Studio C++ build tools. The repository pins the Rust toolchain.

```bash
git clone https://github.com/ohne-b/twitch-drops-miner.git
cd twitch-drops-miner
npm --prefix frontend ci
npm --prefix frontend run build
cargo run --locked -- --host 127.0.0.1
```

Open <http://127.0.0.1:8080>. Data and logs use `data/` and `logs/` in the working directory.
After building the frontend, `cargo build --release --locked --bin twitch-drops-miner`
creates an executable in `target/release/` with the dashboard embedded; it needs no Node.js
at runtime. Use `--help` for host, port and directory options.

</details>

<details>
<summary>Try fixes between releases (edge)</summary>

Use `ghcr.io/ohne-b/twitch-drops-miner:edge` in your existing Compose service and follow
[Updating](#updating). Edge is published manually from validated main and retains the Cargo
version; the image's `org.opencontainers.image.revision` label identifies its commit.
Switch back to `:latest` to return to stable releases.

</details>

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for server/desktop development, testing and review requirements.
Report bugs through [GitHub issues](https://github.com/ohne-b/twitch-drops-miner/issues),
including your version, installation method and redacted logs. Never share credentials or device codes.

## License and credits

[PolyForm Noncommercial 1.0.0](https://github.com/ohne-b/twitch-drops-miner?tab=License-1-ov-file), copyright 2026 ohne-b (OhneB).
This is source-available software for noncommercial use, with permission to modify and
redistribute under the license's terms. Keep the license and required notices with copies.
Previously published copies retain their original license terms. Third-party components
retain their own licenses.

Based on [rangermix/TwitchDropsMiner](https://github.com/rangermix/TwitchDropsMiner),
which builds on [DevilXD/TwitchDropsMiner](https://github.com/DevilXD/TwitchDropsMiner),
and their contributors. The full upstream MIT license is preserved in [NOTICE.md](NOTICE.md).
Campaign data comes from the [SunkwiBOT public catalog](https://github.com/SunkwiBOT/twitch-drops-api).
Bundled font and icon notices are in
[frontend/public/assets/licenses](frontend/public/assets/licenses).
