# Maintaining yettel-cwmp

Keep this a small utility. Prefer removing code; a new module, trait, layer or dependency needs a current reason. Code, documentation and commits are English; user-facing text belongs in the paired language catalogs. Keep private notes, labels, captures and credentials outside Git.

## Build and checks

Use Rust **1.98.1** from `rust-toolchain.toml`; preserve the MSRV declared in `Cargo.toml`. Install the platform C toolchain: GCC/Clang, Xcode Command Line Tools, or Visual Studio C++ Build Tools. On Linux, cross-platform Clippy also needs `clang`, `gcc-mingw-w64-x86-64` and `musl-tools`.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --locked --all-features
cargo build --release --locked
scripts/cross-clippy.sh
```

Cross-Clippy type-checks Windows, macOS and musl code; it does not establish native runtime behavior. Tests use synthetic data, temporary directories, loopback sockets and Unix PTYs. Report sandbox or native-platform limitations rather than weakening checks. Never contact provider servers for development or automated testing. A release check on the maintainer's own line is a separate manual operation; preserve any new management credentials and record only redacted results.

The default release binary is `target/release/yettel-cwmp` (`.exe` on Windows). Never enable `dev-provider-override` in a distributed build. The release guard rejects `YETTEL_CWMP_DEV_PROVIDER`, case-insensitive `dev build`, and `RAZVOJNA VERZIJA` in that binary.

When dependencies change, compare the Linux GNU release size and dependency count on the same toolchain/profile. Use `cargo tree -d -e normal` and the workflow's pinned cargo-deny version for dependency review.

## Session and storage

`capture` validates the selected profile before constructing a transport. `cwmp` sends Inform, dispatches RPCs and receives assignments; `net` implements pinned HTTPS, Digest and bounded HTTP framing. The TUI keeps App/Screen/Action, views, worker and form modules.

Verify the certificate/SPKI pin and TLS handshake signature before any HTTP byte. Pins are the trust anchor; CA-chain, hostname and expiry validation are not additional checks. Keep resumption, proxies and redirects disabled. Idle connections may reconnect before a request; never replay a POST after uncertain delivery. Keep response limits at 64 KiB headers, 8 MiB body and 8 KiB chunk lines.

Persist accepted management credentials before acknowledging their RPC, without a cancellation gap. A complete PPP pair survives late cancellation, the 15-minute deadline or connection loss; provider, TLS and storage failures still propagate. Incomplete captures leave the previous export intact. Network waits have a 30-second inactivity budget.

One Store holds the root `.lock` for the application's lifetime. Files remain under `devices/<SERIAL>/`; deletion removes that router's directory. Create Unix directories as 0700 and files as 0600, leaving existing directory modes alone; Windows inherits access. Write with NamedTempFile, sync, persist, then sync the parent directory once on Unix. Storage remains plaintext, and secret-buffer erasure is best-effort.

Profiles are saved as format version 2 with serial, MAC, management username/password and `credentials_source` (`label` or `server`). Format 1 files from pre-release builds are read without rewriting; their credentials, and the old value `unknown`, count as `server`. Unknown fields are ignored. Keep the `profile_v1` and export fixtures unchanged when simplifying representation.

Restore the terminal after ordinary exit, errors, cancellation, signals and panic. Never print panic payloads or secrets in errors/Debug/stderr. Sanitize untrusted displayed text with `sanitize::safe`; pasted content is data. Offline viewing never constructs a transport. Exit codes are 0 for ordinary exit/cancellation, 1 for startup/argument errors, and 2 without a TTY; help/version need no TTY.

## Fake ACS and manual QA

Build and start the one-session loopback server in terminal 1:

```sh
cargo build --locked --features dev-provider-override --examples --bins
QA_DIR=$(mktemp -d)
printf 'QA_DIR=%s\n' "$QA_DIR"
cargo run --locked --features dev-provider-override --example fake_acs -- \
  --scenario success --write-provider "$QA_DIR/provider.json"
```

Copy the printed QA_DIR path into terminal 2, then run:

```sh
QA_DIR=/tmp/the-printed-directory
YETTEL_CWMP_DEV_PROVIDER="$QA_DIR/provider.json" \
  ./target/debug/yettel-cwmp --state-dir "$QA_DIR/state" --lang en
```

Verify the development badge before receiving settings. Override files use `{"acs":{"host":"localhost","port":12345,"path":"/synthetic"},"pins":[{"kind":"certificate_sha256","sha256":"<64 lowercase hex characters>"}]}`; only `localhost` and `127.0.0.1` are accepted. The fake server writes its actual port and pin. Keep development-only banner text in `src/dev.rs`, outside embedded locale JSON.

Use serial `SYN123456`, MAC `02:11:22:33:44:55`, and key `synthetic-wlan`. The server exits after one session; restart both server and app for each new scenario so the app reloads its port. Retain the same temporary state when checking saved credentials.

| Step | Check |
|---|---|
| Add | N; enter the synthetic label, paste data, and toggle Ctrl+R/F2. The key starts hidden. |
| Receive | R shows the warning every time; Esc returns without connecting, Enter starts progress. |
| Result | Both credentials start hidden; S reveals/hides, C/U copy explicitly, PgUp/PgDn/Home/End scroll. |
| Offline | Stop the fake server; Esc then V opens the saved result without a connection. |
| Change key | K opens the editor; enter a replacement key and press Enter. After rotation, Esc cancels confirmation and Y replaces the saved credentials. |
| Delete | D then Esc preserves records; D then Y removes the router's local directory. |

Repeat with `--lang sr` at **52×16**, checking every title, action, warning and error. Repeat with `NO_COLOR=1`; check help, Esc, Q, Ctrl+C, and a resize below minimum. Capture only synthetic screens from the running app. For tmux clipboard checks, use `set -g set-clipboard on`.

Restart with `--scenario auth-rejected`, `http-500`, `incomplete`, `malformed`, `slow` and `pin-mismatch`. Expect ACS-AUTH, ACS-HTTP, ACS-INCOMPLETE, ACS-PROTOCOL, cancellation during slow receiving, and TLS-PIN with no HTTP sent. Errors always show stage and last RPC. Check key correction after rejection and ensure previous complete settings survive failed captures. `rotate-then-slow` exercises cancellation after management rotation.

## Update a certificate pin

Only the maintainer obtains and independently verifies current or planned provider certificates or keys, through a trusted channel. Agents and automated tests never retrieve the live certificate. From a verified local PEM file, calculate either hash offline:

```sh
openssl x509 -in verified-provider.pem -outform der | openssl dgst -sha256
openssl x509 -in verified-provider.pem -pubkey -noout \
  | openssl pkey -pubin -outform der | openssl dgst -sha256
```

The first hashes the full certificate (`certificate_sha256`), the second SPKI (`spki_sha256`). Put the lowercase 64-character digest in `assets/providers/cetin-rs.json`. Add verified overlap pins before rotation and keep the old pin until it is confirmed obsolete. A new public key requires a new SPKI pin; never add a bypass.

Run TLS tests for changed certificates, same-key renewal, pin-before-HTTP and handshake signatures, then all checks. Publish a verified update before a planned rotation; a live-line check is a separate manual step. Do not request user labels or credentials when investigating a mismatch.

## Add a router model

Use `assets/devices/zte-h3600p.json` as the schema: parameter values/types, hidden and writable paths, credential paths, Inform parameters and identity rules. Supported identity placeholders are `{serial}`, `{prefixed_serial}`, `{router_mac}` and `{router_mac+N}`. Derive values only from synthetic identities in tests.

Add the JSON template, update the explicit bundled selection in `catalog::bundled`, and run `DeviceTemplate::validate` to catch bad parameter references and placeholders. Preserve the existing model's 1,484 parameters, provider endpoint/pins and wire fixtures unless their change is the task. The management address is deliberately unspecified; do not replace it with an observed subscriber address. The endpoint is structured `acs: {host, port, path}`; IPv6 endpoints are unsupported.

Golden fixtures under `tests/fixtures/golden` cover exact SOAP, Inform, HTTP and Digest bytes, including header order and empty Keep-Alive. Compare them before touching protocol code; do not regenerate goldens merely to make a refactor pass. VLAN/MTU/IPTV data are reference presets, not discovered subscription settings.

The emulator leaves `WANIPConnection.3.ExternalIPAddress` empty because it does not know the router's DHCP management address. [TR-098](https://cwmp-data-models.broadband-forum.org/tr-098-1-0-0.html) uses an empty string for an unspecified IP address, and capture works with this representation.

## Translations

Edit `locales/en.json` and `locales/sr.json` together. Use flat dotted keys and complete phrases with matching `{name}` placeholders. Call `t()` or `tf()`; the language is selected once at startup from `--lang` or the system locale, defaulting to English.

Tests compare key sets, placeholder sets, source references, unused keys and all `error.<code>` entries. Keep equivalent security guidance in both languages. Sanitize inserted untrusted values before rendering. Never move the development markers into locale JSON, since both catalogs are embedded in release builds. Obtain native Serbian proofreading and check narrow terminals.

## Release

1. Update the package/lockfile version and dated CHANGELOG entry. Run all checks, native runtime checks and fake-ACS QA in both languages; record actual results and Linux size/dependency changes.
2. Review secret handling, pin/signature checks, rotation-before-acknowledgment, atomic saves, application locking and terminal restoration. Compare bundled assets and wire fixtures; keep private data out of the diff.
3. Run Release manually with `dry_run: true` on the candidate; it packages without publishing. Do not claim success before every actual job completes.
4. After review and merge, the maintainer pushes the matching `v<version>` tag. A public repository then creates a draft release with checksums, notices and provenance.
5. Review filenames, contents, hashes, release notes and the approved commit before publishing the draft. Do not replace existing tags/releases to hide a failed build.

Five archives cover Linux x64/ARM64 (musl), Windows x64/ARM64 and universal macOS (x86_64 plus arm64). Each contains the executable, README.txt, LICENSE and THIRD_PARTY_LICENSES.html. Every target runs `--version`/`--help`; the universal binary's slices are checked with `lipo`.

`install.sh` is served from `main` and installs the latest release on macOS and Linux, so a change reaches users without a release. It finds the archive by its `-<platform>.tar.gz` suffix in `SHA256SUMS` and expects the four files at the archive root; keep archive names and layout stable. Check changes with `shellcheck -s sh install.sh` and a run under `dash`.

Use the workflow's pinned cargo-about to generate notices, then `python3 packaging/release.py verify-licenses THIRD_PARTY_LICENSES.html`. Notices cover normal dependencies across every release architecture. `packaging/release.py` verifies members, checksums, notes and absence of development markers. Preserve public-tag-only provenance and draft publication.

## Verify a download

Compare `SHA256SUMS` with `sha256sum ARCHIVE` on Linux, `shasum -a 256 ARCHIVE` on macOS, or `Get-FileHash ARCHIVE -Algorithm SHA256` in PowerShell. Obtain the archive and checksums from the same release; matching hashes establish integrity, not independent trust in the publisher.

With GitHub CLI, verify build provenance using `gh attestation verify ARCHIVE --repo stlk0/yettel-cwmp`. The install script checks `SHA256SUMS` but not provenance. Binaries remain unsigned and macOS builds are not notarized; follow the README's per-app opening guidance.

## CI economy

CI runs on Linux for pull requests and main pushes, with concurrency cancellation. One cached job combines formatting, host/cross Clippy, default/all-feature tests and the default-release guard; cargo-deny stays separate. Pin every action to a reviewed full commit SHA, including Swatinem/rust-cache v2. Avoid duplicate native Clippy jobs.

Documentation-only paths (`**.md`, `docs/**`, `LICENSE`) may skip CI while it is not a required check. Before making CI required, remove that path filtering so documentation-only PRs can complete their required checks. Recheck branch protection and rulesets when changing this policy.

Native tests cover `x86_64-unknown-linux-musl`, `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. In private repositories, run only for `full-ci`, manual dispatch or a version tag. Adding `full-ci` starts a run; later pushes keep running it until the label is removed. In public repositories, Native also runs on PRs/main pushes. Preserve its `if` and concurrency conditions.

Private repositories may set `NATIVE_RUNNERS`, for example `{"aarch64-apple-darwin":["self-hosted","macOS","ARM64"]}`. Unmapped targets use hosted runners; public repositories ignore this variable. Self-hosted machines need rustup and the native C toolchain and execute repository code.

Release PR dry runs are public-only and limited to packaging-related paths; private repositories use manual dry runs or tags. Publication/provenance require a public matching-tag push. Keep five archives, cargo-about and packaging verification. Only publication gets write/attestation permissions. Dependabot checks Cargo and Actions monthly to avoid unnecessary CI runs.

## Tested scope

Captures with release binaries succeeded on an active Yettel/CETIN line with the original ZTE router disconnected; management credentials changed and were retained. The pinned endpoint was also reachable from a network outside Yettel/CETIN, but receiving settings from other ISPs was not tested. Behavior while the original router stays connected, a fresh PPPoE login with the exported password, and IPTV were not tested. Do not turn these observations into general compatibility guarantees.

The label captions `S/N`, `MAC`, `WLAN Security` and `WLAN SSID` match a real H3600P label; `docs/router-label.svg` uses made-up values only.

See [README](../README.md), [security reporting](../SECURITY.md), and the [synthetic fixture notes](../tests/fixtures/README.md).
