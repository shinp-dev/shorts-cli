use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, VedError, message};
use crate::project::{self, Project};

const HISTORY_LIMIT: usize = 100;

pub fn record_before_edit(project_path: &Path, current: &Project) -> Result<()> {
    push(project_path, Stack::Undo, current)?;
    clear(project_path, Stack::Redo)
}

pub fn undo(project_path: &Path) -> Result<()> {
    restore(project_path, Stack::Undo, Stack::Redo, "undo")
}

pub fn redo(project_path: &Path) -> Result<()> {
    restore(project_path, Stack::Redo, Stack::Undo, "redo")
}

fn restore(project_path: &Path, source: Stack, destination: Stack, action: &str) -> Result<()> {
    let current = project::load(project_path)?;
    let snapshot_path =
        latest(project_path, source)?.ok_or_else(|| message(format!("nothing to {action}")))?;
    let snapshot = project::load(&snapshot_path)?;
    push(project_path, destination, &current)?;
    project::save(project_path, &snapshot)?;
    fs::remove_file(&snapshot_path).map_err(|source| VedError::Io {
        path: snapshot_path,
        source,
    })?;
    Ok(())
}

fn push(project_path: &Path, stack: Stack, project: &Project) -> Result<PathBuf> {
    let directory = stack_dir(project_path, stack)?;
    fs::create_dir_all(&directory).map_err(|source| VedError::Io {
        path: directory.clone(),
        source,
    })?;
    let sequence = snapshots(&directory)?
        .last()
        .and_then(|path| sequence(path))
        .unwrap_or(0)
        + 1;
    let path = directory.join(format!("{sequence:020}.json"));
    project::save(&path, project)?;
    prune(&directory)?;
    Ok(path)
}

fn clear(project_path: &Path, stack: Stack) -> Result<()> {
    let directory = stack_dir(project_path, stack)?;
    if !directory.exists() {
        return Ok(());
    }
    for path in snapshots(&directory)? {
        fs::remove_file(&path).map_err(|source| VedError::Io { path, source })?;
    }
    Ok(())
}

fn prune(directory: &Path) -> Result<()> {
    let entries = snapshots(directory)?;
    let remove_count = entries.len().saturating_sub(HISTORY_LIMIT);
    for path in entries.into_iter().take(remove_count) {
        fs::remove_file(&path).map_err(|source| VedError::Io { path, source })?;
    }
    Ok(())
}

fn latest(project_path: &Path, stack: Stack) -> Result<Option<PathBuf>> {
    let directory = stack_dir(project_path, stack)?;
    if !directory.exists() {
        return Ok(None);
    }
    Ok(snapshots(&directory)?.pop())
}

fn snapshots(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| VedError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    entries.sort();
    Ok(entries)
}

fn sequence(path: &Path) -> Option<u64> {
    path.file_stem()?.to_str()?.parse().ok()
}

fn stack_dir(project_path: &Path, stack: Stack) -> Result<PathBuf> {
    let absolute = std::fs::canonicalize(project_path).map_err(|source| VedError::Io {
        path: project_path.to_path_buf(),
        source,
    })?;
    let parent = absolute.parent().unwrap_or_else(|| Path::new("."));
    let file_name = absolute
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("project");
    let normalized = if cfg!(windows) {
        absolute.to_string_lossy().to_ascii_lowercase()
    } else {
        absolute.to_string_lossy().into_owned()
    };
    let key = format!(
        "{}-{:016x}",
        sanitize(file_name),
        fnv1a(normalized.as_bytes())
    );
    let history_root = parent.join(".ved").join("history");
    let project_history = history_root.join(key);
    let directory = project_history.join(stack.name());
    for component in [
        parent.join(".ved"),
        history_root,
        project_history,
        directory.clone(),
    ] {
        if component.exists()
            && std::fs::symlink_metadata(&component)
                .map_err(|source| VedError::Io {
                    path: component.clone(),
                    source,
                })?
                .file_type()
                .is_symlink()
        {
            return Err(message(format!(
                "history path must not contain a symlink or junction: {}",
                component.display()
            )));
        }
    }
    if directory.exists() {
        let resolved = std::fs::canonicalize(&directory).map_err(|source| VedError::Io {
            path: directory.clone(),
            source,
        })?;
        if !resolved.starts_with(parent) {
            return Err(message(
                "history directory resolves outside the project directory",
            ));
        }
    }
    Ok(directory)
}

fn sanitize(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "project".into()
    } else {
        sanitized
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Debug, Clone, Copy)]
enum Stack {
    Undo,
    Redo,
}

impl Stack {
    fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Canvas;

    #[test]
    fn undo_redo_and_branching_restore_whole_projects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.json");
        let mut project = Project::new(Canvas {
            width: 100,
            height: 200,
            fps: 30,
        });
        project::save(&path, &project).unwrap();

        record_before_edit(&path, &project).unwrap();
        project.canvas.fps = 60;
        project::save(&path, &project).unwrap();
        undo(&path).unwrap();
        assert_eq!(project::load(&path).unwrap().canvas.fps, 30);
        redo(&path).unwrap();
        assert_eq!(project::load(&path).unwrap().canvas.fps, 60);

        undo(&path).unwrap();
        let mut branch = project::load(&path).unwrap();
        record_before_edit(&path, &branch).unwrap();
        branch.canvas.width = 300;
        project::save(&path, &branch).unwrap();
        assert!(redo(&path).is_err());
    }

    #[test]
    fn undo_history_is_limited_to_one_hundred_projects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.json");
        let mut project = Project::new(Canvas {
            width: 100,
            height: 200,
            fps: 30,
        });
        project::save(&path, &project).unwrap();
        for width in 101..=205 {
            record_before_edit(&path, &project).unwrap();
            project.canvas.width = width;
            project::save(&path, &project).unwrap();
        }
        let undo_dir = stack_dir(&path, Stack::Undo).unwrap();
        assert_eq!(snapshots(&undo_dir).unwrap().len(), HISTORY_LIMIT);
    }
}
