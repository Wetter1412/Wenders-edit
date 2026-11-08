pub mod editor;
pub mod gdrive;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use editor::{DocumentMetadata, DocumentSession, EditorManager, SearchMatch};
use gdrive::{GDriveFile, GoogleDriveClient, OAuthToken};
use serde::Serialize;
use std::path::Path;
use tauri::State;
use tokio::sync::Mutex;

pub struct AppState {
    pub editor: Mutex<EditorManager>,
    pub gdrive: Mutex<GoogleDriveClient>,
}

#[derive(Serialize)]
pub struct TabContentResponse {
    pub metadata: DocumentMetadata,
    pub content: String,
}

#[derive(Serialize)]
pub struct ReplaceResponse {
    pub metadata: DocumentMetadata,
    pub content: String,
    pub replaced_count: usize,
}

#[derive(Serialize)]
pub struct CustomFontData {
    pub font_family: String,
    pub data_url: String,
}

#[tauri::command]
async fn create_new_tab(
    title: Option<String>,
    state: State<'_, AppState>,
) -> Result<TabContentResponse, String> {
    let mut editor = state.editor.lock().await;
    let tab_title = title.unwrap_or_else(|| format!("Untitled-{}.txt", editor.sessions.len() + 1));
    let doc = DocumentSession::new_empty(&tab_title);
    let meta = doc.metadata.clone();
    let id = meta.id.clone();
    editor.sessions.insert(id.clone(), doc);
    editor.active_tab_id = Some(id);

    Ok(TabContentResponse {
        metadata: meta,
        content: String::new(),
    })
}

#[tauri::command]
async fn get_all_tabs(state: State<'_, AppState>) -> Result<Vec<DocumentMetadata>, String> {
    let editor = state.editor.lock().await;
    Ok(editor.sessions.values().map(|s| s.metadata.clone()).collect())
}

#[tauri::command]
async fn switch_tab(tab_id: String, state: State<'_, AppState>) -> Result<TabContentResponse, String> {
    let mut editor = state.editor.lock().await;
    let session = editor.sessions.get(&tab_id).ok_or("Không tìm thấy Tab yêu cầu")?;
    let resp = TabContentResponse {
        metadata: session.metadata.clone(),
        content: session.rope.to_string(),
    };
    editor.active_tab_id = Some(tab_id);
    Ok(resp)
}

#[tauri::command]
async fn close_tab(tab_id: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut editor = state.editor.lock().await;
    editor.sessions.remove(&tab_id);
    if editor.sessions.is_empty() {
        let new_doc = DocumentSession::new_empty("Untitled-1.txt");
        let new_id = new_doc.metadata.id.clone();
        editor.sessions.insert(new_id.clone(), new_doc);
        editor.active_tab_id = Some(new_id);
    } else if editor.active_tab_id.as_deref() == Some(&tab_id) {
        editor.active_tab_id = editor.sessions.keys().next().cloned();
    }
    Ok(editor.active_tab_id.clone())
}

#[tauri::command]
async fn open_local_file(path: String, state: State<'_, AppState>) -> Result<TabContentResponse, String> {
    let mut editor = state.editor.lock().await;
    let (metadata, content) = editor.open_local_file(&path)?;
    Ok(TabContentResponse { metadata, content })
}

#[tauri::command]
async fn update_buffer(
    tab_id: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<DocumentMetadata, String> {
    let mut editor = state.editor.lock().await;
    editor.update_content(&tab_id, &content)
}

#[tauri::command]
async fn save_local_file(
    tab_id: String,
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<DocumentMetadata, String> {
    let mut editor = state.editor.lock().await;
    editor.save_local_file(&tab_id, path)
}

#[tauri::command]
async fn undo_action(tab_id: String, state: State<'_, AppState>) -> Result<TabContentResponse, String> {
    let mut editor = state.editor.lock().await;
    let (metadata, content) = editor.undo(&tab_id)?;
    Ok(TabContentResponse { metadata, content })
}

#[tauri::command]
async fn redo_action(tab_id: String, state: State<'_, AppState>) -> Result<TabContentResponse, String> {
    let mut editor = state.editor.lock().await;
    let (metadata, content) = editor.redo(&tab_id)?;
    Ok(TabContentResponse { metadata, content })
}

#[tauri::command]
async fn search_text(
    tab_id: String,
    query: String,
    case_sensitive: bool,
    use_regex: bool,
    state: State<'_, AppState>,
) -> Result<Vec<SearchMatch>, String> {
    let editor = state.editor.lock().await;
    editor.search_in_tab(&tab_id, &query, case_sensitive, use_regex)
}

#[tauri::command]
async fn replace_all_text(
    tab_id: String,
    query: String,
    replacement: String,
    case_sensitive: bool,
    use_regex: bool,
    state: State<'_, AppState>,
) -> Result<ReplaceResponse, String> {
    let mut editor = state.editor.lock().await;
    let (metadata, content, replaced_count) =
        editor.replace_all_in_tab(&tab_id, &query, &replacement, case_sensitive, use_regex)?;
    Ok(ReplaceResponse {
        metadata,
        content,
        replaced_count,
    })
}

#[tauri::command]
async fn load_custom_font(path: String) -> Result<CustomFontData, String> {
    let file_path = Path::new(&path);
    let bytes = std::fs::read(file_path)
        .map_err(|e| format!("Không thể đọc file phông chữ: {}", e))?;

    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("ttf")
        .to_lowercase();

    let mime = match ext.as_str() {
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "font/ttf",
    };

    let raw_name = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("CustomUserFont");

    let clean_family: String = raw_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    let font_family = if clean_family.is_empty() {
        "WEditCustomFont".to_string()
    } else {
        clean_family
    };

    let b64 = BASE64_STANDARD.encode(&bytes);
    let data_url = format!("data:{};base64,{}", mime, b64);

    Ok(CustomFontData {
        font_family,
        data_url,
    })
}

#[tauri::command]
async fn gdrive_get_auth_url(state: State<'_, AppState>) -> Result<String, String> {
    let mut drive = state.gdrive.lock().await;
    Ok(drive.generate_auth_url())
}

#[tauri::command]
async fn gdrive_authenticate(auth_code: String, state: State<'_, AppState>) -> Result<OAuthToken, String> {
    let mut drive = state.gdrive.lock().await;
    drive.exchange_code_for_token(&auth_code).await
}

#[tauri::command]
async fn gdrive_list_files(folder_id: Option<String>, state: State<'_, AppState>) -> Result<Vec<GDriveFile>, String> {
    let drive = state.gdrive.lock().await;
    drive.list_files(folder_id.as_deref()).await
}

#[tauri::command]
async fn gdrive_open_file(
    file_id: String,
    file_name: String,
    state: State<'_, AppState>,
) -> Result<TabContentResponse, String> {
    let content = {
        let drive = state.gdrive.lock().await;
        drive.download_file_content(&file_id).await?
    };

    let mut editor = state.editor.lock().await;
    let metadata = editor.load_from_gdrive(file_name, file_id, content.clone());
    Ok(TabContentResponse { metadata, content })
}

#[tauri::command]
async fn gdrive_save_file(
    tab_id: String,
    parent_folder_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<DocumentMetadata, String> {
    let (title, existing_gdrive_id, content) = {
        let editor = state.editor.lock().await;
        let session = editor.sessions.get(&tab_id).ok_or("Không tìm thấy Tab cần đồng bộ")?;
        (
            session.metadata.title.clone(),
            session.metadata.gdrive_file_id.clone(),
            session.rope.to_string(),
        )
    };

    let drive = state.gdrive.lock().await;
    let uploaded_file = if let Some(gdrive_id) = existing_gdrive_id {
        drive.update_existing_file(&gdrive_id, &content).await?
    } else {
        drive.create_new_file(&title, &content, parent_folder_id.as_deref()).await?
    };

    let mut editor = state.editor.lock().await;
    let session = editor.sessions.get_mut(&tab_id).ok_or("Tab đã bị đóng")?;
    session.metadata.gdrive_file_id = Some(uploaded_file.id);
    session.metadata.is_modified = false;

    Ok(session.metadata.clone())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 🛠️ [KHU VỰC DỄ CHỈNH SỬA 7]: Dán mã Google Client ID của bạn vào chữ YOUR_GOOGLE_CLIENT_ID bên dưới
    let client_id = std::env::var("GOOGLE_CLIENT_ID")
        .unwrap_or_else(|_| "YOUR_GOOGLE_CLIENT_ID.apps.googleusercontent.com".to_string());
    let redirect_uri = "http://localhost:8080/oauth2callback";

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            editor: Mutex::new(EditorManager::new()),
            gdrive: Mutex::new(GoogleDriveClient::new(&client_id, redirect_uri)),
        })
        .invoke_handler(tauri::generate_handler![
            create_new_tab,
            get_all_tabs,
            switch_tab,
            close_tab,
            open_local_file,
            update_buffer,
            save_local_file,
            undo_action,
            redo_action,
            search_text,
            replace_all_text,
            load_custom_font,
            gdrive_get_auth_url,
            gdrive_authenticate,
            gdrive_list_files,
            gdrive_open_file,
            gdrive_save_file
        ])
        .run(tauri::generate_context!())
        .expect("Lỗi khi khởi chạy W edit v0.1");
}