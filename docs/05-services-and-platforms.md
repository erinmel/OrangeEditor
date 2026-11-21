# Host services, integrations and platform capability

The [connection manager](#connection-manager) records the implemented network subset alongside the remaining service contracts. Exact dependency pins and verification evidence are recorded in [10](10-development.md). Capability detection returns supported, partial, unavailable or permission-required with a reason; unsupported OS functionality must not appear as a working node.

Service statuses, errors, remediation, connection/vault dialogs and audio controls follow [13](13-internationalization.md): stable codes and typed redacted message arguments, localized by the host. OS paths, protocol tokens, credential references, device names and remote payloads remain content/identity. The UI locale cannot change connection settings, routing, file encodings or graph parse/format semantics. OS-owned dialog language follows the recorded platform capability.

Scope: ADR-22 selects generic files, HTTP/HTTPS, WebSocket, configurable triggers, audio playback/input and level events. MQTT is conditional. Other protocol rows below are extension proposals, not foundation release requirements. Specialized website APIs, OBS/Twitch and browser-canvas servers are outside delivery.

## Triggers and input

A trigger is an entry/event-source node, independent of its physical source. The host routes manual UI/CLI commands, timers, keyboard hooks, audio analysis and installed package sources through the same typed, bounded, run-owned ingress (11 §10). Stop/revoke removes subscriptions and stale generations are rejected. Opening a document or configuring a source never starts it.

The manual node exposes an accessible on-node test button; it enqueues one activation on a selected running run and displays disabled/stopped status otherwise. The keyboard node opens binding configuration from the node: one key or simultaneous modifier/key combination, focused-application or explicitly granted global scope, trigger-on-press, key-repeat policy and rearm after release. Default ignores OS autorepeat; no timed key sequences are implied. Capture mode consumes the candidate binding without firing the graph. Conflicts/reserved combinations and unavailable global hooks are reported. Stable key codes and a declared physical/logical interpretation are stored separately from localized captions. Focused bindings do not intercept ordinary text/IME editing. A headless host needs an explicitly supplied input adapter; no window or global hook is initialized merely to load a graph.

Global registration, permission/session restrictions, conflict handling, release detection and teardown require per-OS evidence (P-07), not an inference from editor keyboard shortcuts. A plugin-defined trigger registers its payload schema and source lifetime through the normal event-source service; it cannot invoke a graph handler directly or inject arbitrary OS hooks into the core.

The desktop's On key pressed definition version 2 accepts a logical key name
through its `key` input. An empty input uses the node's configured key (F8 by
default), modifiers and repeat rules remain settings, and focused editing and
reserved editor shortcuts retain priority. Version 1 remains loadable. Global
registration requires a separate platform adapter.

## Files and paths

`FileService` exposes bounded read/write streams, text codecs, replace/append, list/filter/watch and native path picker handles. User-selected roots become grants bound to package/workspace, requested operations and resolved directory identity. Path text is not an authorization boundary: reject traversal, resolve symlinks/reparse points under platform semantics, and use directory-relative handles/no-follow primitives where available to avoid check-then-open races. If the adapter cannot enforce a restricted-root operation safely it refuses it. Paths are OS-native data internally; UI display may be lossy but stored paths must not assume every Unix filename is valid UTF-8. Graph portable paths are workspace-relative or user-remappable asset references. Case sensitivity, normalization, reserved names and rename semantics are tested per filesystem; no global lowercasing.

Read/write streams use proposed 64 KiB chunks and bounded in-flight buffers, with explicit UTF-8/default encoding, newline/BOM policy and decode errors. Large file nodes stream; whole-file convenience nodes have a visible size limit. Atomic replacement writes a sibling temporary file, completes encoding/write, optionally syncs, then uses the platform replace primitive; cross-volume move is refused as atomic or exposed as a different copy operation. Atomic visibility does not imply crash durability; durable mode includes file and directory synchronization where the platform supports it, and reports the supported guarantee. Sharing violations, disk full, permissions and partial writes are typed failures; append is not an application-level multi-writer transaction. Watches may coalesce/drop events and require rescan after overflow; they are not a durable change log. Cancel prevents future chunks and closes owned handles; a blocking syscall may finish after logical cancellation.

Graph **directory bundle** saves use immutable generation directories plus a small current-generation manifest/pointer committed with the tested per-OS atomic file replacement primitive. Never assume replacing a populated directory is atomic. Keep the previous generation until recovery validation succeeds; enumerate orphan generations safely after restart. A single-file archive uses the same sibling-temp/replace rule. WU-05 tests power-loss simulations at each commit stage and documents actual filesystem guarantees.

## Connection manager

The Connections panel offers one HTTP / HTTPS protocol. The base URL chooses
the scheme and may include a path prefix. Put a query in the node's request
path. WebSocket also accepts either WS or WSS. HTTP and WebSocket offer None,
Basic, Bearer and API key authentication. HTTP also offers OAuth client
credentials and refresh token. MQTT offers None and Username and password,
with an optional password. AMQP offers Username and password or the broker's
default guest credentials. The API key is stored as a credential reference in
`api_key.token`, and `api_key.header` defaults to `X-API-Key`. Standard and
custom HTTP profile headers are sent to the configured origin. Request nodes
return a redirect as an HTTP status, so a graph must choose explicitly whether
to request a different address.

Check settings validates and opens a profile without firing a Request node.
Create and edit network profiles in Connections. The node's dropdown lists
saved profiles for its protocol and changes the node reference in one undoable
edit. The editor creates graph-owned profiles. Host-provided shared
profiles are configured by the embedding host or CLI. Opening or updating a
flow acquires profiles named by its nodes, including nodes in function bodies.
Unused declarations can remain incomplete. Adding a node prefers a configured
profile of the correct protocol over a placeholder.

The text HTTP Request uses method, relative path, body (`content`), content
type and optional request `headers` (one `Name: value` line per header).
Every input remains wireable. The body is sent as UTF-8 without reformatting,
including when it contains JSON. The node returns status, success (2xx), and
response text. The failed route is for an invalid request, transport failure,
or unreadable response. Existing saved version 1 nodes are upgraded to
version 2 with the same ports, literals and wires, and empty defaults for the
new inputs. Binary and streamed request variants retain their payload types.

HTTP Request and HTTP JSON Request accept optional wireable text maps for
`header_fields` and `query`. Query names and values are UTF-8 percent-encoded
and appended to any query supplied in the path. Requests accept at most 64 query
pairs, a 16 KiB relative request target, and 32 additional headers bounded to
32 KiB including the node's content type. Header names are case-insensitive.
Graph headers cannot override profile headers or set authorization or HTTP
framing headers. Response header names are lowercase and each maps to a list
of values in response order. Invalid UTF-8 in header values is replaced.

HTTP JSON Request defaults to POST with a `core/json@1` body and
`application/json` content type. It returns status, success, response text and
headers, plus an optional JSON value. Non-JSON or empty response text produces
None, and the JSON text `null` produces a present value. Any HTTP status can
carry a JSON body, including error responses. Invalid requests, transport
errors and unreadable response bodies use Failed.

Credentials, authorization and binary payloads (2026-11-12; `orange-sdk`,
`orange-vault`, `orange-host`, `orange-net`). A setting value that is exactly
`credential://<name>` (name: 1 to 128 of `a-z 0-9. _ - /`) is a reference. When the
host opens a connection it replaces each reference, at any depth of the settings, with
the secret from `HostOptions::secrets` (`SecretStore`) and hands the result to the
adapter only; the document, the shared-connection table, events and errors keep the
reference. A missing secret, or a reference with no store configured, refuses the
connection by name. `orange-vault` provides `OsVault` (keyring 3.6.3: Windows
Credential Manager, macOS Keychain and, on Linux, the desktop's Secret Service over
D-Bus, which keeps secrets across restarts; a machine without a Secret Service reports
the store as unavailable rather than keeping secrets somewhere weaker)
and `MemoryVault`; the `orange secret set|remove|check` commands manage `OsVault`
entries and read the secret from standard input. An editor sets secrets up without that command
(2026-11-16): adapters describe their settings (`ConnectionAdapter::about`,
`texts`, `toggles`, with `choices` and `fields`), and a text setting marked `secret`
is typed masked; `Host::declare_connection_with_secrets` stores it with
`SecretStore::replace` under `connection/<connection ID>/<setting>` before the
declaration is committed and writes only the reference, so nothing is declared when the
store refuses. Undoing the declaration leaves the stored secret; it is replaced when
the same setting is typed again. `Host::test_connection` resolves the references and
opens the adapter once and releases it. Adapters that connect on first use may only
validate settings. The editor shows a host-owned error if the adapter refuses to open
so that an adapter's error text cannot expose a credential. HTTP and WebSocket connections take
`bearer` (a token), `basic` (`{user, password}`) and `headers` (an object); the HTTP
adapter adds them to every request and the WebSocket adapter to its opening handshake.
`org.orange.net/http-request-bytes` sends a `core/bytes@1` body with a content type
and a `headers` list of `Name: value` lines for that request, and returns status,
response content type and response bytes;
`org.orange.net/ws-send-bytes` and the entry `org.orange.net/ws-binary` carry binary
WebSocket messages. A graph can therefore exchange protobuf, gRPC-Web frames or a
payload it encrypted itself. Not provided: HTTP/2, so native gRPC is not reachable;
streaming of request bodies; client certificates; secret rotation notices to open
connections (a changed secret is used by connections opened afterwards).

OAuth and streamed responses (2026-11-12; `orange-net`). An HTTP connection's `oauth`
setting is `{token_url, client_id, client_secret}` for the client-credentials grant
or `{token_url, client_id, refresh_token}` (with an optional `client_secret`) for the
refresh grant, plus optional `scope` and `client_auth` (`body`, the default, or
`basic`). The token endpoint must be `https://`, or `http://` on this machine only.
Nothing is requested when the connection opens. The first request fetches an access
token; it is reused until 30 seconds before `expires_in` runs out; a request answered
with 401 fetches a new token and is sent once more. A connection set up with a refresh
token follows the server's rotation of it; when the token came from the credential
store (`"refresh_token": "credential://<name>"`) the replacement is also stored under
that name, so the sign-in survives a restart. The adapter never sees the store or the
name: the host gives each connection it opens a `Renew`, the adapter reports the
setting whose secret changed, and the host writes it through `SecretStore::replace`,
which a read-only store refuses. A connection using client credentials never changes
grant. A token endpoint that refuses takes the node's failed
route with `org.orange.net/Authorization`; errors repeat the status and the
standard's error code, never a secret or token. `orange oauth login <settings.json>
<name>` runs the authorization-code flow with PKCE (S256): it listens on
`127.0.0.1:<redirect_port>`, prints the address to open, accepts only the return that
carries its own `state`, exchanges the code and stores the refresh token in the
credential store under `<name>`. `org.orange.net/http-stream` sends a request like
the binary node and reads the response as it arrives: each non-empty line (at most
1 MiB) fires the connection's `http-stream-line` entries, and the node continues at
the end with `status` and `lines`. A flow that is behind slows the reading rather
than losing lines. `stream_timeout_ms` (default 600,000) bounds one response. Not
provided: OAuth for WebSocket and MQTT connections; the device-code and password grants;
OpenID Connect discovery; cancelling a stream that sends nothing (it ends at the next
line or at the timeout); streaming of request bodies.

MQTT (2026-11-12; `orange-net` qualified for 3.1.1). Adapter
`org.orange.net/mqtt` with settings `url` (`mqtt://host[:port]`, default port 1883, or
`mqtts://` for TLS on 8883 with the bundled roots), `username`, `password`,
`client_id` (generated when absent), `topics` (up to 64 filters, `+` and `#`
allowed), `qos` (0, 1 or 2, default 0), `keep_alive_s` (5 to 3,600, default 30),
`ca_file` (a PEM file of certificate authorities, which then replace the bundled ones:
for a broker with a private authority), `persistent` (the broker keeps the session's
subscriptions and missed messages between connections; needs `client_id`) and `will`
(`topic`, `message`, `retain`: published by the broker when the connection is lost
without a disconnect).
Nodes `mqtt-publish` (`topic`, `text`, `retain`) and `mqtt-publish-bytes` (`topic`,
`data`, `retain`) queue a message, at most 256 waiting; entries `mqtt-message`
(payload `[topic, text]`, fired for UTF-8 payloads) and `mqtt-binary` (payload bytes,
fired for every message). The client is this crate's own: one thread per connection
that connects when a flow has attached its triggers, subscribes, pings at half the
keep-alive, treats twice the keep-alive without a packet as a lost connection,
reconnects with backoff from 250 ms to 5 s and subscribes again. With `qos` 1 a
published message is kept until the broker acknowledges it and is sent again with the
duplicate flag after a reconnect, so a subscriber can receive it twice; received
messages are acknowledged after they are handed to the entries. With `qos` 2 the
four-step exchange is completed in both directions: an unfinished message keeps its
packet ID across a reconnect, and a received one is handed to the entries once however
often the broker repeats it. After a reconnect the session subscribes first and then
repeats what was unfinished. When the broker reports no kept session, exchanges it had
already confirmed are dropped. A closing session waits up to 500 ms for its disconnect
to be sent, so a normal stop does not publish the will. Not provided: MQTT 5,
client certificates, WebSocket transport, a per-node topic filter (subscriptions belong
to the connection), and messages over 8 MiB.

Files (2026-11-12; `orange-files`, package `org.orange.files`). A
`org.orange.files/folder` connection has settings `path` (an existing directory),
`read_only` and `poll_ms` (50 to 60,000, default 500). Nodes: `read-text`,
`write-text`, `append-text`, `read-bytes`, `write-bytes`, `list`, `exists`, `delete`,
`move`, `copy`, `make-directory`, `remove-directory` (empty directories only),
`size` (output `bytes`), `read-range` (`offset`, `length`; fewer bytes at the end of
the file, none past it), `append-bytes`
and the entry `on-change` (payload: the changed file's path). Paths are relative to
the folder with `/` between plain names; `..`, absolute and drive forms, device names
and links leading out of the folder are refused. A write goes to a temporary file in
the same directory, is flushed and renamed over the target. A read or write of a
whole file, and one piece read or appended, is at most 64 MiB; a larger file is walked
with `size`, `read-range` and `append-bytes`, and `copy` and `move` have no size limit
because they never hold the content. Listings, recursive operations and watcher snapshots
are bounded to 10,000 descendants. `copy-directory` also caps copied bytes at 1 GiB.
`copy-directory` and `move-directory` require a new destination under an existing
parent, and a move stays on the same filesystem. `delete-directory` cannot remove
the connected root. Tree mutations refuse links and reparse points and tree listings
skip them. Concurrent external mutation can interrupt an operation.

`list` accepts a relative `path`, an optional `pattern` and a `recursive` switch.
Patterns are case-sensitive. `*` and `?` match within a path component, and a
standalone `**` matches zero or more components. A pattern without a slash matches
the basename. `file-info` reports `is_directory`, `bytes` (zero for a directory),
optional `modified_unix_ms` and `read_only`.

`on-change` accepts a wired `folder` relative to the connection. An empty
input watches the connected folder. Its payload is an `org.orange.files/change@1`
record: `kind` (`created`, `changed`, `deleted` or `renamed`), `path`,
`previous_path` (empty unless renamed) and `is_directory`. Polling starts from a
silent baseline and coalesces changes between polls. Renames are inferred from
file identity on Unix and matching metadata on Windows. Ambiguous matches are
reported as deletion and creation. Oversized snapshots report a source fault
and recovery takes a new silent baseline. These are bounded polling snapshots.

Implemented subset (2026-11-10; `orange-host`, `orange-net`): connection declarations
(11 §16.1), graph and shared scope, and adapter-owned sessions (02).
The `org.orange.net/http` adapter takes settings `base_url` (required) and `timeout_ms` (whole
request, default 30,000). Its node `org.orange.net/http-request` takes `method`,
`path` and `content` and returns `status` and `response`; any HTTP status is a result,
while a transport failure, an invalid request or an unreadable or over-8-MiB response
takes the node's `failed` route with an `org.orange.net/*` code. `http://` and
`https://` endpoints work. TLS is rustls with the ring provider; certificates and host
names are verified against the bundled Mozilla roots (`webpki-roots`), not the system
trust store, and verification cannot be turned off. Pinned CAs, mTLS and system trust
are not implemented. The HTTP base must be absolute, with an explicit host and
without URL user information, a query or a fragment. Put the query on the
individual request path. A WebSocket URL may include its resource query.

Answers and failures (2026-11-15): the request nodes also return `success`,
true for 200 to 299, so a graph branches on it while a 404 or 500 still arrives as
`status` and `response`; `failed` stays for a request that could not be completed.
`method` defaults to `GET` and `path`, `content` (and on the binary and streamed
nodes `content_type` and `headers`) to empty, so a GET with nothing typed validates
and sends no body.

WebSocket (2026-11-10): adapter `org.orange.net/websocket`, setting `url` (`ws://` or
`wss://`, same TLS stack). A session is one socket with its own I/O thread, started
when the connection is opened; it connects in the background and reconnects with
backoff from 250 ms to 5 s. `org.orange.net/ws-send` queues one text message (input
`text`) and completes at once: `then` means queued, not delivered. Up to 256 text
or binary messages and 16 MiB of payload wait while the socket is down or busy.
The limits include the head being written. The worker checks incoming traffic
between batches, bounds socket writes to two seconds each and flushes its close
reply when the server initiates shutdown. A failed write can be retried after
reconnection, so delivery needs an application-level receipt when duplicates
matter. Sending beyond the limits fails with `org.orange.net/Busy`. The entry `org.orange.net/ws-message`
fires once per received text message with the text as `payload`, for every diagram
listening on that session, so diagrams sharing a connection share one socket and all
hear each message. A diagram whose mailbox is full misses that message. Binary
messages are ignored; ping/pong is answered by the library. Not implemented: binary
payloads, subprotocols and headers, connection-state events (opened, closed, failed),
per-message acknowledgement, and messages that arrived before a diagram started
listening. State (2026-11-15): the entries `ws-connected` (payload the URL) and
`ws-disconnected` (payload why: closed by the server, connection lost) fire each time
the socket opens or an open socket stops; a failed attempt to connect fires neither.
`ws-status` reads `connected` and `queued`, the messages waiting. The sends' `then` is
shown as "Queued" (definition hint `then_label`, 12 §4). Nothing else in this section is
implemented: no credentials or vault, profiles and revisions, leases, pooling policy,
retries, redirects policy, proxy or DNS rules, or Test Connection. The text
below remains a proposal.

Profiles store versioned adapter ID/schema, endpoint, typed options, TLS/auth policy, credential references and retry/deadline policy. App scope is per-user storage; workspace scope is separately authorized and can shadow an app display name but never its stable ID. Exports omit credentials, auth headers and secret-bearing query/options, preserving remappable placeholders; opaque extension options declare sensitivity. Import does not activate sessions or grant network authority.

`acquire(profile_id, revision, purpose, owner, grant)` returns a lease. Pool key includes resolved profile revision, adapter/provider version, authentication identity/scopes, TLS policy and isolation policy; credentials themselves never become stringified keys/logs. HTTP clients may pool requests; WebSocket subscriptions and optional MQTT client sessions have protocol-specific sharing restrictions. Later protocol plugins must declare their own sharing rules. Profile edits create a new version: active leases retain prior configuration unless revocation forces closure. Last lease releases idle transports after a bounded policy timeout; subscriptions/listeners live under explicit owners.

Test Connection validates schema and performs a bounded handshake/read-only health check supported by the adapter; it discloses when contacting a server/authenticating has observable effects. No sample publish/write/mutation is silently used as a connection test. Status includes DNS, transport, TLS, auth, negotiated protocol and safe diagnostics.

Defaults: verified certificates/hostname, system trust unless a profile pins an approved CA, explicit mTLS credential references, finite connect/read/overall deadlines, reconnect with exponential backoff+jitter and a ceiling. No universal retry on uncertain side effects. Redirects never forward credentials to a new origin without an applicable grant; host header/URL/query logging is sanitized. Endpoint/destination authorization is checked on each request/redirect/session recreation, including private/local targets if a grant restricts them. Proxy policy and DNS/IP restrictions are explicit profile fields, not hidden environment-derived permissions. TLS bypass is an exceptional profile option with visible persistent diagnostics and cannot become the default.

## Vault and secret broker

| OS | Selected adapter and limits | Required verification |
|---|---|---|
| Windows | Credential Manager generic credentials via WinCred under current user/logon session; no administrator requirement for ordinary storage | Create/read/update/delete; no-logon-session behavior; local/session persistence and size-limit handling |
| macOS | Keychain Services, access controls and secure host dialog | Native SDK API/source review and signed-app prompt behavior; primary page was JS-only in this research |
| Linux | Secret Service-compatible user-session provider over D-Bus; service may be missing/locked | GNOME/KDE provider compatibility, unlock/cancel, missing session bus, headless failure; avoid assuming a draft spec equals deployed support |

No plaintext file fallback. Headless runs needing an unavailable interactive vault fail with `CredentialUnavailable` or use a separately configured OS-protected service identity/provider; no hidden desktop prompt. Secure host input masks secret entry, excludes history/telemetry/clipboard by default and avoids exposing it to guest editor controls. Memory is minimized/zeroized where supported, but absolute removal of all library/OS copies is not claimed. Vault deletion/rotation invalidates new broker uses and closes dependent sessions according to policy. Rotation updates an opaque credential revision; operations either use one complete revision or fail, never combine partial token pairs.

Broker imports: `authenticated_request(profile, operation, body)`, `credential_metadata(ref)` and registered `auth_transform(provider, ref, challenge, destination)`. Normal plugins receive success/results, not raw secrets. Auth transforms are versioned, allowlisted host-side implementations whose output is bound to a destination/session; they are not arbitrary untrusted guest algorithms with secret access. Generic bearer/basic/mTLS transforms use namespaced provider IDs in `auth.kind`. Specialized secret transforms are not bundled merely because a future plugin might need them; they follow the reviewed-host-primitive or exceptional raw-secret policy below. Generic WebSocket/core contains no website-specific authentication rule.

An expansion requiring a new secret algorithm has two choices: contribute a host-reviewed transform in a future host release, or request exceptional `credential.read` authority for a specific reference/purpose. The latter explicitly gives that guest the secret and increases exfiltration risk; it is not described as no-secret access. Workers have full native-user trust regardless of manifest promises. Credential providers for isolated guests use write-only secure host input and broker-owned opaque handles; an explicitly trusted external vault worker is a different trust tier. Graph inspection, logs, traces, exports, crash reports and test snapshots apply structured sensitivity fields; never rely only on matching a known token string.

OAuth/browser/device authorization is an optional later broker capability, not a foundation integration requirement. If adopted, it must own concurrent refresh, atomic token rotation, revocation and cancellation; plugins supply provider metadata and public clients never embed confidential secrets. Baseline credential references and generic authentication remain in scope.

## Protocol/API packages

Each request carries owner/generation/deadline, typed payload/stream and a checked connection lease. Results separate transport, authentication, protocol status, application error and cancellation. Payload/chunk/decompression limits are finite; stream queues apply backpressure. Initial defaults are proposals: 1 MiB ordinary message, 8 MiB whole-response convenience, streaming for larger data, 30 s request deadline with explicit long-stream settings. Protocol package defaults can be more restrictive. No common node claims identical delivery semantics across transports.

| Package | Scope and protocol-specific behavior |
|---|---|
| HTTP/HTTPS | HTTP/1.1 and HTTP/2 via compatible client; methods/headers/query/multipart/JSON/bytes/streams; retain status, headers and body separately. Automatic retry only with explicit safe/idempotent/replayable-body rule; 429/Retry-After observed. TLS and redirect policy from profile. HTTP/3 is later. |
| WebSocket | RFC6455 text/binary, fragmentation/message limits, ping/pong/close, subprotocols and bounded receiver. One stream consumer owner with explicit fan-out subscriptions. Reconnect opens a new session; delivery/replay of lost messages is application policy. |
| MQTT | **Conditional.** Select 5.0 or 3.1.1; client ID, clean start/session expiry, topic filters, QoS 0/1/2, retain and last will. Preserve retained/duplicate/QoS metadata; protocol QoS does not make downstream file writes exactly once. Limit inflight publishes and account for broker/session reconnect state. |
| AMQP | The built-in adapter implements AMQP 0-9-1, using existing broker topology, publisher confirms and manual consumer acknowledgements after host mailbox admission. Processing-success acknowledgement remains a separate capability. AMQP 1.0 requires a different adapter. |
| REST | **Outside foundation delivery.** HTTP-oriented convenience nodes for resource paths, status mapping, pagination and schema decoding. No separate transport or universal service schema. |
| GraphQL | **Outside foundation delivery.** Query/mutation operationName/variables over HTTP; retain GraphQL `data` and `errors` together, separate from HTTP failure. Subscription uses explicitly negotiated **graphql-transport-ws**; legacy `graphql-ws` subprotocol is not silently substituted. Schema import/introspection optional; credentials and introspection permission explicit. |
| gRPC | **Outside foundation delivery.** HTTP/2 plus protobuf `FileDescriptorSet`/import resolution; select fully qualified service/method and reflect dynamic messages. Unary/client-stream/server-stream/bidirectional have separate node contracts and bounded stream handles. Preserve status/details/trailers/metadata/deadlines; cancel each direction and drain half-close correctly. Reflection is optional and server-dependent; descriptor import works without it. gRPC-Web and arbitrary custom codecs are later. |
| Webhooks | **Optional plugin.** Outbound HTTP node templates. Inbound explicit listener with default loopback bind, chosen port, route/method, finite body/deadline/concurrency limits and provider-specific bearer/HMAC signature verification, replay window/dedup policy. Public bind/TLS requires explicit configuration; no automatic firewall/NAT/tunnel changes. A response token is completed once before timeout; stop closes owned listeners and outstanding responses safely. |

AMQP 1.0 is a different separately versioned optional adapter, not a baseline commitment. The broker matrix must name versions and test features; shared terminology such as queue/ack does not prove wire compatibility. Long-running gRPC/GraphQL streams and MQTT subscriptions use the runtime's bounded event policy, with transport-specific recovery; loss is reported rather than fabricated catch-up.

### AMQP 0-9-1 and wired MQTT filters

`org.orange.net/amqp` accepts `amqp://` (5672) or `amqps://` (5671). Settings
include a URL or separate virtual host, username and a credential-backed password,
exchange, queue, routing key, optional queue binding keys, prefetch (1–256),
heartbeat (5–300 seconds) and an optional custom `ca_file`. Broker topology is
provisioned separately, and setting binding keys requests the corresponding bindings.

`amqp-publish` and `amqp-publish-bytes` take a wireable routing key with a profile
fallback. The publisher has a 16-item queue, accepts payloads up to 8 MiB and waits
up to 15 seconds for a confirm. A return, NACK or connection loss is a typed failure.
An ambiguous in-flight publish is released to the caller as a failure for an
explicit retry decision. `amqp-message` and `amqp-binary` take a queue input with
a profile fallback. They deliver exchange, routing key, redelivery status and
text or bytes in typed records. Each armed consumer uses one worker and reconnects
with bounded backoff. The consumer acknowledges on admission to the host
mailbox. Busy or closed admission requests broker requeue, and invalid UTF-8 on
the text entry rejects without requeue.

MQTT message entry version 2 accepts a wireable topic filter. Empty input uses
the connection's configured filters, and version 1 stays available to saved
graphs. QoS 2 is selectable in the profile settings.

## Audio architecture

Implemented part (2026-11-11; `orange-audio`, package `org.orange.audio`). Connections:
`org.orange.audio/output` (setting `device`: the device's endpoint identifier or name,
`null` for the built-in silent device, absent for the system default) and `org.orange.audio/input` (`device`,
`threshold_db` default -30, `release_db` default 6 dB below it, `window_ms` 5 to 1,000,
default 50, `hold_ms` 0 to 60,000, default 0: how long the level must stay at the
threshold or above, in whole measurements, before the entry fires). Nodes: `play` (inputs `path`, `gain` 0 to 8) reads a WAV file with
integer or float PCM, converts it to the device's rate and channels and continues when
the sound has ended or was stopped; `stop` ends every sound on the connection;
`input-level` returns the latest RMS level in dBFS (silence is -120);
`on-level` is an entry fired once when the level reaches the threshold and again only
after it fell to the release level, with the level as payload. An output opens at first
use, so a document opens without the device and only the playing node takes `failed`.
One output mixes at most 32 sounds; a file over 64 MiB or 600 s is refused.

The device callback runs only the mixer: it takes commands from a bounded channel
without waiting, adds prepared samples, limits to full scale and hands ended sounds
back on a second bounded channel so their memory is freed on a control thread. It does
not allocate, lock, log or read files. The capture callback only sums squares and
steps the threshold; a session thread turns crossings into entry firings.

Playback controls and formats (2026-11-12): `play`, `loop` and file-backed audio
sources read WAV, FLAC, MP3, Ogg Vorbis and M4A (AAC-LC or ALAC). Files are
identified by content. MP3, Ogg and M4A use Symphonia 0.6.1.
`loop` starts the sound again each time it ends and continues
only when stopped. `pause`, `resume`, `seek` (input `seconds`, 0 to 600) and
`set-volume` (input `gain`, 0 to 8) act on every sound of their output connection;
sounds that need separate control play on separate connections. A paused output is
silent and holds each sound's position; a seek past a sound's end ends it and wraps a
repeating one; the volume multiplies each sound's own gain. A playing node waits
however long its sound is paused or repeats, and reports a stalled device when the
device has asked for no samples for two seconds. The callback rules are unchanged.

Folder playback (2026-11-13): `play-from` (inputs `folder`, `pick`, `gain`;
output `file`) plays the sound file a number chooses among the folder's WAV, FLAC, MP3,
Ogg and M4A files in name order, counting round; a counter makes a playlist and a seeded
random number a repeatable shuffle, so the node itself draws no randomness. A folder
without sound files is the fault `org.orange.audio/Empty`.

From the editor (2026-11-15): every audio node's definition names its
connection kind (`editor.connection_adapter`, 12 §4), so a node dropped onto the canvas
is bound to the document's first connection of that kind, or to a new graph connection
of that kind with no settings (the system default device) declared in the same edit.
Each adapter lists its `device` choices through `ConnectionAdapter::choices`: the
devices cpal reports for that direction now, then `null`. The list is read when the
editor first shows it and again on request, on the editor's thread, never on a flow's
or a device's thread; the properties panel of a node shows the list for the connection
it names and the Document tab's connection form for the adapter chosen. `gain`
defaults to 1 on `play`, `loop` and `play-from`, and `pick` to 0. A random file of a
folder is `play-from` with Random u64 (`org.orange.math/random-u64`, seeded by the
runtime's entropy rules, 11) wired into `pick`; the audio package draws no randomness
itself. Device lists were read on Windows 11 (WASAPI) only (10); macOS and Linux
listing, and devices that appear or disappear while the list is open, are not checked.

Devices that come and go (2026-11-15): devices are listed with cpal 0.18's
`DeviceId`, persisted as `<host>:<id>` (WASAPI endpoint ID, CoreAudio UID, ALSA PCM
name), and the choice list writes the identifier and shows the name; a setting that
is not an identifier, or names an endpoint that is gone, is matched by name, so older
documents still open. An output that could not be opened is not remembered: the next
node that plays opens it again; one whose stream reported a fatal error (device not
available, host unavailable, stream invalidated) or that stalled is dropped and
reopened at the next play. An input whose device is missing or fails is reopened
with backoff from 500 ms to 5 s, and `input-level` fails while it is down. Overruns
(`Xrun`), followed route changes and a refused real-time mode are not failures. The
error callback only stores a flag; the data callbacks are unchanged.

Setting up from the editor (2026-11-15): `play` and `loop` mark `path` as a
sound file and `play-from` marks `folder` as a folder (definition hint `paths`, 12 §4),
and the editor asks the application for the system's chooser. The input adapter lists
`threshold_db`, `release_db`, `hold_ms` and `window_ms` through
`ConnectionAdapter::fields`, and `ConnectionAdapter::probe` measures the chosen device
apart from every flow while the connection form's live level is switched on; the
form marks the threshold and release on the meter and lets the device go when it
closes. Identifiers were checked on Windows (WASAPI) only (10); CoreAudio and ALSA
identifiers, a device unplugged while playing, and silent rerouting of a default
output are not checked. Rerouting to another device is not done: only the same
setting is opened again.

Audio streams (2026-11-16, 22 §6): `audio-in` (Audio in) gives what an input
connection captures as a value of the package record type `org.orange.audio/stream@1`
(one field, `source: u64`), with the capture's `rate` and `channels`; `audio-out`
(Audio out, inputs `stream` and `gain`), `level` (Level, output `level` in dBFS) and
`record` (Record to file, inputs `stream`, `path` and `seconds`, output `recorded`)
take it. The value names a live input and holds no samples, so it is not a
`core/stream@1` (11 §12): sample-rate audio is not pulled through the graph item by
item, and package effects cannot take core stream inputs. Copying it fans out by
design; each consumer reads independently. It is refused (`org.orange.audio/Ended`)
once its input connection has closed, and a value of another shape is
`org.orange.audio/Input`.

Bounded buffers and policies: each opened input keeps one ring of one second of
interleaved `f32` samples at the device's own rate and channels, allocated when the
device opens and never grown. The capture callback is its only writer (atomic stores,
no lock, no allocation) and also meters, as before. Readers keep their own positions:
a reader more than half a ring behind skips to a quarter ring before the newest
sample (drop-oldest), and a reader that has caught up gets nothing. Audio out reads in
the output callback through a reader made on the control thread: it starts 30 ms
behind the capture, jumps back to 30 ms when more than 250 ms behind (clock drift, a
paused output), plays silence when caught up, converts the rate by linear
interpolation and maps channels as source `n` to output `n`, a mono source to every
channel, more output channels repeating the source's in order and extra source
channels left out. It ends when the output's sounds are stopped, the run cancels it or
its input closes; when the input device is reopened after a failure it follows the
new ring. Streams played on one output are mixed by the existing mixer and count
against `MAX_VOICES`; Jump to time does not move them. Record to file writes 16-bit PCM
WAV at the stream's rate and channels, `seconds` in (0, 3600], counted in captured
frames, on the node's worker thread; cancelling finalizes a valid shorter file, a
file that cannot be written is `org.orange.audio/Write`, and a device that stops
mid-way leaves a valid file and is `org.orange.audio/Device`. Audio in and Record wait
up to 3 s for the device to open. The silent input now captures timed silence (48 kHz,
two channels, 10 ms blocks), so streams work on machines without devices.

Endpoints: Audio in uses the input connection, so it takes any capture endpoint the
system lists (microphones, line inputs, virtual cables such as VB-Cable, which are
ordinary WASAPI endpoints). An input `device` of `loopback:` and an output's setting
(`loopback:` alone is the default output) captures what that output plays; it is
offered and listed ("<name> (what it plays)") on Windows only, through cpal 0.18.2's
WASAPI loopback (an input stream on a render endpoint in the output's default
format). cpal 0.18.2 also implements macOS loopback through a temporary aggregate
device; it is not verified, so not offered, and other systems refuse the setting with
a reason. Linux has no ALSA loopback in cpal; PulseAudio or PipeWire monitor sources
may appear as ordinary inputs, which is not checked. Windows evidence is in 10.

`entry on-level` is now labelled On level and `input-level` Input level, over any input
connection, loopback included; Seek is labelled Jump to time and its description says
it moves every sound on the output to the same time. `record` marks `path` with the
`save` path hint (12 §4), and the desktop editor opens the system's save dialog for it.

Not implemented from the design below: `AudioAsset` handles, control of one sound
among several on a connection (separate connections are separate players),
streaming from disk (a sound is decoded whole, up to 64 MiB and 600 s),
Opus and AAC, resampling better than linear interpolation,
events for devices that appear or disappear, and multi-device
synchronization. Crossings closer than
10 ms are reported as one.


Audio is a general host capability, not a DSP evaluation domain in the graph runtime. Basic operations are open/decode, play/pause/stop/seek/loop, gain/mute and bounded mixing/routing, plus selected-device capture and level events. TTS synthesis, transcription, sound recognition and a DSP/DAW editor are outside baseline. Existing virtual cables are selectable OS endpoints; creating virtual endpoints or drivers is outside baseline.

`AudioAsset` owns decoder/metadata or a streaming source descriptor. `PlaybackSession` owns selected output route, mixer controls, player instances and device leases. Nodes choose opened assets, folder lists/filters or explicit file references; random selection uses the graph seed and a recorded sorted file snapshot (filesystem iteration order is not a seed). Empty set, changed file and decode errors are observable. Playlist transition/loop/shuffle policy is explicit. Already-opened assets are handles with run/workspace ownership, not copies of file contents per node.

Proposed baseline decode formats: PCM WAV, FLAC, MP3 and Ogg Vorbis, contingent on decoder feature/license/fixture review. Additional codecs are reported through a capability query, not extension inference. Seek reports unsupported for nonseekable input and actual landed position for compressed formats. Volume, mute, balance/channel mapping and mixing have explicit gain/clipping rules (f32 internal mix; final limiter/clamp policy selected and tested). Per-device resampling/rate/channel conversion happens on preallocated processing stages; state preparation and decoder I/O are off callback threads.

Callback receives prepared sample blocks, mixes/copies from bounded rings and records lock-free counters. No allocation, file/network access, guest execution, logging, waiting lock or UI call. Underflow yields silence and a counter/event on a non-callback path; overload drops/rejects commands under explicit policy. Mixing may run native DSP on the callback only after bounded-cost validation. Network/plugin-generated audio arrives prebuffered; guest/worker jitter never controls the hardware callback clock. Separate output devices have independent clocks/drift; simultaneous multi-device playback is best effort unless a synchronization feature is explicitly implemented later.

Persist selected backend/device descriptor with stable OS ID where available plus human label and format; never persist enumeration index. On disappearance: stop/pause route and emit DeviceUnavailable; default **no silent rerouting**, especially for private/virtual output. User may enable fallback to system default per session. Hotplug enumerates again and offers explicit reconnect; seekable assets resume only at a visible policy boundary. Output route to a virtual cable is a render endpoint, not microphone capture access.

| Capability | Windows | Linux | macOS |
|---|---|---|---|
| Choose playback device per session | WASAPI-backed adapter candidate; hardware test required | PipeWire adapter preferred on PipeWire desktop; ALSA/Pulse compatibility fallback investigated | CoreAudio adapter candidate; hardware test required |
| Existing virtual audio endpoint | Enumerate/render to user-installed virtual cable endpoint | Render to existing sink / route through PipeWire graph | Render to user-installed virtual device |
| Capture one application's sound (`app:` input choices, 2026-11-18) | Built: apps with an audio session now are listed by executable name, and WASAPI process loopback (Windows 10 version 2004 or later) captures one process and its children at 48 kHz stereo; a choice whose process has ended is refused | Built: apps playing sound are listed from `pw-dump` and captured with `pw-record --target <serial>`; without PipeWire the choice is absent and the reason is shown | Not available: route the app to a virtual device such as BlackHole and pick that device; Core Audio process taps (macOS 14.2+, System Audio Recording permission) are the native route |
| App-owned virtual microphone/source | Outside baseline; driver deployment is separate | Outside baseline; a future PipeWire source adapter needs independent creation/capture/cleanup evidence | Outside baseline; native deployment is separate |
| Capture/output permission or session denial | Report unavailable/permission-required | Report unavailable/permission-required | Report unavailable/permission-required |
| Select audio input | Enumerate/open OS-exposed capture endpoints; verify permissions and device identity | Enumerate/open available capture sources; verify session policy | Enumerate/open available capture devices; verify native permissions |
| Headless audio | Requires available user/service audio session; no universal guarantee | Requires PipeWire/audio service and permissions; server-only environment may be unavailable | Requires audio service/session and configured output |

PipeWire documentation verifies virtual sources are possible and shows source/sink distinctions; it does not prove the proposed Rust adapter, current user's permissions, sandbox portal behavior or operation on every distro. Implement application-lifetime nodes through a client API/session adapter; never silently modify persistent system PipeWire configuration. Explicit profile may choose persistent administrative setup later. Windows SYSVAD documents driver installation, so Orange baseline does not pretend user-space endpoint creation. Apple primary driver/keychain pages were JS-only during retrieval; exact macOS supported versions/API/entitlements remain P1/P5 hardware/research gates.

## Capture, level analysis and plugin audio

An owned capture session binds an explicitly selected OS device, channel policy and negotiated format. Host acquisition checks permission; import/open never opens the microphone. Stop/revocation/device loss closes capture and invalidates handles. Device names and transient enumeration indexes are not stable identity. Default is no silent device fallback. Selecting an input is separate from loopback/system-output capture, which requires its own future capability contract. P-04 must verify input and output independently on each advertised OS/session.

Capture callbacks copy into preallocated bounded rings; overflow/gaps are counted and reported off callback. Analysis workers compute timestamped RMS and peak levels over declared windows and channels. Values use digital full-scale units (dBFS), not calibrated sound-pressure dB SPL. The contract must state normalization, silence representation, channel aggregation, window/hop and output cadence before implementation; these provider/version parameters are pinned for replay. No hard-coded algorithm or schema is claimed implemented here.

Threshold nodes configure above/below, level metric, threshold, optional hysteresis and a nonnegative continuous hold duration. Zero hold means the first qualifying analysis window, not zero physical latency. One event fires on qualification; it rearms only after the configured reset condition. A failing window, capture gap or restart resets hold state. Hold uses capture timestamps and monotonic time, never UI frames. Meter updates may coalesce; accepted trigger events use the bounded event policy. Deterministic tests use injected level/timestamp sequences and recorded audio; live microphone input is external ingestion, not seeded replay.

Frequency-band energy and fundamental-pitch estimation are separate conditional capabilities. Before adoption select and test window/rate, frequency resolution/range, latency, thresholds and confidence/unvoiced handling; do not call a loud harmonic the detected fundamental or promise arbitrary sound recognition. These choices need a bounded feasibility slice, not speculative DSP in the core.

A future TTS or other generator plugin can return a validated asset handle or feed a negotiated bounded PCM sink. The service defines sample format, backpressure, end-of-stream, underflow, cancellation and ownership; selected output device/gain are the same as file playback. Plugin work stays off hardware callbacks. This path must be exercised with a synthetic producer before public audio API freeze, without shipping a TTS integration.

### Sources and analyzers (2026-11-17; ADR-30)

An **audio source** connection (`org.orange.audio/input`) is a microphone or other input, what the system plays (loopback), or a **sound file**. It is opened once per flow and shared: the capture callback only stores samples in the source's ring and measures the whole-sound level. A file source plays into the same ring at real-time pace once Play source (or "Play at start") starts it, optionally also heard on an output ("Hear it"), so whatever listens to a microphone listens to a file the same way.

Listening is done by **entries and nodes with their own settings**, not by the connection: On band level (band, threshold, release, hold, window; replaces the first On level), On transient (band, sensitivity, floor, minimum gap; 45–120 Hz by default for kicks) and Band level (a band of a stream). Each analyzer reads the ring with its own cursor on its own thread. A band is a 2nd-order Butterworth high-pass at `low_hz` and low-pass at `high_hz` (12 dB per octave each), so neighbouring frequencies leak in at reduced level; a side left at 0 is open. On transient fires on a rise of the band's level of 3 to 12 dB (by sensitivity) above its recent energy average, above a floor and after a minimum gap, and re-arms only once the level falls back, so a held note is one hit. This is a band-energy onset detector, not spectral flux or pitch tracking.

Transport belongs to the file source: Play, Pause, Resume, Stop and Jump to time act on that source alone, and On finished fires at its end. Pause sounds, Resume sounds, Jump to time and Set volume still act on every sound of an output connection.

The live Audio source node publishes the existing `org.orange.audio/stream@1`
record. Level and Band level take that stream and publish `f64` dBFS values.
Audio format returns its sample rate and channel count. Transient and Source
finished take the stream as a root argument. Source control applies transport to
that file source, and Sound control applies transport and gain to the chosen
output. Band trigger takes the stream directly and emits the measured dBFS on
each qualifying threshold crossing. The lower and upper frequency inputs default
to open boundaries. The entry includes threshold, reset level, hold time and
measurement time. Level, Band level and Band trigger share the capture ring and
perform analysis on provider workers. Older saved graphs retain their definitions.

Audio Out version 2 starts when its stream and its `enabled` input are available.
Enable defaults to true and can be wired to a Boolean value. Disabling it stops
its voice on the shared mixer. The `playing` output reports that the voice is
active. A stream, enable or gain change re-arms the source. Version 1 remains
available to saved graphs with execution wires.

Frequency Splitter uses one capture cursor and worker per node. Its chart uses 96
logarithmic peak-amplitude buckets from 20 Hz to 24 kHz in dBFS, measured over
4,096 Hann-windowed frames and published at most once per 50 ms. Buckets between
measured Fourier bins are interpolated for display. Versions 2 and 3
publishes a distinct audio stream for each selected band. A derived ring keeps
the input sample rate and channel count, with each channel filtered independently
by two cascaded band-pass sections, with a high-pass fallback when the upper
edge reaches Nyquist. These filters have finite roll-off.
The same spectrum worker runs the filters and fills the derived one-second rings.
At most 12 bands are active. Version 1 retains its 25 ms band levels and
Above or Below routes for saved graphs. Editing a version 1 card to add a band
upgrades it to version 3, retaining its existing routes and adding a filtered
stream to each existing band as well as the new one.
Ranges beginning outside the supported filter range of the negotiated sample
rate remain unavailable. Capture overruns and ring replacement clear affected
readings and reset filter and trigger state. A source gap closes the old
derived rings. Supported bands receive new rings when capture resumes.
Legacy Spectrum, Band Level and Band Trigger
remain available to saved graphs.

Audio Level Trigger measures full-stream RMS over 25 ms and publishes `level: f64`
in dBFS and `condition_met: bool`. It fires once when the configured above or
below condition holds for the requested time. The defaults are -35 dBFS,
150 ms hold and a 4 dB reset margin. Time advances with completed audio windows.
Missing audio clears the reading and qualification. Real silence can qualify
for a below rule. Both nodes analyze off the audio callback, and their graph
events have their own bounded ingress independent of chart updates.

Noise Gate (`org.orange.audio/noise-gate@1`) takes a live audio `stream` and
publishes a `gated_stream` of the same audio stream type, rate and channel count.
It measures smoothed power across the input channels and applies the same gain
to every channel. The gate opens above Threshold and stays open until the level
falls to Threshold minus Reset margin for the Hold time. Attack and Release set
the duration of the linear gain changes between muted and unity. The settings
default to -40 dBFS, 4 dB, 100 ms, 5 ms and 80 ms, respectively. Latency includes
the detector's 10 ms smoothing and attack. Closed gates keep publishing PCM
silence, so downstream playback and recorders retain their stream. The worker
owns a bounded one-second output ring and resets the envelope on lost samples
or a replaced source ring. It runs outside the audio callback.

Compressor, Limiter, Gain and Filter work the same way: each takes a live
`stream`, processes it on its own worker with the shared reading loop, and
publishes a stream of the same type, rate and channels (`compressed_stream`,
`limited_stream`, `adjusted_stream`, `filtered_stream`). Settings:

| Node | Settings and defaults |
|---|---|
| Compressor | Threshold -18 dBFS, Ratio 4 (1–20), Attack 10 ms, Release 120 ms, Makeup 0 dB. Reduction is (level − threshold) × (1 − 1/ratio), linked across channels |
| Limiter | Ceiling -1 dBFS, Release 50 ms. A peak over the ceiling lowers the gain at once |
| Gain | -60 to +24 dB, default 0 |
| Filter | Low-pass, high-pass or band-pass biquad; Cutoff 1 kHz (kept under 0.45 × rate), Q 0.707 |

Attack, Hold and Release are Duration inputs on processors that expose them.
Enter a duration such as `5ms`, wire a Duration directly, or wire a decimal
and select its unit in the conversion menu. The processor checks its bounds
when the stream starts or its inputs change. The processing runs on the existing
workers outside the audio callbacks.

### Application playback and virtual cables

On Linux, the source selector uses `pw-dump` to list active playback streams
and `pw-record` to feed the selected stream's raw stereo samples into the
capture ring. The identifier combines the PipeWire session and stream serial.
If the session changes or the stream ends, select the app's new stream. Capture
requests disable WirePlumber fallback, reconnection and moving so a missing
target cannot silently redirect the capture. Missing tools or a running
PipeWire server surface as a source-picker status.
[PipeWire pw-cat 1.6.11](https://docs.pipewire.org/page_man_pw-cat_1.html)
[WirePlumber linking policy](https://pipewire.pages.freedesktop.org/wireplumber/policies/linking.html)

Windows keeps WASAPI device and system loopback. Native app capture is pending a
verified safe Rust binding. The process-loopback API requires Windows build
20348 or later. The selector labels recognized VB-Cable and VoiceMeeter
endpoints so the app can be routed to their capture side.
[Microsoft process-loopback parameters](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)

macOS reports native app capture unavailable. Route the app through a virtual
endpoint such as BlackHole and select its capture device. A future native
adapter can use Core Audio process taps on macOS 14.2 or later with the
system-audio recording permission. ScreenCaptureKit has a separate screen
recording permission and capture lifecycle.
[Apple Core Audio taps](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps)
[Apple ScreenCaptureKit sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)

## Specialized integrations

No OBS, Twitch, website API or browser-canvas server is bundled or required by foundation gates. Such plugins may reuse approved HTTP/WebSocket, vault, event-source and audio services. Optional webhook plugins require a host-provided listener capability with explicit authority and response lifetime; outbound HTTP alone does not provide inbound listening. A new native/privileged primitive still needs a compatible host release or the documented trusted-worker tier.

## Primary evidence and limitations (retrieved 2026-10-20)

Historical research register, not dependency approval or current delivery scope. ADR-22 narrows scope; optional protocol/platform entries below remain background only. Capture, trigger and analysis APIs have not been selected or verified.

| Source | Version/coverage | Limit |
|---|---|---|
| [HTTP semantics](https://datatracker.ietf.org/doc/html/rfc9110) | RFC9110, method/idempotency/status semantics | No selected Rust client's compatibility proof |
| [WebSocket](https://datatracker.ietf.org/doc/html/rfc6455) | RFC6455, handshake/frames/close | Subprotocols require separate contracts |
| [MQTT](https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html) | OASIS MQTT5, QoS/session concepts | 3.1.1 adapter separately needs conformance fixtures |
| [RabbitMQ semantics](https://www.rabbitmq.com/docs/semantics) | Resolved RabbitMQ4.4, AMQP0-9-1 ack/transaction caveats | Moving broker docs; no universal ACID/exactly-once claim |
| [gRPC core concepts](https://grpc.io/docs/what-is-grpc/core-concepts/) | Streaming shapes/status/metadata/deadline lifecycle | HTTP/2/protobuf dependency APIs still need exact-pin review |
| [GraphQL WebSocket protocol](https://raw.githubusercontent.com/enisdenjo/graphql-ws/master/PROTOCOL.md) | graphql-transport-ws, IDs/multiplexing | Moving master, pin commit in fixture |
| [WinCred](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew) | Generic credential create/update/logon constraints | Complete adapter persistence/size policy not tested |
| [Secret Service](https://specifications.freedesktop.org/secret-service/latest/) | Resolved 0.2 DRAFT, 2026-10-19; collections/sessions/lock | Deployed Linux providers may implement older contract |
| [Keychain](https://developer.apple.com/documentation/security/keychain-services) | Official entry point | JS-only; detailed API evidence unavailable in this retrieval |
| [PipeWire loopback](https://pipewire.pages.freedesktop.org/pipewire/page_module_loopback.html) | Resolved 1.6.9; virtual sinks/sources/session manager | Does not verify app client adapter or distro policy |
| [Windows SYSVAD](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/) | Driver sample; endpoint installation requires driver deployment | No Orange driver planned; existing-endpoint tests remain |
| [Apple audio driver](https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in) | Official driver extension entry point | JS-only; do not infer completed deployment design |

Verification: V04/V07/V09/V10; ADR-09/10/11 in [08](08-decisions.md).
