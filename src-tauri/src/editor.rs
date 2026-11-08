use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use regex::RegexBuilder;
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DocumentMetadata {
    pub id: String,
    pub title: String,
    pub file_path: Option<String>,
    pub gdrive_file_id: Option<String>,
    pub encoding: String,
    pub total_lines: usize,
    pub total_chars: usize,
    pub total_bytes: usize,
    pub is_modified: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchMatch {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub matched_text: String,
    pub line_preview: String,
}

pub struct DocumentSession {
    pub metadata: DocumentMetadata,
    pub rope: Rope,
    pub undo_stack: Vec<Rope>,
    pub redo_stack: Vec<Rope>,
}

impl DocumentSession {
    pub fn new_empty(title: &str) -> Self {
        let id = Uuid::new_v4().to_string();
        let rope = Rope::new();
        Self {
            metadata: DocumentMetadata {
                id,
                title: title.to_string(),
                file_path: None,
                gdrive_file_id: None,
                encoding: "UTF-8".to_string(),
                total_lines: 1,
                total_chars: 0,
                total_bytes: 0,
                is_modified: false,
            },
            rope,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn refresh_stats(&mut self) {
        self.metadata.total_lines = self.rope.len_lines();
        self.metadata.total_chars = self.rope.len_chars();
        self.metadata.total_bytes = self.rope.len_bytes();
    }

    pub fn push_undo(&mut self) {
        // 🛠️ [KHU VỰC DỄ CHỈNH SỬA 8]: Số lần Hoàn tác (Undo) tối đa (Mặc định: 100 bước)
        if self.undo_stack.len() >= 100 {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(self.rope.clone());
        self.redo_stack.clear();
    }
}

pub struct EditorManager {
    pub sessions: HashMap<String, DocumentSession>,
    pub active_tab_id: Option<String>,
}

impl EditorManager {
    pub fn new() -> Self {
        let mut manager = Self {
            sessions: HashMap::new(),
            active_tab_id: None,
        };
        let default_doc = DocumentSession::new_empty("Untitled-1.txt");
        let id = default_doc.metadata.id.clone();
        manager.sessions.insert(id.clone(), default_doc);
        manager.active_tab_id = Some(id);
        manager
    }

    pub fn open_local_file(&mut self, path_str: &str) -> Result<(DocumentMetadata, String), String> {
        let path = Path::new(path_str);
        let file = File::open(path).map_err(|e| format!("Không thể mở file: {}", e))?;
        let mut reader = BufReader::new(file);
        let mut raw_bytes = Vec::new();
        reader.read_to_end(&mut raw_bytes).map_err(|e| format!("Lỗi đọc file: {}", e))?;

        let mut detector = EncodingDetector::new();
        detector.feed(&raw_bytes, true);
        let encoding = detector.guess(None, true);

        let (decoded_cow, _, _) = encoding.decode(&raw_bytes);
        let content_str = decoded_cow.into_owned();
        let rope = Rope::from_str(&content_str);

        let title = path.file_name().and_then(|n| n.to_str()).unwrap_or("Unknown").to_string();
        let id = Uuid::new_v4().to_string();
        let mut session = DocumentSession {
            metadata: DocumentMetadata {
                id: id.clone(),
                title,
                file_path: Some(path_str.to_string()),
                gdrive_file_id: None,
                encoding: encoding.name().to_string(),
                total_lines: rope.len_lines(),
                total_chars: rope.len_chars(),
                total_bytes: rope.len_bytes(),
                is_modified: false,
            },
            rope,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        session.refresh_stats();

        let meta = session.metadata.clone();
        let initial_view = session.rope.to_string();
        self.sessions.insert(id.clone(), session);
        self.active_tab_id = Some(id);

        Ok((meta, initial_view))
    }

    pub fn load_from_gdrive(&mut self, title: String, gdrive_id: String, content: String) -> DocumentMetadata {
        let id = Uuid::new_v4().to_string();
        let rope = Rope::from_str(&content);
        let mut session = DocumentSession {
            metadata: DocumentMetadata {
                id: id.clone(),
                title,
                file_path: None,
                gdrive_file_id: Some(gdrive_id),
                encoding: "UTF-8".to_string(),
                total_lines: 0,
                total_chars: 0,
                total_bytes: 0,
                is_modified: false,
            },
            rope,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        };
        session.refresh_stats();
        let meta = session.metadata.clone();
        self.sessions.insert(id.clone(), session);
        self.active_tab_id = Some(id);
        meta
    }

    pub fn update_content(&mut self, tab_id: &str, new_content: &str) -> Result<DocumentMetadata, String> {
        let session = self.sessions.get_mut(tab_id).ok_or("Không tìm thấy Tab đang mở")?;
        session.push_undo();
        session.rope = Rope::from_str(new_content);
        session.metadata.is_modified = true;
        session.refresh_stats();
        Ok(session.metadata.clone())
    }

    pub fn save_local_file(&mut self, tab_id: &str, override_path: Option<String>) -> Result<DocumentMetadata, String> {
        let session = self.sessions.get_mut(tab_id).ok_or("Không tìm thấy Tab cần lưu")?;
        if let Some(new_path) = override_path {
            session.metadata.file_path = Some(new_path.clone());
            if let Some(name) = Path::new(&new_path).file_name().and_then(|n| n.to_str()) {
                session.metadata.title = name.to_string();
            }
        }

        let target_path = session.metadata.file_path.as_ref().ok_or("File chưa có đường dẫn lưu trữ")?;
        let full_text = session.rope.to_string();
        let encoding = Encoding::for_label(session.metadata.encoding.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        let (encoded_bytes, _, _) = encoding.encode(&full_text);

        let file = File::create(target_path).map_err(|e| format!("Không thể tạo file: {}", e))?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&encoded_bytes).map_err(|e| format!("Lỗi ghi file: {}", e))?;
        writer.flush().map_err(|e| format!("Lỗi lưu dữ liệu: {}", e))?;

        session.metadata.is_modified = false;
        session.refresh_stats();
        Ok(session.metadata.clone())
    }

    pub fn undo(&mut self, tab_id: &str) -> Result<(DocumentMetadata, String), String> {
        let session = self.sessions.get_mut(tab_id).ok_or("Tab không tồn tại")?;
        if let Some(prev_rope) = session.undo_stack.pop() {
            session.redo_stack.push(session.rope.clone());
            session.rope = prev_rope;
            session.refresh_stats();
            Ok((session.metadata.clone(), session.rope.to_string()))
        } else {
            Err("Không còn bước nào để Undo".to_string())
        }
    }

    pub fn redo(&mut self, tab_id: &str) -> Result<(DocumentMetadata, String), String> {
        let session = self.sessions.get_mut(tab_id).ok_or("Tab không tồn tại")?;
        if let Some(next_rope) = session.redo_stack.pop() {
            session.undo_stack.push(session.rope.clone());
            session.rope = next_rope;
            session.refresh_stats();
            Ok((session.metadata.clone(), session.rope.to_string()))
        } else {
            Err("Không còn bước nào để Redo".to_string())
        }
    }

    pub fn search_in_tab(
        &self,
        tab_id: &str,
        query: &str,
        case_sensitive: bool,
        use_regex: bool,
    ) -> Result<Vec<SearchMatch>, String> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.sessions.get(tab_id).ok_or("Tab không tồn tại")?;
        let pattern = if use_regex { query.to_string() } else { regex::escape(query) };

        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| format!("Regex không hợp lệ: {}", e))?;

        let mut results = Vec::new();
        for (line_idx, line_slice) in session.rope.lines().enumerate() {
            let line_str = line_slice.to_string();
            for mat in re.find_iter(&line_str) {
                results.push(SearchMatch {
                    line: line_idx + 1,
                    start_col: mat.start() + 1,
                    end_col: mat.end() + 1,
                    matched_text: mat.as_str().to_string(),
                    line_preview: line_str.trim_end().chars().take(120).collect(),
                });
                if results.len() >= 500 {
                    return Ok(results);
                }
            }
        }
        Ok(results)
    }

    pub fn replace_all_in_tab(
        &mut self,
        tab_id: &str,
        query: &str,
        replacement: &str,
        case_sensitive: bool,
        use_regex: bool,
    ) -> Result<(DocumentMetadata, String, usize), String> {
        let session = self.sessions.get_mut(tab_id).ok_or("Tab không tồn tại")?;
        let pattern = if use_regex { query.to_string() } else { regex::escape(query) };

        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| format!("Regex lỗi: {}", e))?;

        let full_text = session.rope.to_string();
        let count = re.find_iter(&full_text).count();

        if count > 0 {
            session.push_undo();
            let replaced = re.replace_all(&full_text, replacement).into_owned();
            session.rope = Rope::from_str(&replaced);
            session.metadata.is_modified = true;
            session.refresh_stats();
            Ok((session.metadata.clone(), replaced, count))
        } else {
            Ok((session.metadata.clone(), full_text, 0))
        }
    }
}