# Development measurements

## Automation acceptance

Run the repeatable editor and automation checks from the workspace root.

```sh
node tools/check-automation.mjs --offline
node tools/check-automation.mjs --offline --network
```

Omit `--offline` when Cargo needs to fetch locked dependencies. The first
command exercises the core, desktop editor, files and audio fixtures, the
save/reopen inventory of searchable nodes and conversions, and in-memory
network tests. It reports TCP coverage as skipped. The second also requires
IPv4 loopback and checks the real HTTP, WebSocket, MQTT and AMQP socket
fixtures. It exercises a saved editor graph against HTTP and WebSocket
sockets and verifies the HTTP status and body delivered to the editor.
A requested TCP pass exits nonzero when loopback is blocked.

| Check | Evidence |
| --- | --- |
| Searchable desktop nodes and conversions | Every listed kind creates one card, and saves and reopens with the same view. The headless desktop paints each searchable card and checks that English and Spanish labels are distinct within a category. Hidden compatibility definitions have their own migration tests. |
| Graph behavior | Focused assertions cover branches, validation failures, variable changes, editor result reporting, stop and reload. The shipped JSON example runs through the desktop with nested, missing and non-text values. |
| Connections | Deterministic fixtures check request encoding and responses. The TCP pass checks wire exchanges against local fixtures. |
| Deployment | A public HTTPS certificate check, dedicated test brokers and physical audio devices require separate runs in the target environment. |

To exercise the public HTTPS fixture separately:

```sh
cargo test --locked -p orange-net --features network-service-tests --test http a_real_https_endpoint_is_reached_with_certificate_verification -- --exact
```

The MQTT and RabbitMQ integration tests use the same network-service-tests feature. Configure a
dedicated test broker for `a_real_broker_delivers_what_the_graph_publishes`
or `real_rabbitmq_delivers_confirmed_publish`. Their setup and environment
variable names are documented with the tests. Catalog coverage is an
authoring and serialization check. Runtime checks prove the asserted
scenarios and do not certify every configuration of every node.

### Docker network lab

From the workspace root, start the loopback-only stack and run the saved
graph against it. The test first uses the saved profiles, then runs the
same graph with authenticated profiles and checks credential rejection.

```sh
docker compose -f tools/network-lab/compose.yaml up --build --wait
cargo test --locked -p orange-net --features network-service-tests --test docker_network -- --nocapture
```

Open [network-lab.json](../app/crates/apps/orange-cli/graphs/network-lab.json)
in OrangeEditor for a manual run. Start the graph, fire the buttons in the
six labeled groups and watch **Variables**. HTTP writes status 200, a JSON
body, four binary bytes, and two NDJSON lines. WebSocket echoes text and
bytes. MQTT returns the topic and text or binary payload. AMQP returns
publisher confirms and delivery records. Stop the editor run before running
the automated test so they do not compete for the same AMQP messages.

The saved graph uses a single anonymous HTTP profile for JSON, bytes and
streaming. WebSocket and MQTT are anonymous too. Its AMQP profile
uses OrangeEditor's built-in `guest` credentials because RabbitMQ
authenticates every connection. To run the protected cases in the editor,
stop the graph. Edit the existing HTTP profile and add the two additional
HTTP profiles in **Connections**, then select the corresponding profiles
on the request and stream line cards. Edit the other saved profiles in place.
All values below are synthetic and exist only for this loopback lab.

| Profile | Protected endpoint | Authentication |
| --- | --- | --- |
| HTTP | `http://127.0.0.1:18080/basic` | Basic, user `lab`, password `lab-http-only` |
| HTTP bytes | `http://127.0.0.1:18080/key` | API key, header `X-API-Key`, token `lab-bytes-only` |
| HTTP stream | `http://127.0.0.1:18080/bearer` | Bearer `lab-stream-only` |
| WebSocket | `ws://127.0.0.1:18081/auth/echo` | Bearer `lab-websocket-only` |
| MQTT | `mqtt://127.0.0.1:11884` | User `lab`, password `lab-mqtt-only` |
| AMQP | `amqp://127.0.0.1:15673`, virtual host `orange-auth` | User `lab`, password `lab-amqp-only` |

The live test resolves credentials via a test secret store and checks that
HTTP reports a wrong password as status 401 and that a WebSocket upgrade
without authorization also returns 401. The offline acceptance command
checks the graph and profiles plus the mock routes and stream encoding.
Use the public HTTPS and TLS-specific tests for transport encryption.

### Reading lab results

| What you need | Use these nodes |
| --- | --- |
| One HTTP result | **HTTP request** gives you text. Use **HTTP request (binary)** for exact bytes. |
| Results while an HTTP response arrives | **HTTP request (streamed)** starts the request and reports status and line count on completion. **On HTTP stream line** runs once per line. Set both cards to the same profile. For NDJSON, wire its payload to **Parse JSON** and **Get JSON path**. |
| WebSocket messages | **On WebSocket message** receives unsolicited messages on its own. Add **Send WebSocket message** if the graph should send a message too. Select the same profile to share one socket. The binary variants carry bytes. |
| UTF-8 text in a binary message | Use **Bytes to Text (UTF-8)** for strict decoding or **Bytes to Text (UTF-8 with replacement)** for a readable preview. The lab bytes `00 FF 80 0A` deliberately contain invalid UTF-8. Keep the original bytes for binary processing. |

HTTPS and WSS unwrap transport encryption before data reaches the graph.
Application-encrypted payloads require the sender's cipher, key, nonce
and authentication-tag format before text decoding or JSON parsing.

When finished, bring down this Compose project with
`docker compose -f tools/network-lab/compose.yaml down`.

## Performance

For complete-execution latency/allocation JSON, CI architecture coverage and the older
reference-machine procedure, see [CI and performance](../docs/10-development.md#checks).
Use separate feature-mode binaries for latency and allocation measurements.

The Rust `orange-measure` binary is development tooling. It is not a core dependency, and heavy measurement profiles are not invoked by ordinary workspace tests.

Build the exact locked source before timing:

```sh
cargo build --release -p orange-measure --locked --offline
cargo run --release -p orange-measure --locked --offline -- --warmup 5 --runs 15 index-update
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 authoring-compare
cargo run --release -p orange-measure --locked --offline -- --warmup 3 --runs 15 import-profile
cargo run --release -p orange-measure --locked --offline -- --warmup 2 --runs 7 ownership-memory
```

Offline mode requires cached dependencies; otherwise fetch the locked dependencies first. Run comparisons serially after compilation, using at least three independent paired processes. Set `ORANGE_MEASURE_OWNER_FIRST=1` for the second authoring process to reverse candidate order. `session-edit` and `w9` cover the wider edit matrix; W9 keeps its fixed seeded workload regardless of the generic run-count argument.

- `index-update`: root/function-local edits and wholesale function replacement/removal/restoration, including many-function and large-body cases. Transaction application and index update are timed; reset and fresh-build oracle checks are outside the timer.
- `authoring-compare`: equivalent snapshot-plus-session and document-owning workers on zero-diagnostic graphs. Caller work excludes explicit backpressure waits; newest/burst latency includes them. The profile verifies newest-request publication, history, missing/foreign receipts, registry/effects, cancellation and shutdown.
- `import-profile`: encoded inputs are built outside the window. Node/key/value-heavy shapes, wide extension arrays and node-quota refusal report allocator bytes separately from timing. It prints accepted/rejected outcomes so the same source works against the older importer.
- `ownership-memory`: document clone/drop, session construction/update/drop and 100 wide Fork races in one Run, with explicit stop/drain/dispose/drop stages.

For the pre-membership index comparator, create an owned worktree at the checkout containing this harness, restore only `app/crates/core/orange-model/src/index.rs` from `e9aead5` in that worktree, then build its binary and run the same `index-update` command. For pre-budget import, restore only `codec.rs`, `error.rs` and `lib.rs` from `6854560` in the isolated comparator. Never restore over unrelated work.

Method, measured results, selected ownership direction and limitations are in [the verification record](../docs/10-development.md#performance). These profiles do not declare UI, security-isolation or reference-machine NFR gates passed.

## Package archive vectors

`package-archive.mjs` is an independent Node encoder for the canonical `.orangeplug` archive and its content digest ([04](../docs/04-expansion-plugins.md), "Canonical archive"). `node tools/package-archive.mjs` prints the vector set; `package-archive.test.mjs` checks that it equals `app/crates/platform/orange-packages/tests/fixtures/archive-v1.json`, which the Rust reader's tests also reproduce. Regenerate the fixture only for an intentional format change.
