//! Saving the full, untruncated document so a notice can point at it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Temp copies older than this are removed whenever a new one is saved.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Per-user directory for temp copies, created private on Unix.
fn temp_dir() -> Result<PathBuf, String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let dir = std::env::temp_dir().join(format!("full-outputs-{user}"));
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

fn remove_stale(dir: &Path) {
    let now = SystemTime::now();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > MAX_AGE);
            if old && e.file_name().to_string_lossy().starts_with("full-output-") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Write `text` to `dest`, or, if `dest` is `None`, to a fresh private file in
/// the per-user temp directory named with the given extension (`json`/`yaml`).
pub fn save(text: &str, ext: &str, dest: Option<&Path>) -> Result<PathBuf, String> {
    let path = match dest {
        Some(p) => p.to_path_buf(),
        None => {
            let dir = temp_dir()?;
            remove_stale(&dir);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            dir.join(format!("full-output-{stamp}-{}.{ext}", std::process::id()))
        }
    };
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}
