//! Canvases, mini apps, sheets, media and what chats produce.

use super::*;

#[tauri::command]
pub fn chat_media_models(app: AppHandle) -> Result<crate::chat::media::MediaModels, String> {
    Ok(crate::chat::media::models(&config_dir(&app)?))
}

/// Chooses the image, speech and voice models; an empty value goes back to the default.
#[tauri::command]
pub fn chat_set_media_models(
    app: AppHandle,
    image_model: Option<String>,
    speech_model: Option<String>,
    voice_model: Option<String>,
    video_model: Option<String>,
) -> Result<crate::chat::media::MediaModels, String> {
    let dir = config_dir(&app)?;
    let mut file = crate::chat::providers::load(&dir);
    file.image_model = image_model.filter(|m| !m.trim().is_empty());
    file.speech_model = speech_model.filter(|m| !m.trim().is_empty());
    file.voice_model = voice_model.filter(|m| !m.trim().is_empty());
    file.video_model = video_model.filter(|m| !m.trim().is_empty());
    crate::chat::providers::save(&dir, &file)?;
    Ok(crate::chat::media::models(&dir))
}

/// The local image server's address (AUTOMATIC1111 API); empty to remove it.
#[tauri::command]
pub async fn chat_set_image_server(
    app: AppHandle,
    url: Option<String>,
) -> Result<crate::chat::media::MediaModels, String> {
    let dir = config_dir(&app)?;
    let url = url
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    if let Some(u) = &url {
        if !(u.starts_with("http://") || u.starts_with("https://")) {
            return Err("Give the server's address, like http://127.0.0.1:7860.".into());
        }
        // A quick check that something answers there.
        let ok = crate::support::http::client(std::time::Duration::from_secs(8))?
            .get(format!("{u}/sdapi/v1/sd-models"))
            .send()
            .await;
        if !ok.is_ok_and(|r| r.status().is_success()) {
            return Err(format!("No AUTOMATIC1111-style image server answered at {u} (Stable Diffusion WebUI needs --api)."));
        }
    }
    let mut file = crate::chat::providers::load(&dir);
    file.image_server = url;
    crate::chat::providers::save(&dir, &file)?;
    Ok(crate::chat::media::models(&dir))
}

/// The checkpoints the local image server offers, as image models.
#[tauri::command]
pub async fn chat_image_server_models(app: AppHandle) -> Result<Vec<String>, String> {
    let Some(server) = crate::chat::providers::load(&config_dir(&app)?).image_server else {
        return Ok(Vec::new());
    };
    let list: Vec<serde_json::Value> = reqwest::get(format!("{server}/sdapi/v1/sd-models"))
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(list
        .iter()
        .filter_map(|m| m["model_name"].as_str().or(m["title"].as_str()))
        .map(|m| format!("{}{m}", crate::chat::media::SDWEBUI_PREFIX))
        .collect())
}

/// Reads an uploaded workbook (base64) into CSV text per sheet.
#[tauri::command]
pub fn chat_read_spreadsheet(data: String) -> Result<Vec<(String, String)>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    if bytes.len() > 30 * 1024 * 1024 {
        return Err("That workbook is too large.".into());
    }
    crate::chat::sheets::read_workbook(&bytes)
}

#[tauri::command]
pub fn chat_get_canvas(app: AppHandle, id: String) -> Result<crate::chat::canvas::Canvas, String> {
    crate::chat::canvas::load(&config_dir(&app)?, &id)
}

/// Saves the user's edit as a new version.
#[tauri::command]
pub fn chat_save_canvas(
    app: AppHandle,
    id: String,
    title: Option<String>,
    content: String,
) -> Result<crate::chat::canvas::Canvas, String> {
    let dir = config_dir(&app)?;
    let existing = crate::chat::canvas::load(&dir, &id)?;
    crate::chat::canvas::write(
        &dir,
        &existing.conversation_id,
        Some(&id),
        title.as_deref(),
        &content,
        "user",
    )
}

/// Opens an app canvas in a window of its own (LibreChat's artifact in a new window), or brings
/// its window forward when it is open already.
#[tauri::command]
pub fn chat_open_app_window(app: AppHandle, id: String) -> Result<(), String> {
    use tauri::Manager as _;
    let canvas = crate::chat::canvas::load(&config_dir(&app)?, &id)?;
    if canvas.kind != "app" {
        return Err("Only apps open in a window of their own.".into());
    }
    let label = format!(
        "app-{}",
        id.chars()
            .filter(char::is_ascii_alphanumeric)
            .take(24)
            .collect::<String>()
    );
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    let base = if cfg!(windows) {
        "http://cortexapp.localhost"
    } else {
        "cortexapp://localhost"
    };
    let url: tauri::Url = format!("{base}/window/{}", canvas.id)
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::CustomProtocol(url))
        .title(if canvas.title.is_empty() {
            "App".to_string()
        } else {
            canvas.title.clone()
        })
        .inner_size(1000.0, 760.0)
        .min_inner_size(360.0, 300.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Whether Microsoft's Fluent emoji in `style` (`3d` or `flat`) are on this device.
#[tauri::command]
pub fn chat_emoji_status(
    app: AppHandle,
    style: String,
) -> Result<crate::chat::emoji_pack::Status, String> {
    Ok(crate::chat::emoji_pack::status(&emoji_root(&app)?, &style))
}

/// Downloads an emoji pack in the background; `chat://emoji` reports when it is ready or failed.
#[tauri::command]
pub fn chat_emoji_install(
    app: AppHandle,
    style: String,
) -> Result<crate::chat::emoji_pack::Status, String> {
    use tauri::Emitter as _;
    let root = emoji_root(&app)?;
    crate::chat::emoji_pack::pack(&style).ok_or("Unknown emoji style.")?;
    let now = crate::chat::emoji_pack::status(&root, &style);
    if now.state == "ready" || now.state == "downloading" {
        return Ok(now);
    }
    let style2 = style.clone();
    let root2 = root.clone();
    tauri::async_runtime::spawn(async move {
        let result = crate::chat::emoji_pack::install(&root2, &style2).await;
        if let Err(e) = &result {
            log::warn!("the {style2} emoji could not be installed: {e}");
        }
        let _ = app.emit(
            "chat://emoji",
            crate::chat::emoji_pack::status(&root2, &style2),
        );
    });
    Ok(crate::chat::emoji_pack::Status {
        state: "downloading".into(),
        error: None,
    })
}

/// What a chat made (canvases, apps, images, files), or with no id, what every chat made.
#[tauri::command]
pub async fn chat_outputs(
    app: AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<crate::chat::outputs::Output>, String> {
    let dir = config_dir(&app)?;
    tokio::task::spawn_blocking(move || match conversation_id {
        Some(id) => crate::chat::outputs::for_conversation(&dir, &id),
        None => crate::chat::outputs::all(&dir),
    })
    .await
    .map_err(|e| e.to_string())
}

/// Saves a copy of a file from a chat's workspace where the user picks.
#[tauri::command]
pub async fn chat_save_output(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let dir = config_dir(&app)?;
    let ws = crate::chat::sandbox::workspace(&dir, &conversation_id)?;
    let file = ws.join(&path);
    let (Ok(root), Ok(real)) = (ws.canonicalize(), file.canonicalize()) else {
        return Err("No such file.".into());
    };
    if !real.starts_with(&root) {
        return Err("No such file.".into());
    }
    let bytes = crate::chat::at_rest::read(&real).map_err(|e| e.to_string())?;
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(target) =
        crate::support::dialogs::save_file(app.dialog().file().set_file_name(&name)).await?
    else {
        return Ok(false);
    };
    std::fs::write(target.into_path().map_err(|e| e.to_string())?, bytes)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// The video as Ogg Theora (base64), which the Linux media player can decode with only its base
/// plugins, for systems without H.264 or WebM support. Converted once and kept beside the clip
/// (in a hidden folder, out of Outputs).
#[tauri::command]
pub async fn chat_playable_video(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<String, String> {
    use base64::Engine as _;
    let real = workspace_file(&app, &conversation_id, &path)?;
    let ws = crate::chat::sandbox::workspace(&config_dir(&app)?, &conversation_id)?;
    let cache = ws
        .join(".playable")
        .join(format!("{}.ogv", path.replace(['/', '\\'], "_")));
    if !cache.exists() {
        let bytes = crate::chat::at_rest::read(&real).map_err(|e| e.to_string())?;
        if bytes.len() > 200 * 1024 * 1024 {
            return Err("The video is too large to convert here.".into());
        }
        let cache2 = cache.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(cache2.parent().ok_or("bad path")?)
                .map_err(|e| e.to_string())?;
            let tmp =
                std::env::temp_dir().join(format!("cortex-play-{}", uuid::Uuid::new_v4().simple()));
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            let out = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(&tmp)
                .args([
                    "-c:v",
                    "libtheora",
                    "-q:v",
                    "7",
                    "-c:a",
                    "libvorbis",
                    "-q:a",
                    "4",
                ])
                .arg(&cache2)
                .output();
            let _ = std::fs::remove_file(&tmp);
            match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(format!(
                    "The video could not be converted: {}",
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .last()
                        .unwrap_or_default()
                )),
                Err(_) => Err("ffmpeg is not installed.".to_string()),
            }
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    let ogv = std::fs::read(&cache).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(ogv))
}

/// A still from a video (JPEG, base64), for when the window cannot play it itself.
#[tauri::command]
pub async fn chat_video_poster(
    app: AppHandle,
    conversation_id: String,
    path: String,
) -> Result<String, String> {
    use base64::Engine as _;
    let real = workspace_file(&app, &conversation_id, &path)?;
    let bytes = crate::chat::at_rest::read(&real).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let tmp =
            std::env::temp_dir().join(format!("cortex-poster-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-ss", "1", "-i"])
            .arg(&tmp)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=960:-2",
                "-q:v",
                "4",
                "-f",
                "image2",
                "-c:v",
                "mjpeg",
                "pipe:1",
            ])
            .output();
        let _ = std::fs::remove_file(&tmp);
        let out = out.map_err(|_| "ffmpeg is not installed.".to_string())?;
        if out.stdout.is_empty() {
            return Err("No still could be taken.".into());
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(out.stdout))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Every canvas, newest change first, to add one to a chat as context.
#[tauri::command]
pub fn chat_list_canvases(app: AppHandle) -> Result<Vec<CanvasEntry>, String> {
    Ok(crate::chat::canvas::list_all(&config_dir(&app)?)
        .into_iter()
        .map(|c| CanvasEntry {
            id: c.id,
            title: c.title,
            kind: c.kind,
            conversation_id: c.conversation_id,
        })
        .collect())
}

#[tauri::command]
pub fn chat_list_apps(app: AppHandle) -> Result<Vec<crate::chat::canvas::Canvas>, String> {
    Ok(crate::chat::canvas::list_apps(&config_dir(&app)?))
}

/// Opens a local dev server's page in Cortex's browser window (the Terminal panel's preview).
#[tauri::command]
pub fn chat_open_preview(app: AppHandle, url: String) -> Result<(), String> {
    let parsed: url::Url = url
        .parse()
        .map_err(|_| "That is not a web address.".to_string())?;
    let local = matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if !local || !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only pages served on this computer open as a preview.".into());
    }
    crate::inapp_browser::open(&app, &parsed)
}

/// A video attachment as frames and a soundtrack transcript (ffmpeg).
#[tauri::command]
pub async fn chat_read_video(
    app: AppHandle,
    name: String,
    data: String,
) -> Result<crate::chat::video::Digest, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| e.to_string())?;
    if bytes.len() > 200 * 1024 * 1024 {
        return Err("Videos can be at most 200 MB.".into());
    }
    let ext = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("mp4")
        .to_string();
    // A private temporary file, removed once read.
    let path = std::env::temp_dir().join(format!(
        "cortex-video-{}.{ext}",
        uuid::Uuid::new_v4().simple()
    ));
    crate::chat::at_rest::write(&path, bytes).map_err(|e| e.to_string())?;
    let digest = crate::chat::video::digest(&config_dir(&app)?, &path).await;
    let _ = std::fs::remove_file(&path);
    digest.map_err(String::from)
}

/// A web page's title, description and preview image, for the hover card on a citation.
#[tauri::command]
pub async fn chat_link_preview(
    app: AppHandle,
    url: String,
) -> Result<crate::chat::preview::Preview, String> {
    crate::chat::preview::get(&config_dir(&app)?, &url).await
}
