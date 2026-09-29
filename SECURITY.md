# Security policy

## Report privately

Use GitHub's [Report a vulnerability](https://github.com/stlk0/yettel-cwmp/security/advisories/new) form when private vulnerability reporting is enabled. Include a concise description, affected version, impact and reproduction steps using synthetic data.

**Do not publish passwords, Wi-Fi keys, serial numbers, MAC addresses, label photographs, saved profiles, exported settings or raw provider traffic in an issue or pull request.** Do not test a suspected issue against provider infrastructure or someone else's router.

If the private form is unavailable, ask the maintainer for a private reporting channel without disclosing vulnerability details or personal data in the public request. No response-time guarantee is currently offered.

## Supported versions

Security fixes target the latest published release. There is no separate long-term support branch. Development builds, including provider-override builds, are intended only for synthetic local testing.

## Scope

See [Security model](docs/maintaining.md#session-and-storage) for storage, TLS, credential persistence and disclosure protections. Local credentials are plaintext and memory erasure is best-effort. A provider-side provisioning issue may require coordination beyond this project; do not contact the provider on the maintainer's behalf without authorization.
