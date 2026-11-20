fn main() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-env-changed=LUMO_URL");
    println!("cargo:rerun-if-env-changed=CORTEX_SENTRY_DSN_KEY_MACOS");
    println!("cargo:rerun-if-env-changed=CORTEX_SENTRY_DSN_KEY_WINDOWS");
    println!("cargo:rerun-if-env-changed=CORTEX_SENTRY_DSN_KEY_LINUX");
    println!("cargo:rerun-if-env-changed=CORTEX_UPDATE_MANIFEST_URL");
    println!("cargo:rerun-if-env-changed=CORTEX_RELEASES_REPO");

    let dist = std::path::Path::new("dist");
    if dist.exists() {
        for entry in walk_dist(dist) {
            println!("cargo:rerun-if-changed={}", entry.display());
        }
    }

    tauri_build::build()
}

fn walk_dist(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return paths;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        paths.push(path.clone());
        if path.is_dir() {
            paths.extend(walk_dist(&path));
        }
    }
    paths
}
