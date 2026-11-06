# Security

The emulator runs local, user-supplied firmware in Renode. Treat firmware, models and monitor commands as trusted inputs; this application is not a sandbox for hostile code. Bridge and USB/IP listeners bind loopback; do not expose them through port forwarding. Review Renode's debugger/monitor exposure and local firewall policy.

Before sharing diagnostics, inspect paths, filenames, USB identifiers and log contents. Do not attach firmware, personal data, credentials or private references to public issues.

For suspected vulnerabilities, use the repository's private vulnerability reporting feature if enabled, or an owner-designated private channel. No reporting email or supported-release commitment has been established yet. Avoid public exploit details while arranging private contact. No security support is promised for unreviewed pre-release builds.
