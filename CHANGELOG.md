# Changelog

## [v2.1.0-rc.1](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.1.0-rc.1) — 2026-10-11

- add optional Telegram notifications for claimed rewards on desktop and server, with delivery separate from mining
- configure, test and remove Telegram notifications in Settings > Notifications; bot tokens stay private
- remove the redundant terminal-update hint from Maintenance

## [v2.0.4](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.4) — 2026-10-10

- hide expired campaigns from Available and remove the Expired filter, while keeping History and pending claims intact
- compact mobile campaign cards with centered artwork, short dates and reward counts beside the action buttons
- give the desktop campaign list and details equal width with a tighter gap, and restore focus to the button that opened details
- remove Cancel during desktop update checks; downloads can still be cancelled

## [v2.0.3](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.3) — 2026-10-09

- show game search results in a taller dropdown over the selected games, with keyboard navigation and responsive positioning ([#89](https://github.com/ohne-b/twitch-drops-miner/pull/89))
- remove the duplicate waiting message after confirming Twitch device login ([#88](https://github.com/ohne-b/twitch-drops-miner/pull/88))

## [v2.0.2](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.2) — 2026-10-09

- show the installed version and update checks inline in Settings > Maintenance on desktop and server
- keep desktop updates, cancellation and install recovery on that page, with an Update button and download icon
- shorten desktop folder shortcuts to Data and Logs, and remove server-only shutdown controls from desktop settings
- align Activity rows with and without campaign detail buttons while preserving room for longer messages ([#84](https://github.com/ohne-b/twitch-drops-miner/issues/84))
- add a link to the signed-in Twitch profile beside the account in Settings ([#85](https://github.com/ohne-b/twitch-drops-miner/pull/85))

## [v2.0.1](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.1) — 2026-10-08

- fix Twitch Activate and other external links opening in the desktop app's default browser
- use `app.twitch-drops-miner.local` as the desktop app identifier and data-folder name

Desktop settings and login start fresh. The old 2.0.0 data folder is left untouched; there is no migration or deletion.
Docker settings, credentials and mounts are unchanged.

## [v2.0.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.0) — 2026-10-08

- add a desktop app for Windows, macOS and Linux, using the same mining core and dashboard as Docker ([#79](https://github.com/ohne-b/twitch-drops-miner/pull/79))
- add tray controls and mining status, optional startup at sign-in, window-state restoration and reward notifications
- add signed in-app updates, with an update link in the sidebar and explicit download and install actions
- add optional sleep prevention while mining, plus keyboard and mouse-wheel zoom; display sleep and manual sleep remain available
- check stream playlists and new segment response headers alongside watch telemetry, preserving the watch cadence and Twitch-confirmed progress ([#80](https://github.com/ohne-b/twitch-drops-miner/pull/80))

Downloads include a Windows x64 installer, a universal macOS DMG, and Linux x64 AppImage/DEB packages.
Existing Docker settings, credentials, history and container mounts remain compatible.
The desktop app stores its data separately and requires its own Twitch login; it does not import a Docker installation.
Windows installers have no Authenticode signature; macOS uses ad-hoc signing without notarization.
In-app update packages are signed and verified separately.

## [v1.6.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.6.0) — 2026-10-07

- add Pause and Resume to Now mining, keeping selected games and current progress; pause persists across restarts while inventory refreshes and earned claims continue ([#75](https://github.com/ohne-b/twitch-drops-miner/pull/75), [#77](https://github.com/ohne-b/twitch-drops-miner/pull/77))
- search Twitch games without active campaigns, save their official names and covers, and fill in artwork for previously selected games ([#76](https://github.com/ohne-b/twitch-drops-miner/pull/76))
- show confirmed mining progress and Paused, Idle or Disconnected status in the browser tab
- simplify setup and troubleshooting in the README and update its dashboard screenshot

Existing settings, credentials, history, service names and container mounts remain compatible.
Pause is off by default. Manual-channel timers continue counting down while paused.

## [v1.5.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.5.0) — 2026-10-05

- show your Twitch avatar, equipped badges and chat-colored name in the sidebar, with dark colors lightened for readability ([#71](https://github.com/ohne-b/twitch-drops-miner/pull/71))
- add the account identity and badge collection to Settings > Twitch account, with badge descriptions and explicit feedback when Twitch does not provide the full collection
- simplify the Campaigns count, place it beside refresh and center desktop pagination; keep phone pagination beside the tabs
- show an empty progress bar for known rewards at zero minutes, keep Last confirmed beside the selection mode inside Now mining, and balance the card's artwork and spacing ([#70](https://github.com/ohne-b/twitch-drops-miner/pull/70))

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining selection, watch cadence and claim requirements are unchanged.

## [v1.4.4](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.4.4) — 2026-10-04

- keep campaign tabs, search and filters full width above the list and details, with fixed detail headers and reachable controls in short windows ([#65](https://github.com/ohne-b/twitch-drops-miner/pull/65))
- move campaign pagination into the top toolbar, remove the bottom pagination gap and wrap crowded controls on narrow phones
- move Last confirmed into the Mining page header with smaller, darker text and a compact phone layout ([#68](https://github.com/ohne-b/twitch-drops-miner/pull/68))
- show `0 / required minutes` for unconfirmed rewards in Mining and Campaigns, with an unconfirmed tooltip and no invented timestamps
- switch the project license to PolyForm Noncommercial 1.0.0, with matching package and image metadata; preserve upstream MIT and third-party notices

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining selection, watch cadence and claim requirements are unchanged.

## [v1.4.3](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.4.3) — 2026-10-04

- correct conflicting live progress using newer Twitch account inventory, keep watching when only the disputed counter reaches 100%, and preserve pending reconciliation across reconnects ([#63](https://github.com/ohne-b/twitch-drops-miner/pull/63))
- use 24-hour time throughout the dashboard, including Activity and tooltips, while preserving local dates and timezones
- offer Docker Hub images alongside GHCR through a shared mirror that copies both supported architectures without rebuilding ([#62](https://github.com/ohne-b/twitch-drops-miner/pull/62))
- replace duplicate comparison links in release notes with a single Changelog link ([#61](https://github.com/ohne-b/twitch-drops-miner/pull/61))

Existing settings, credentials, history, service names and container mounts remain compatible.
Watch cadence and claim requirements are unchanged.

## [v1.4.2](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.4.2) — 2026-10-04

- align narrow campaign cards to equal row heights, with readable names and counts and actions below the campaign identity ([#56](https://github.com/ohne-b/twitch-drops-miner/pull/56))
- return campaign details to the originating Mining or Activity view with its search, filters, scroll position and keyboard focus; keep Settings tabs from scrolling the page ([#57](https://github.com/ohne-b/twitch-drops-miner/pull/57))
- show claim dates beside claimed rewards without a duplicate History section, omit repeated reward names, show shared Up next deadlines once, and remove the reward selection stripe
- preserve complete reward artwork and use matching game artwork for channels when their own image is missing
- give mobile Campaigns a full-width search field, fit Activity to the available viewport, and distinguish an empty activity log from unmatched filters
- align mining list dividers, add compact scrollbar spacing and retain full-size phone touch targets
- simplify Mining preferences with shorter labels and accessible info buttons for priority and reward rules ([#59](https://github.com/ohne-b/twitch-drops-miner/pull/59), [#60](https://github.com/ohne-b/twitch-drops-miner/pull/60))
- default Connection Quality to 3 (15-second connect and 30-second request timeouts), preserve saved choices, and shorten troubleshooting guidance ([#60](https://github.com/ohne-b/twitch-drops-miner/pull/60))

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining selection, watch cadence and claim requirements are unchanged.

## [v1.4.1](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.4.1) — 2026-10-04

- align Available and History cards and list rows, with matching artwork, spacing, borders and inset dividers; keep phone text readable by placing counts and actions below the campaign identity ([#53](https://github.com/ohne-b/twitch-drops-miner/pull/53))
- prevent background scrolling behind mobile campaign details and preserve the list position when closing details on phones and tablets ([#54](https://github.com/ohne-b/twitch-drops-miner/pull/54))
- publish the production images already tested by validation, and speed up CI with Rust dependency caching and a shared fixture build for isolated browser jobs ([#51](https://github.com/ohne-b/twitch-drops-miner/pull/51), [#52](https://github.com/ohne-b/twitch-drops-miner/pull/52))

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining selection, watch cadence and claim requirements are unchanged.

## [v1.4.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.4.0) — 2026-10-03

- redesign the dashboard around Mining, Campaigns, Activity and Settings with compact icon actions and responsive layouts
- add a dedicated mining-preferences editor with a scrollable game list, visible drag handles and a direct return to Mining
- open campaign and reward details in a dedicated pane, preserve list filters and position, and keep History limited to recorded claims
- add typed Activity events with category and severity filters, repeat counts and explicit recovery
- separate application state from web transports, publish revisioned snapshots and patches, and preserve unsaved settings through reconnects and the offered compatibility reload
- simplify Settings tabs, campaign navigation and refresh controls while retaining keyboard access and mobile layouts

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining selection, watch cadence and claim requirements are unchanged. Version-one durable
records remain independent of dashboard presentation fields. Reload the dashboard after upgrading.

## [v1.3.1](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.3.1) — 2026-10-02

- fix the v1.3.0 inventory refresh and campaign discovery regression when unrestricted public campaigns omit their unused channel list; preserve strict Twitch account validation and claim evidence ([#47](https://github.com/ohne-b/twitch-drops-miner/pull/47))

Existing settings, credentials, history, service names and container mounts remain compatible.
Unavailable, stale or malformed catalog data still reports incomplete refreshes.

## [v1.3.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.3.0) — 2026-10-02

- add the Mining priority selector with Default (manual order), Short events first and Ending soonest; preserve saved game order, manual-channel timers and selected-game priority ([#45](https://github.com/ohne-b/twitch-drops-miner/pull/45))
- make dashboard actions more compact, keep native sort menus legible, and show a temporary copy-success tick without shifting the Twitch authorization row ([#36](https://github.com/ohne-b/twitch-drops-miner/pull/36), [#37](https://github.com/ohne-b/twitch-drops-miner/pull/37), [#38](https://github.com/ohne-b/twitch-drops-miner/pull/38), [#39](https://github.com/ohne-b/twitch-drops-miner/pull/39))
- use fixed-size icons for update checks, manual-channel mining and inventory refresh, retaining accessible progress, success and retry feedback ([#40](https://github.com/ohne-b/twitch-drops-miner/pull/40), [#41](https://github.com/ohne-b/twitch-drops-miner/pull/41))
- accept empty CurrentDrop sessions as no result and strengthen inventory validation so malformed or duplicate account records cannot invent eligibility or claim history ([#43](https://github.com/ohne-b/twitch-drops-miner/pull/43), [#44](https://github.com/ohne-b/twitch-drops-miner/pull/44))
- retry interrupted read responses only when safe to replay, preserve token refresh after interrupted validation rejections, and keep notification failures from delaying mining ([#44](https://github.com/ohne-b/twitch-drops-miner/pull/44))

Existing settings, credentials, history, service names and container mounts remain compatible.
Mining priority defaults to the existing manual order. Automatic priority uses known reward
dates; it does not predict whether enough watch time remains. Completed watch time is never
treated as a successful claim. Mock tests and image health checks do not prove live Twitch earning.

## [v1.2.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.2.0) — 2026-09-29

- standardize dashboard icons with the MDI React package and borderless icon actions, including campaign/history sorting, filtering, mining controls and game priorities ([#35](https://github.com/ohne-b/twitch-drops-miner/pull/35))
- move recorded claim history into Campaigns with shared search, game filters, sorting and layouts; retain older claims without catalog metadata and remove CSV/JSON exports
- simplify Twitch account status and device authorization with a copy-code action and an aligned Activate/Done row; move dashboard connection status into Connection settings
- include local claim history in the confirmed Clear All Cache action, remembering cleared reward IDs so refreshes and late claim receipts cannot restore them

Existing data, credentials, settings, service names and container mounts remain compatible.
History is preserved on upgrade; only the explicit Clear All Cache action deletes recorded
claims. Completed campaign records remain intact. Clipboard copying requires HTTPS or localhost.

## [v1.1.5](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.5) — 2026-09-28

- advance the Mining card using current Twitch reward evidence and stop selecting rewards whose confirmed watch requirement is complete ([#33](https://github.com/ohne-b/twitch-drops-miner/pull/33))
- reconcile delayed completion and auto-claim evidence with bounded inventory refreshes, preserving account-issued claim IDs, claim retries and History imports
- reject stale reward and channel progress while preserving manual-channel timers, selected-game priority and the existing watch cadence

Existing settings, credentials, data and container configuration remain compatible.
Completed watch time is never treated as a successful claim. Offline regression tests
verify local transitions and races; they do not prove live Twitch earning.

## [v1.1.4](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.4) — 2026-09-28

- restore 15-second idle HTTP connection expiry so minute-spaced watch events do not reuse long-idle connections
- retry transient watch connection failures and HTTP 429/5xx responses promptly with bounded, cancellable backoff instead of immediately waiting for the next watch minute
- preserve the same watch payload across retries, stop after acknowledgement, and retain existing recovery when attempts are exhausted

Existing settings, credentials, data and container configuration remain compatible.
These changes restore transport behavior from the Python implementation; they do not
guarantee that Twitch credits watch time or establish the cause of every disconnect.

## [v1.1.3](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.3) — 2026-09-28

- preserve valid Twitch logins when public channel pages or settings scripts return HTTP 401/403
- resume stalled rewards after a successful inventory refresh by clearing stale estimates while preserving confirmed progress and newer account evidence

Existing settings, credentials, data and container configuration remain compatible.
Device-code login is unchanged. Recovery does not guarantee that Twitch credits watch time.

## [v1.1.2](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.2) — 2026-09-28

- discard failed watch beacon addresses and automatically renew Twitch connections after three consecutive watch failures ([#28](https://github.com/ohne-b/twitch-drops-miner/pull/28))
- import Twitch-confirmed rewards into History, including badges claimed outside the miner; label unknown claim times as first seen and keep cleared entries cleared
- preserve confirmed claims when inventory refresh overlaps progress updates, preventing awarded rewards from remaining eligible because of stale local state

Existing settings, credentials, data and container configuration remain compatible.
History imports require available campaign metadata; CSV exports add a final
`claimed_at_is_observed` column. Advanced diagnostics remain off by default.

## [v1.1.1](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.1) — 2026-09-28

- add opt-in automatic badge and emote mining across games, including required prerequisite drops
- keep selected games first, then prioritize other matching campaigns by soonest expiry; preserve manual watching, benefit filters and ignore rules

Both new options default off under Settings > Mining. Existing selections, settings,
credentials, data and container configuration remain compatible.

## [v1.1.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.1.0) — 2026-09-28

- add campaign sorting by Default, Newest, Ending Soonest, Most Drops and A-Z; move the campaign count into the heading and Clear filters inside Filters ([#24](https://github.com/ohne-b/twitch-drops-miner/pull/24))
- improve server-side diagnostics for upstream failures, invalid responses and incomplete catalog refreshes ([#23](https://github.com/ohne-b/twitch-drops-miner/pull/23))
- add opt-in advanced diagnostics with bounded, redacted JSON previews and nested transport causes; enable with `TDM_DIAGNOSTICS=true` or `--diagnostics`, independently of ordinary verbosity ([#25](https://github.com/ohne-b/twitch-drops-miner/pull/25))

Existing settings, credentials, progress and history remain compatible. Advanced diagnostics
are off by default and appear only in server logs, never in the dashboard.

## [v1.0.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v1.0.0) — 2026-09-27

- restore campaign discovery through the public SunkwiBOT catalog while keeping device-code login and private account inventory with Twitch ([#20](https://github.com/ohne-b/twitch-drops-miner/pull/20))
- show eligible selected-game channels and allow manual watching by channel name or URL, with an optional timer back to automatic mode ([#18](https://github.com/ohne-b/twitch-drops-miner/pull/18), [#19](https://github.com/ohne-b/twitch-drops-miner/pull/19))
- show inventory refresh progress, completion and retry inside the button; remove redundant preparation and settings-save messages ([#19](https://github.com/ohne-b/twitch-drops-miner/pull/19), [#20](https://github.com/ohne-b/twitch-drops-miner/pull/20), [#21](https://github.com/ohne-b/twitch-drops-miner/pull/21))
- use Twitch Drops Miner branding, the new logo and a concise README with a dashboard preview ([#15](https://github.com/ohne-b/twitch-drops-miner/pull/15), [#16](https://github.com/ohne-b/twitch-drops-miner/pull/16), [#18](https://github.com/ohne-b/twitch-drops-miner/pull/18))
- publish amd64/arm64 images exclusively at `ghcr.io/ohne-b/twitch-drops-miner`, retaining `latest.json` release notices without a dashboard updater ([#17](https://github.com/ohne-b/twitch-drops-miner/pull/17), [#18](https://github.com/ohne-b/twitch-drops-miner/pull/18))
- adopt PolyForm Strict 1.0.0 for the project, preserve upstream MIT and asset notices, and remove redundant guides and archived plans ([#21](https://github.com/ohne-b/twitch-drops-miner/pull/21))

Update the image name when upgrading from 0.1.0. Existing settings, credentials and history
remain compatible. Earlier published copies retain their original license terms.
The public catalog is a third-party metadata source whose coverage can vary; it receives
no Twitch credentials or account identifiers.

## [v0.1.0](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v0.1.0) — 2026-09-26

- introduce the standalone Rust backend with the React dashboard and original Twitch device-code login
- keep mining opt-in, order active campaigns with progress first, and retain completed campaigns in Finished
- preserve settings, mining selections, claim history and dashboard password protection across the migration
- add release availability notices in Settings > Maintenance and publish `latest.json` with every release
- publish multi-architecture images to GHCR, with optional Docker Hub publication from the same build
- remove Telegram, retire the previous backend and simplify repository setup and documentation

This starts the project's release sequence at 0.1.0. Builds carrying the inherited 1.3.2
development version need a manual upgrade to this release. One fresh Twitch device-code
login is required when migrating from the previous backend. Existing data and credentials
are preserved; updates are installed from the terminal.
