# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- An install script for macOS and Linux that downloads the latest release with curl, checks it against `SHA256SUMS` and installs it to `~/.local/bin`; macOS opens the unsigned app without a security prompt.

## [0.1.0] - 2026-09-29

### Added

- First public release: a terminal application that receives the internet (PPPoE) username and password for a Yettel / CETIN Serbia line by acting as the ZTE ZXHN H3600P router.
- English and Serbian Latin interfaces with saved routers, offline viewing of saved settings, Wi-Fi key replacement, deletion, help, explicit credential reveal/copy and stable error codes.
- A connection warning before every receive operation, cancellable progress and always-visible error context.
- Certificate/SPKI pinning verified before any HTTP data, with TLS resumption, redirects and proxies disabled.
- Management credentials changed by the provider are saved before they are acknowledged; complete internet credentials survive late cancellation, the session deadline and connection loss.
- Linux x64/ARM64, Windows x64/ARM64 and universal macOS release archives with checksums, build provenance and third-party notices.
