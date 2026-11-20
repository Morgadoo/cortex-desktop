use crate::account::AccountId;
use crate::reporting::config_failure;
use crate::support::error::StoreError;
use crate::support::settings_file::{self, Storage};
use tauri::AppHandle;

pub use cortex_tools::state::{ConnectorState, ToolPolicy};

pub fn connector_state_path(
    app_handle: &AppHandle,
    account: &AccountId,
) -> Result<std::path::PathBuf, String> {
    crate::settings::account_config_dir(app_handle, account)
        .map(|dir| dir.join("connector_state.json"))
}

pub fn load_connector_state(
    app_handle: &AppHandle,
    account: &AccountId,
) -> Result<ConnectorState, String> {
    let path = connector_state_path(app_handle, account)?;
    settings_file::read_json(&path, Storage::Plain)
        .inspect_err(|e| {
            if matches!(e, StoreError::Parse { .. }) {
                config_failure("Connector state load failed");
            }
        })
        .map(Option::unwrap_or_default)
        .map_err(String::from)
}

pub fn save_connector_state(
    app_handle: &AppHandle,
    account: &AccountId,
    state: &ConnectorState,
) -> Result<(), String> {
    let path = connector_state_path(app_handle, account)?;
    settings_file::write_json(&path, Storage::Plain, state)
        .inspect_err(|e| {
            if matches!(e, StoreError::Encode { .. }) {
                config_failure("Connector state save failed");
            }
        })
        .map_err(String::from)
}
