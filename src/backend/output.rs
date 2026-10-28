//! Publish output by replacing a directory entry, never truncating a hard link.
use std::{fs, path::Path, path::PathBuf, time::SystemTime, time::UNIX_EPOCH};

pub fn check(destination: &Path, protected: &[PathBuf]) -> Result<(), String> {
    if let Ok(output) = destination.canonicalize() {
        if protected
            .iter()
            .filter_map(|p| p.canonicalize().ok())
            .any(|p| p == output)
        {
            return Err("Output must not overwrite a source file".into());
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if !metadata.file_type().is_file() {
            return Err("Output must be a regular file".into());
        }
    }
    Ok(())
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn publish(
    destination: &Path,
    protected: &[PathBuf],
    prepare: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), String> {
    check(destination, protected)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = parent.join(format!(".helium-output-{}-{stamp}", std::process::id()));
    // Exclusive creation: cleanup can only own this freshly created directory.
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    let staging = Staging(path);
    let file = staging.0.join("output");
    prepare(&file).map_err(|e| format!("Cannot prepare output: {e}"))?;
    check(destination, protected)?;
    fs::rename(&file, destination)
        .map_err(|e| format!("Cannot publish {}: {e}", destination.display()))
}

pub fn write(destination: &Path, contents: &[u8], protected: &[PathBuf]) -> Result<(), String> {
    publish(destination, protected, |file| fs::write(file, contents))
}

pub fn copy(source: &Path, destination: &Path, protected: &[PathBuf]) -> Result<(), String> {
    publish(destination, protected, |file| {
        fs::copy(source, file).map(|_| ())
    })
}
