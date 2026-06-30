use std::path::{Path, PathBuf};

/// Validate a path before using it as a shell spawn working directory.
///
/// Returns the canonical path when it exists, is a readable directory, and
/// canonicalization succeeds.
pub fn validate_spawn_cwd(path: &Path) -> Option<PathBuf> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    std::fs::read_dir(&canonical).ok()?;
    Some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_existing_directory() {
        let dir = std::env::temp_dir();
        let validated = validate_spawn_cwd(&dir).expect("temp dir should validate");
        assert!(validated.is_dir());
    }

    #[test]
    fn rejects_missing_path() {
        assert!(validate_spawn_cwd(Path::new("/nonexistent/sherion-security-test"))
            .is_none());
    }

    #[test]
    fn rejects_file_path() {
        let file = std::env::temp_dir().join("sherion-security-file-test");
        std::fs::write(&file, b"x").unwrap();
        assert!(validate_spawn_cwd(&file).is_none());
        let _ = std::fs::remove_file(file);
    }
}
