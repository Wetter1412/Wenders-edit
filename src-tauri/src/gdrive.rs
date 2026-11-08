use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GDriveFile {
    pub id: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "modifiedTime")]
    pub modified_time: Option<String>,
    pub size: Option<String>,
}

#[derive(Deserialize)]
struct FileListResponse {
    files: Vec<GDriveFile>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
}

pub struct GoogleDriveClient {
    http: Client,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_verifier: Option<String>,
    pub token: Option<OAuthToken>,
}

impl GoogleDriveClient {
    pub fn new(client_id: &str, redirect_uri: &str) -> Self {
        Self {
            http: Client::new(),
            client_id: client_id.to_string(),
            redirect_uri: redirect_uri.to_string(),
            code_verifier: None,
            token: None,
        }
    }

    pub fn generate_auth_url(&mut self) -> String {
        let mut random_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        self.code_verifier = Some(verifier);

        let scope = urlencoding::encode("https://www.googleapis.com/auth/drive");
        let redirect = urlencoding::encode(&self.redirect_uri);

        format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
            self.client_id, redirect, scope, challenge
        )
    }

    pub async fn exchange_code_for_token(&mut self, auth_code: &str) -> Result<OAuthToken, String> {
        let verifier = self.code_verifier.as_ref().ok_or("Chưa khởi tạo phiên đăng nhập PKCE")?;

        let params = [
            ("client_id", self.client_id.as_str()),
            ("code", auth_code),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", self.redirect_uri.as_str()),
        ];

        let resp = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Lỗi kết nối Google OAuth: {}", e))?;

        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(format!("Xác thực thất bại: {}", err_body));
        }

        let token_data: OAuthToken = resp.json().await.map_err(|e| format!("Lỗi đọc Token: {}", e))?;
        self.token = Some(token_data.clone());
        Ok(token_data)
    }

    fn bearer_token(&self) -> Result<&str, String> {
        self.token
            .as_ref()
            .map(|t| t.access_token.as_str())
            .ok_or_else(|| "Bạn chưa đăng nhập Google Drive".to_string())
    }

    pub async fn list_files(&self, folder_id: Option<&str>) -> Result<Vec<GDriveFile>, String> {
        let token = self.bearer_token()?;
        let parent = folder_id.unwrap_or("root");
        let query = format!("'{}' in parents and trashed = false", parent);

        let url = format!(
            "https://www.googleapis.com/drive/v3/files?q={}&fields=files(id,name,mimeType,modifiedTime,size)&orderBy=folder,name&pageSize=100",
            urlencoding::encode(&query)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("Lỗi gọi Google Drive API: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Google Drive trả về mã lỗi: {}", resp.status()));
        }

        let list: FileListResponse = resp.json().await.map_err(|e| format!("Lỗi đọc danh sách file: {}", e))?;
        Ok(list.files)
    }

    pub async fn download_file_content(&self, file_id: &str) -> Result<String, String> {
        let token = self.bearer_token()?;
        let url = format!("https://www.googleapis.com/drive/v3/files/{}?alt=media", file_id);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("Lỗi tải file: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Không thể tải nội dung file (Mã lỗi: {})", resp.status()));
        }

        resp.text().await.map_err(|e| format!("Lỗi giải mã văn bản: {}", e))
    }

    pub async fn update_existing_file(&self, file_id: &str, content: &str) -> Result<GDriveFile, String> {
        let token = self.bearer_token()?;
        let url = format!(
            "https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media&fields=id,name,mimeType,modifiedTime,size",
            file_id
        );

        let resp = self
            .http
            .patch(&url)
            .bearer_auth(token)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(content.to_string())
            .send()
            .await
            .map_err(|e| format!("Lỗi đồng bộ lên Google Drive: {}", e))?;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(format!("Lưu thất bại: {}", err_text));
        }

        resp.json::<GDriveFile>().await.map_err(|e| format!("Lỗi đọc phản hồi: {}", e))
    }

    pub async fn create_new_file(
        &self,
        name: &str,
        content: &str,
        parent_folder_id: Option<&str>,
    ) -> Result<GDriveFile, String> {
        let token = self.bearer_token()?;
        let boundary = "wedit_multipart_boundary_2026";
        let parent = parent_folder_id.unwrap_or("root");

        let metadata_json = serde_json::json!({
            "name": name,
            "mimeType": "text/plain",
            "parents": [parent]
        });

        let body = format!(
            "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{b}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{content}\r\n--{b}--",
            b = boundary,
            meta = metadata_json.to_string(),
            content = content
        );

        let url = "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id,name,mimeType,modifiedTime,size";

        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("Content-Type", format!("multipart/related; boundary={}", boundary))
            .body(body)
            .send()
            .await
            .map_err(|e| format!("Lỗi tạo file mới trên Drive: {}", e))?;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(format!("Không thể tạo file trên Drive: {}", err_text));
        }

        resp.json::<GDriveFile>().await.map_err(|e| format!("Lỗi phân tích phản hồi Drive: {}", e))
    }
}