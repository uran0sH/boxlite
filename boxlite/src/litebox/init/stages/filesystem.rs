//! Stage 1: Filesystem setup.
//!
//! Creates box directory structure.

use crate::litebox::init::types::{FilesystemInput, FilesystemOutput};
use boxlite_shared::errors::BoxliteResult;

/// Create box directories.
///
/// **Single Responsibility**: Only creates directories, nothing else.
pub fn run(input: FilesystemInput<'_>) -> BoxliteResult<FilesystemOutput> {
    let layout = input
        .runtime
        .non_sync_state
        .layout
        .box_layout(input.box_id.as_str());

    layout.prepare()?;

    Ok(FilesystemOutput { layout })
}
