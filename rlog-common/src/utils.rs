use std::path::Path;

use anyhow::Context;

/// Read a file.
///
/// In case of error, a human readable context is added to the underlying error.
pub fn read_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<u8>> {
    let path = path.as_ref();
    std::fs::read(path).with_context(|| format!("Cannot open file {}", path.to_string_lossy()))
}
