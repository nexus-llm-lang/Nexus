# DNS rebinding mitigation for `host-http` (nexus-vbei)

## Threat model

`bootstrap/src/lib/nexus_host_bridge/src/url_guard.rs::is_blocked_host`
performs a *syntactic* SSRF screen: it parses the URL host, classifies IP
literals against private/loopback/link-local ranges, and rejects a small
allowlist of dangerous hostnames (`localhost`, `metadata.google.internal`,
`*.local`, …).

The hostname check happens in `parse_url` (`host_impl.rs:157`) before the
bridge calls `wasi:http/outgoing-handler::handle`. The actual DNS
resolution and TCP connect occur **inside** the WASI runtime when the
outgoing-handler future is driven. There is therefore an unavoidable TOCTOU
window between the syntactic check and the connect.

A malicious DNS server can exploit this:

1. Attacker controls `evil.example.com`.
2. Guest issues `GET http://evil.example.com/`. `is_blocked_host` sees a
   non-IP hostname not on the deny-list, returns `None`.
3. Bridge calls `outgoing_handler::handle`. The runtime resolves
   `evil.example.com`. The attacker's authoritative DNS returns a private
   IP — for example `169.254.169.254` (AWS/GCP IMDS) — with a low TTL.
4. Runtime opens TCP to the private IP. The bridge never sees that IP.

The classic refinement (DNS rebinding) returns a public IP first to pass
any pre-resolve check, then a private IP for the connect. Because the
WASI 0.2 `outgoing-handler` does not split resolve from connect, even an
extra `resolve-then-validate` pass before `handle` does **not** close the
window: the runtime will resolve again at connect time.

## Scope

* In scope: outbound `http://` and `https://` requests issued through
  `host-http-request` / `host-http-request-with-options` when the embedder
  policy allows network egress (`allow_net = true`).
* Out of scope:
  * IP-literal URLs — already pinned by the existing syntactic check.
  * The `host-http-listen` server side (no DNS involvement).
  * Response-content based filtering (different threat model — covers data
    exfil, not destination control).

## Mitigation options considered

### Option 1 — Resolve in the bridge, connect to the IP

Use `wasi:sockets/ip-name-lookup::resolve-addresses` to obtain the IP set,
validate every returned IP against `blocked_ipv4_reason` /
`blocked_ipv6_reason`, then perform the connect ourselves to a single
pinned IP.

This is the only option that actually closes the TOCTOU window: once the
bridge owns the resolved IP, no later DNS query can change the connect
target.

Sub-options for "perform the connect ourselves":

* **1a. Hand off the IP to `outgoing-handler` via authority rewriting.**
  Set `request.set_authority(Some("<ip>:port"))` and inject `Host:
  <original-hostname>` in the headers. Works for plain `http://`, but
  **breaks `https://`** because the runtime will negotiate TLS using the
  authority as the SNI / certificate-validation hostname — it will see an
  IP, fail SNI, fail certificate verification (no SAN match on a literal
  IP for normal certs). The threat model requires `https://` to keep
  working; otherwise legitimate guests are broken.
* **1b. Replace `outgoing-handler` with a custom HTTP/1.1 client built on
  `wasi:sockets/tcp` plus a TLS layer.** This is the only fully correct
  path. It is also a multi-week rewrite: the bridge would have to write
  request lines, parse responses, drive chunked-encoding, support HTTPS
  via a wasm-side TLS implementation (rustls in wasm, `mbedtls`, or
  similar), and re-implement the timeout semantics added in
  `nexus-upzz.7`. Continuation/lifetime invariants in `host_impl.rs`
  (`ConnEntry` / `SERVERS` / `CONNS`) would all need re-validation.

### Option 2 — Pin the resolved IP

A degenerate form of 1b: cache the resolved IP and reuse it for the single
connect inside the bridge. Requires a custom client (because
`outgoing-handler` does its own resolve). So this collapses into 1b.

### Option 3 — Restrict DNS at the host

Configure the wasmtime / WASI host's DNS resolver to refuse private-IP
responses. This is enforceable only **outside** the bridge (in the
embedder), is not portable across runtimes, and silently fails open for
embedders that don't apply the same configuration. Rejected as a primary
mitigation — may still be recommended as defence-in-depth in deployment
docs.

## Recommendation

Land Option 1 in **two phases**, gated by the readiness of a wasm-side TLS
story.

### Phase A (interim, narrow but cheap) — pre-resolve & validate

1. Add `import wasi:sockets/ip-name-lookup@0.2.6;` to
   `bootstrap/src/lib/nexus_host_bridge/wit/world.wit`.
2. In `host_impl.rs::parse_url` (after the syntactic `is_blocked_host`
   check passes for a non-literal hostname): call `resolve-addresses`,
   drain the stream, and reject the request if **any** returned IP is
   blocked by `blocked_ipv4_reason` / `blocked_ipv6_reason`.
3. Continue calling `outgoing-handler::handle` with the original hostname.

This does **not** close the TOCTOU window — the runtime re-resolves at
connect time. It does:

* catch the trivial case where DNS *consistently* returns a private IP
  (no rebinding required, just a hostile zone);
* catch slow/non-rebinding misconfigurations (split-horizon DNS leaking
  internal A records);
* shorten the window an attacker has, since they must serve a different
  IP between two queries the bridge issues nanoseconds apart.

A racing attacker who alternates A-record values per query still wins.
Document this honestly in the bridge module docs and in `url_guard.rs`'s
existing `Limitation:` note (do not delete the note — replace it with a
description of the residual window).

Cost estimate: ~1 day. Touches WIT (and therefore the wasm regen
pipeline), `host_impl.rs` (~30 LoC), tests on `url_guard.rs` (mock
resolver). No HTTPS/TLS work.

### Phase B (full mitigation) — custom HTTPS client on wasi:sockets

Replace `outgoing-handler::handle` for outbound requests with a
bridge-resident HTTP/1.1 client:

1. `resolve-addresses` → first non-blocked IP.
2. `create-tcp-socket` → `start-connect(net, ip:port)` → `finish-connect`.
3. For `https://`: wrap the input/output streams with a TLS client
   (rustls or equivalent compiled to wasm). SNI = original hostname,
   certificate verification = original hostname. This keeps HTTPS working
   while the connect target is the validated IP.
4. Write request line + canonical headers (re-use `headers_codec`),
   stream the body, parse status line + headers, drain the response body,
   honour `connect_timeout` / `first_byte_timeout` already plumbed in
   Phase A.
5. Re-validate `host-http-request-with-options` timeout semantics.
6. Mock-resolver tests: a public-looking hostname that resolves to a
   private IP must be rejected before the connect socket is even created.

Cost estimate: 1–2 weeks. Largest unknown is the TLS dependency —
see ["wasm TLS choice"](#wasm-tls-choice) below.

### Why not "do Phase B now in this PR"

* The 90-minute issue budget for nexus-vbei does not cover a custom HTTPS
  client.
* Picking the TLS crate is its own decision (size, maintenance, wasm
  build cost). Doing it under time pressure inside an SSRF fix would mix
  concerns and risk regressions in the timeout / streaming work landed in
  upzz.6 / upzz.7.
* Phase A delivers measurable improvement against the lazy attacker
  immediately and reduces the surface that Phase B has to worry about.

## Implementation sketch (Phase A)

Files touched:

* `bootstrap/src/lib/nexus_host_bridge/wit/world.wit`
  ```wit
  world bridge {
    // …
    import wasi:sockets/ip-name-lookup@0.2.6;
    // …
  }
  ```
* `bootstrap/src/lib/nexus_host_bridge/src/host_impl.rs`
  * `use bindings::wasi::sockets::ip_name_lookup::resolve_addresses;`
  * `use bindings::wasi::sockets::network::IpAddress;`
  * New helper `validate_resolved_ips(host: &str) -> Result<(), String>`
    that drains the resolve-address-stream, converts each `IpAddress`
    variant to `std::net::IpAddr`, and runs it through
    `url_guard::is_blocked_host_ip` (new helper — see below).
  * Call site: end of `parse_url`, only when `host.parse::<IpAddr>()`
    failed (i.e. it was a hostname).
* `bootstrap/src/lib/nexus_host_bridge/src/url_guard.rs`
  * Extract `blocked_ipv4_reason` / `blocked_ipv6_reason` calls into a
    public `is_blocked_host_ip(IpAddr) -> Option<&'static str>`. Keep
    `is_blocked_host(&str)` calling into it. Ensures the resolve path
    uses *exactly* the same range table as the syntactic path.
  * Update the module-level `Limitation:` note to read: "this and the
    resolve-validate companion close the static-deny case and the
    consistent-misconfig case but not the per-query-rebinding case;
    Phase B is tracked at <bd issue link>."
* `bootstrap/src/lib/nexus_host_bridge/src/url_guard.rs` tests:
  * `is_blocked_host_ip` matrix mirroring existing `is_blocked_host`
    cases (no regression on the syntactic path).
* New host-side test (Rust unit, not WASM, since the resolver is a WASI
  binding): inject a fake resolver via a thin trait the host_impl path
  takes by injection. Cover:
  * legit public hostname → all returned IPs public → pass
  * hostile hostname → returned IP is `169.254.169.254` → reject
  * mixed result (`8.8.8.8` and `127.0.0.1`) → reject (any-blocked → no)

The Rust-unit-test path is necessary because the Rust unit tests build
for the host target, where the wit_bindgen-generated `resolve-addresses`
is not available. Two acceptable shapes:

* **A. Trait-injected resolver.** Define
  `trait HostResolver { fn resolve(&self, name: &str) -> Result<Vec<IpAddr>, String>; }`
  in `url_guard.rs` (or a new `resolver.rs`). `host_impl.rs` uses a wasm
  impl that calls `resolve_addresses`; tests use a `MockResolver`.
* **B. IP-list validator only.** Keep the wasm-side resolver call
  inline; export only `validate_resolved_ip_list(&[IpAddr])` for unit
  testing. Smaller surface; loses ability to test the iteration logic in
  isolation.

Recommendation: A. Marginally more code, but the iteration / "any blocked
→ reject" logic is exactly the part most likely to regress, and a trait
boundary makes the wasm-only `resolve_addresses` call site small enough
to eyeball.

## Phase B notes

### wasm TLS choice

Top candidates as of 2026-04:

* **`rustls` + `webpki-roots`** — pure Rust, builds for `wasm32-wasip2`.
  Largest binary impact (~600 KB after LTO/strip). Most maintained.
* **`mbedtls`** — smaller, C, requires a `cc`/wasi-sdk toolchain in the
  build pipeline. Adds non-Rust deps to `bootstrap.sh`.
* **Wait for `wasi:tls` proposal** — exists in draft (Phase 1 in WASI WG
  as of 2026-Q1). Would obviate the TLS-in-wasm question. Not yet
  available in any wasmtime release that nexus targets. Tracking via
  upstream WASI repo only.

Recommendation: rustls when Phase B lands, unless `wasi:tls` reaches
phase 3 first.

### Compatibility risk

Switching off `outgoing-handler` means losing whatever the embedder host
configured for it (proxies, custom CA bundles, request inspection
hooks). Phase B should be gated on a config flag
(`nexus_host_bridge::Config::http_client = OutgoingHandler |
SocketsClient`) so embedders can opt in / out.

## Acceptance reconciliation

bd issue `nexus-vbei` lists, as acceptance:

> Either a design doc choosing an approach (and why the others were
> rejected), or an implementation landing option 1 with tests that
> exercise resolve-phase blocking.

This document is the design-doc deliverable. Phase A is the chosen
implementation path; Phase B is the full closure. Phase A should be filed
as a follow-up bd issue (`nexus-vbei.A`), Phase B as `nexus-vbei.B`. The
parent `nexus-vbei` stays open until Phase B lands, because the TOCTOU
window is not actually closed by Phase A — only narrowed.

## Honest residual risk after Phase A

* Per-query DNS rebinding still wins. An attacker whose authoritative
  server alternates A-record values between the bridge's resolve call and
  the runtime's resolve call inside `outgoing-handler::handle` reaches
  the private IP unimpeded.
* Mitigation depends on Phase B.
* Recommendation for embedders concerned about SSRF *today*: keep
  `allow_net = false` for untrusted code, and/or restrict the host-level
  resolver to refuse RFC1918/link-local responses (Option 3 as
  defence-in-depth, not as primary control).
