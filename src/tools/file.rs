//! File tool for reading/writing/listing files (task workers only).

use crate::error::Result;
use std::path::Path;

/// Read a file's contents.
pub async fn file_read(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    
    // Workspace path guard - reject reads from identity/memory paths
    if is_protected_path(path) {
        return Err(crate::error::AgentError::Other(
            anyhow::anyhow!("can't read from protected path: {}", path.display())
        ).into());
    }
    
    tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read file: {}", path.display()))
        .map_err(Into::into)
}

/// Write content to a file.
pub async fn file_write(
    path: impl AsRef<Path>,
    content: impl AsRef<[u8]>,
) -> Result<()> {
    let path = path.as_ref();
    
    // Workspace path guard
    if is_protected_path(path) {
        return Err(crate::error::AgentError::Other(
            anyhow::anyhow!("can't write to protected path: {}. Use memory_save tool instead.", path.display())
        ).into());
    }
    
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }
    
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("failed to write file: {}", path.display()))
        .map_err(Into::into)
}

/// List files in a directory.
pub async fn file_list(path: impl AsRef<Path>) -> Result<Vec<FileEntry>> {
    let path = path.as_ref();
    
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(path)
        .await
        .with_context(|| format!("failed to read directory: {}", path.display()))?;
    
    while let Some(entry) = reader.next_entry().await? {
        let metadata = entry.metadata().await?;
        let file_type = if metadata.is_file() {
            FileType::File
        } else if metadata.is_dir() {
            FileType::Directory
        } else {
            FileType::Other
        };
        
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            file_type,
            size: metadata.len(),
        });
    }
    
    Ok(entries)
}

/// Check if a path is in a protected location (identity, memory, etc.).
fn is_protected_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("prompts/")
        || path_str.contains("identity/")
        || path_str.contains("data/")
        || path_str.ends_with("SOUL.md")
        || path_str.ends_with("IDENTITY.md")
        || path_str.ends_with("USER.md")
}

/// File entry metadata.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
}

/// File type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Other,
}

use anyhow::Context as _;
