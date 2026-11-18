pub mod builder;
pub mod commands;
pub mod state;

use std::path::PathBuf;

use tauri::{Manager, Runtime};

use crate::util::errors::AppError;

use self::state::AppState;

pub fn sync_asset_protocol_scope<R: Runtime, M: Manager<R>>(
    manager: &M,
    state: &AppState,
) -> Result<(), AppError> {
    allow_asset_directories(
        manager,
        state.cache_data_dir(),
        &state.app_settings_snapshot().indexed_roots,
    )
}

pub(super) fn allow_asset_directories<R: Runtime, M: Manager<R>>(
    manager: &M,
    cache_dir: PathBuf,
    roots: &[String],
) -> Result<(), AppError> {
    let scope = manager.asset_protocol_scope();
    let mut allowed_dirs = vec![cache_dir];
    allowed_dirs.extend(roots.iter().map(PathBuf::from));
    allowed_dirs.sort();
    allowed_dirs.dedup();

    for path in allowed_dirs {
        scope.allow_directory(&path, true).map_err(|error| {
            AppError::Message(format!(
                "failed to authorize asset path {}: {error}",
                path.display()
            ))
        })?;
    }

    Ok(())
}
