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

/// Open a URL with the platform default handler.
pub fn open_url(url: &str) -> bool {
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
