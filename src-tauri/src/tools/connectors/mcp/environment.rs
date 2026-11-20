//! What a local (stdio) server's process inherits from Cortex, and what it is given.

use super::*;

/// Variables a stdio server inherits from Cortex. Everything else is withheld, so a token exported
/// in the user's shell profile doesn't reach every local server; a server that needs one gets it
/// through its config's `env` or `secret_env`.
pub(super) const INHERITED_ENV: &[&str] = &[
    // Identity, locale and scratch space.
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TMPDIR",
    "__CF_USER_TEXT_ENCODING",
    // Corporate networks.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    // Servers that open a browser or use the keyring on Linux.
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    // Version managers whose shims the resolved PATH can point at.
    "NVM_DIR",
    "VOLTA_HOME",
    "PYENV_ROOT",
    "ASDF_DIR",
    "ASDF_DATA_DIR",
    // Windows programs fail to start, or fail oddly, without these.
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "TEMP",
    "TMP",
    "PROGRAMDATA",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "COMMONPROGRAMFILES",
    "COMMONPROGRAMFILES(X86)",
    "COMMONPROGRAMW6432",
    "COMPUTERNAME",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
];

/// Windows variable names are case-insensitive, and proxy variables are often lowercase.
pub(super) fn is_inherited(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    INHERITED_ENV.contains(&upper.as_str()) || upper.starts_with("LC_")
}

pub(super) fn inherited_env(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    vars.into_iter()
        .filter(|(name, _)| name.to_str().is_some_and(is_inherited))
        .collect()
}

/// The variables set on top of the inherited ones: the resolved search path, then the config's
/// `env` laid over it.
pub(super) fn child_env(
    env: &HashMap<String, String>,
    search_path: &str,
) -> HashMap<String, String> {
    let mut resolved = HashMap::from([("PATH".to_string(), search_path.to_string())]);
    resolved.extend(env.clone());
    resolved
}
