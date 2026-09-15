use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Result, VedError, message};
use crate::project::Project;

pub fn load(path: &Path) -> Result<Project> {
    let bytes = fs::read(path).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    if matches!(
        json.get("version").and_then(serde_json::Value::as_u64),
        Some(1 | 2)
    ) {
        json["version"] = serde_json::Value::from(crate::project::PROJECT_VERSION);
        if json.get("voice_clips").is_none() {
            json["voice_clips"] = serde_json::Value::Array(Vec::new());
        }
    }
    let project: Project = serde_json::from_value(json).map_err(|source| VedError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    project.validate()?;
    Ok(project)
}

pub fn save(path: &Path, project: &Project) -> Result<()> {
    project.validate()?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| message("project path must have a valid file name"))?;
    let temp = unique_temp_path(parent, file_name);
    let bytes = serde_json::to_vec_pretty(project)
        .map_err(|error| message(format!("could not serialize project: {error}")))?;

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|source| VedError::Io {
                path: temp.clone(),
                source,
            })?;
        file.write_all(&bytes).map_err(|source| VedError::Io {
            path: temp.clone(),
            source,
        })?;
        file.write_all(b"\n").map_err(|source| VedError::Io {
            path: temp.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| VedError::Io {
            path: temp.clone(),
            source,
        })?;
        drop(file);
        replace_file(&temp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn unique_temp_path(parent: &Path, file_name: &str) -> PathBuf {
    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, destination: &Path) -> Result<()> {
    fs::rename(temp, destination).map_err(|source| VedError::Io {
        path: destination.to_path_buf(),
        source,
    })
}

pub(crate) fn replace_existing(temp: &Path, destination: &Path) -> Result<()> {
    replace_file(temp, destination)
}

#[cfg(windows)]
fn replace_file(temp: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
    // SAFETY: both buffers are NUL-terminated and remain alive for the duration of the call.
    let success = unsafe { MoveFileExW(source_wide.as_ptr(), destination_wide.as_ptr(), flags) };
    if success == 0 {
        return Err(VedError::Io {
            path: destination.to_path_buf(),
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Canvas;

    #[test]
    fn save_replaces_existing_project_without_truncation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.json");
        let first = Project::new(Canvas {
            width: 1080,
            height: 1920,
            fps: 30,
        });
        save(&path, &first).unwrap();

        let second = Project::new(Canvas {
            width: 1920,
            height: 1080,
            fps: 60,
        });
        save(&path, &second).unwrap();

        assert_eq!(load(&path).unwrap(), second);
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "temporary save file should not remain"
        );
    }

    #[test]
    fn old_project_versions_migrate_in_memory() {
        let directory = tempfile::tempdir().unwrap();
        for version in [1, 2] {
            let path = directory.path().join(format!("project-v{version}.json"));
            std::fs::write(
                &path,
                format!(
                    r#"{{"version":{version},"canvas":{{"width":1080,"height":1920,"fps":30}},"media":[],"timeline":[]}}"#
                ),
            )
            .unwrap();
            let migrated = load(&path).unwrap();
            assert_eq!(migrated.version, crate::project::PROJECT_VERSION);
            assert!(migrated.text_overlays.is_empty());
            assert!(migrated.image_overlays.is_empty());
            assert!(migrated.audio_clips.is_empty());
            assert!(migrated.audio_ducking.is_empty());
            assert!(migrated.voice_clips.is_empty());
        }
    }
}
