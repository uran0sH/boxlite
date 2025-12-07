//! Stage 5: Box spawn.
//!
//! Starts the boxlite-shim subprocess and waits for guest readiness.

use crate::controller::ShimController;
use crate::litebox::init::types::{SpawnInput, SpawnOutput};
use crate::util::find_binary;
use crate::vmm::{VmmController, VmmKind};
use boxlite_shared::errors::BoxliteResult;

/// Spawn box subprocess.
///
/// **Single Responsibility**: Subprocess creation and boot.
pub async fn run(input: SpawnInput<'_>) -> BoxliteResult<SpawnOutput> {
    let mut controller = ShimController::new(
        find_binary("boxlite-shim")?,
        VmmKind::Libkrun,
        input.box_id.clone(),
    )?;

    let guest_session = controller.start(input.config).await?;

    Ok(SpawnOutput {
        controller,
        guest_session,
    })
}
