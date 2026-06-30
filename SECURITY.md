# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability in Sherion, please report it responsibly:

1. **Preferred:** Open a [GitHub Security Advisory](https://github.com/hocestnonsatis/sherion/security/advisories/new) (private report).
2. **Alternative:** Open a GitHub issue labeled `security` if you cannot use private advisories.

Please include:

- A description of the vulnerability and its impact
- Steps to reproduce
- Affected version(s)
- Suggested fix (if any)

We aim to acknowledge reports within **72 hours** and provide a fix or mitigation plan within **30 days** for confirmed issues.

## Threat Model

Sherion is a **local terminal emulator**. It does not implement networking, remote sessions, or cloud sync.

| Trust boundary | Notes |
| -------------- | ----- |
| User input (keyboard, mouse, paste) | Trusted; paste is sanitized by default |
| User config (`sherion.toml`) | Trusted; can specify shell binary and args |
| PTY output (shell, SSH, remote programs) | **Untrusted** — can emit escape sequences with side effects |

### Side effects from untrusted PTY output

- **OSC 52** — programs can read/write the system clipboard (configurable)
- **OSC 8 hyperlinks** — programs can display clickable links (opening is restricted to `http`, `https`, `mailto`)
- **OSC 7** — programs can report a working directory (validated before use as spawn CWD)

## Known Limitations

- **Shell execution:** Sherion spawns a user shell with full user privileges. Custom `terminal.shell` / `terminal.shell_args` in config can run arbitrary programs.
- **OSC 52 copy:** Remote or untrusted programs can write to the system clipboard when `osc52` includes copy (default: `copypaste`).
- **No sandbox:** There is no privilege separation between Sherion and spawned shells.
- **Windows ConPTY:** The vendored terminal backend may load `con.bash` from PATH or the install directory; avoid running from world-writable locations.
- **Install script:** The recommended `curl | sh` installer trusts the GitHub release channel; verify checksums when possible.
- **Vendored parser:** Sherion patches `alacritty_terminal` locally; upstream security fixes must be merged manually.

## Hardening Recommendations

For sessions connecting to untrusted or remote hosts:

```toml
[terminal]
sanitize_paste = true   # default
osc52 = "copy"          # or "disabled" — blocks clipboard read into shell
```

Additional guidance:

- Restrict write access to your config file (`chmod 600 ~/.config/sherion/sherion.toml`).
- Do not run Sherion from directories you do not trust (legacy `./sherion.toml` fallback).
- Keep Sherion updated to the latest release.

## Security Controls (0.1.1+)

- URL opening restricted to `http://`, `https://`, and `mailto:` schemes
- OSC 52 paste responses sanitized when `sanitize_paste = true`
- Spawn working directories validated (must exist, be a readable directory)
- Config defaults to XDG path (`~/.config/sherion/sherion.toml`)
- Windows custom shell arguments escaped when `shell_args` is non-empty
- CI runs `cargo audit` and `cargo deny` on every push/PR
