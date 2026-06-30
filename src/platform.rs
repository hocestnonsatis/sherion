/// Play the system terminal bell when configured.
pub fn play_audible_bell() {
    #[cfg(target_os = "linux")]
    {
        const CANDIDATES: &[&str] = &[
            "/usr/share/sounds/freedesktop/stereo/bell.oga",
            "/usr/share/sounds/freedesktop/stereo/complete.oga",
        ];
        for path in CANDIDATES {
            if std::path::Path::new(path).exists() {
                let _ = std::process::Command::new("paplay")
                    .arg(path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                return;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return;
    }

    tracing::debug!("audible bell requested but no platform sound backend available");
}

/// Returns true when the URL uses an allowed scheme for external opening.
pub fn is_safe_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }

    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    if let Some(rest) = url.strip_prefix("http://") {
        return !rest.is_empty();
    }
    if url.starts_with("mailto:") {
        return is_safe_mailto(url);
    }

    false
}

fn is_safe_mailto(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("mailto:") else {
        return false;
    };
    if rest.is_empty() || rest.contains(['\n', '\r', '?', '&']) {
        return false;
    }
    let lower = rest.to_ascii_lowercase();
    !(lower.contains("%0a") || lower.contains("%0d"))
}

/// Open a URL with the platform default handler.
pub fn open_url(url: &str) -> bool {
    if !is_safe_url(url) {
        tracing::warn!(%url, "refusing to open URL with disallowed scheme");
        return false;
    }

    let result = {
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open")
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported platform",
            ))
        }
    };

    match result {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, %url, "failed to open URL");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_http_and_https_urls() {
        assert!(is_safe_url("https://example.com/path"));
        assert!(is_safe_url("http://localhost:8080"));
    }

    #[test]
    fn allows_mailto_urls() {
        assert!(is_safe_url("mailto:user@example.com"));
    }

    #[test]
    fn rejects_file_and_javascript_urls() {
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("data:text/html,hello"));
    }

    #[test]
    fn rejects_mailto_header_injection() {
        assert!(!is_safe_url("mailto:a@b.com%0aBcc:c@evil.com"));
        assert!(!is_safe_url("mailto:a@b.com?subject=hi&body=bye"));
        assert!(is_safe_url("mailto:user@example.com"));
    }

    #[test]
    fn rejects_empty_scheme_only_urls() {
        assert!(!is_safe_url("https://"));
        assert!(!is_safe_url(""));
    }
}
