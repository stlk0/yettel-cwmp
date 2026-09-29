# Development rules

A small single-purpose utility. Prefer deleting code over adding guards for hypothetical
cases. A new module, trait, layer or dependency needs a concrete current reason.

Before finishing a change run: `cargo fmt --check`,
`cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --locked`,
`cargo test --locked --all-features`, `scripts/cross-clippy.sh`.

Keep code, docs and commits in English. User-facing text lives only in `locales/en.json`
and `locales/sr.json`, with identical keys.

Invariants:
1. Tests and development never contact provider servers; use synthetic values and loopback.
2. Do not change the bundled endpoint, pins, device parameter values or protocol bytes
   unless that is the task; golden fixtures must match.
3. Verify the pin before sending any HTTP byte; no pin bypass, proxy or redirects; release
   builds omit `dev-provider-override`.
4. Save provider-rotated management credentials before acknowledging the RPC.
5. Credentials never appear in errors, Debug, stderr or panic output; the UI shows them
   only after an explicit user action.
6. Restore the terminal on every exit path.

GitHub Actions minutes are limited: read the CI section of `docs/maintaining.md` before
changing workflows.
