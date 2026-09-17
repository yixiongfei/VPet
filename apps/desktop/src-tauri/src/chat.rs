//! Local conversation is a transcript, not long-term memory. Only the existing
//! explicit memory-command path writes reusable memories (design document §3).
//! Feedback records examples; exporting examples does not train model weights.

use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use reqwest::{Client, Url};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::watch;

const MAX_USER_CHARS: usize = 4_000;
const MAX_CONTEXT_CHARS: usize = 10_000;
const MAX_OUTPUT_BYTES: usize = 48_000;
const MAX_WIRE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_HISTORY_MESSAGES: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Persona {
    pub name: String,
    pub background: String,
    pub appearance: String,
    pub personality: String,
    pub speaking_style: String,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            name: "萝莉斯".into(),
            background: "住在用户桌面上的伙伴，陪伴日常生活、学习和工作，有自己的喜好与小脾气。".into(),
            appearance: "与桌面立绘一致：银灰色长发、头顶一撮呆毛，金黄色的眼睛，穿着舒适可爱的日常装扮。".into(),
            personality: "温柔、好奇、坦率，亲近但有边界。认真倾听，不一味附和。".into(),
            speaking_style: "用自然的简体中文聊天，通常两到四句话。像熟悉的朋友，不用客服套话；偶尔有轻巧的动作描写，不每句都撒娇。".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatSettings {
    pub model: String,
    pub endpoint: String,
    pub temperature: f32,
    pub persona: Persona,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            // 9B 在 Arc 核显（Vulkan）上约 8 tok/s、首字 2 s；想更快可在设置里换 qwen3:4b（18 tok/s）
            model: "qwen3.5:9b".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            temperature: 0.75,
            persona: Persona::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub status: String,
    pub rating: Option<String>,
    pub corrected_text: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ModelMessage {
    role: String,
    content: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamEvent {
    request_id: String,
    delta: String,
    done: bool,
    /// 收尾清理后的最终文本。只在成功结束的那条 done 事件里带，气泡用它替换流式内容
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Serialize)]
pub struct ModelStatus {
    connected: bool,
    models: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct TrainingExport {
    path: String,
    samples: usize,
}

struct ActiveRequest {
    id: String,
    cancel: watch::Sender<bool>,
}

pub struct ChatState {
    store: Mutex<ChatStore>,
    active: Mutex<Option<ActiveRequest>>,
    client: Client,
    exports_dir: PathBuf,
}

struct ChatStore {
    conn: Connection,
}

impl ChatStore {
    fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(db_error)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, String> {
        conn.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS chat_settings (
                 id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chat_messages (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 id TEXT NOT NULL UNIQUE,
                 role TEXT NOT NULL CHECK(role IN ('user','assistant')),
                 content TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 status TEXT NOT NULL,
                 rating TEXT CHECK(rating IS NULL OR rating IN ('up','down')),
                 corrected_text TEXT,
                 source TEXT NOT NULL,
                 prompt_json TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_chat_feedback ON chat_messages(rating);",
        ).map_err(db_error)?;
        Ok(Self { conn })
    }

    fn settings(&self) -> Result<ChatSettings, String> {
        let value: Option<String> = self.conn.query_row(
            "SELECT value FROM chat_settings WHERE id=1", [], |r| r.get(0),
        ).optional().map_err(db_error)?;
        match value {
            Some(value) => serde_json::from_str(&value)
                .map_err(|e| format!("对话设置损坏，请重新保存设置：{e}")),
            None => Ok(ChatSettings::default()),
        }
    }

    fn save_settings(&self, settings: &ChatSettings) -> Result<(), String> {
        let json = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        self.conn.execute(
            "INSERT INTO chat_settings(id,value) VALUES(1,?1)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value", [json],
        ).map_err(db_error)?;
        Ok(())
    }

    fn messages(&self, limit: usize) -> Result<Vec<ChatMessage>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT id,role,content,created_at,status,rating,corrected_text,source
             FROM (SELECT * FROM chat_messages ORDER BY seq DESC LIMIT ?1) ORDER BY seq",
        ).map_err(db_error)?;
        let rows = stmt.query_map([limit as i64], row_message).map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
    }

    fn insert(&self, message: &ChatMessage, prompt: Option<&[ModelMessage]>) -> Result<(), String> {
        let prompt_json = prompt.map(serde_json::to_string).transpose().map_err(|e| e.to_string())?;
        self.conn.execute(
            "INSERT INTO chat_messages(id,role,content,created_at,status,rating,corrected_text,source,prompt_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![message.id, message.role, message.content, message.created_at, message.status,
                message.rating, message.corrected_text, message.source, prompt_json],
        ).map_err(db_error)?;
        Ok(())
    }

    fn rate(&self, id: &str, rating: Option<String>, correction: Option<String>) -> Result<ChatMessage, String> {
        if rating.as_deref().is_some_and(|s| !matches!(s, "up" | "down")) {
            return Err("反馈只接受 up、down 或清除评价。".into());
        }
        let correction = correction.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if correction.as_ref().is_some_and(|s| s.chars().count() > MAX_USER_CHARS) {
            return Err("修订内容最多 4000 字。".into());
        }
        let count = self.conn.execute(
            "UPDATE chat_messages SET rating=?2, corrected_text=?3
             WHERE id=?1 AND role='assistant' AND status='complete' AND source='model'",
            params![id, rating, correction],
        ).map_err(db_error)?;
        if count == 0 {
            return Err("只有模型已完成的回答可以评价或修订。".into());
        }
        self.conn.query_row(
            "SELECT id,role,content,created_at,status,rating,corrected_text,source FROM chat_messages WHERE id=?1",
            [id], row_message,
        ).map_err(db_error)
    }

    /// Snapshots retain the persona, memory context and conversation actually sent
    /// with this answer. Changing today's persona never relabels old examples.
    fn training_samples(&self) -> Result<Vec<serde_json::Value>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT content,corrected_text,prompt_json FROM chat_messages
             WHERE role='assistant' AND source='model' AND status='complete'
               AND (rating='up' OR (corrected_text IS NOT NULL AND length(trim(corrected_text))>0))
               AND prompt_json IS NOT NULL ORDER BY seq",
        ).map_err(db_error)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))
        }).map_err(db_error)?;
        let mut samples = Vec::new();
        for row in rows {
            let (original, correction, prompt) = row.map_err(db_error)?;
            let mut messages: Vec<ModelMessage> = serde_json::from_str(&prompt)
                .map_err(|e| format!("训练样本上下文损坏：{e}"))?;
            let content = correction.unwrap_or(original);
            if content.trim().is_empty() || messages.first().map(|m| m.role.as_str()) != Some("system") {
                continue;
            }
            // 快照末尾是预填充的「名字：」（见 wire_messages）：把回答接在它后面，
            // 训练样本和推理时模型看到的东西完全一致。老快照以 user 结尾，直接追加
            match messages.last_mut() {
                Some(last) if last.role == "assistant" => last.content.push_str(&content),
                Some(last) if last.role == "user" => messages.push(ModelMessage { role: "assistant".into(), content }),
                _ => continue,
            }
            samples.push(serde_json::json!({"messages": messages}));
        }
        Ok(samples)
    }
}

fn row_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    Ok(ChatMessage {
        id: row.get(0)?, role: row.get(1)?, content: row.get(2)?, created_at: row.get(3)?,
        status: row.get(4)?, rating: row.get(5)?, corrected_text: row.get(6)?, source: row.get(7)?,
    })
}

fn db_error(error: rusqlite::Error) -> String { format!("对话数据库操作失败：{error}") }

fn chat_state(app: &AppHandle) -> Result<State<'_, ChatState>, String> {
    app.try_state::<ChatState>().ok_or_else(|| "对话存储初始化失败，请检查数据目录权限并重启。".into())
}

fn with_store<T>(app: &AppHandle, f: impl FnOnce(&ChatStore) -> Result<T, String>) -> Result<T, String> {
    let state = chat_state(app)?;
    let store = state.store.lock().map_err(|_| "对话数据库锁不可用。".to_string())?;
    f(&store)
}

pub fn init(app: &AppHandle) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let state = ChatState {
        store: Mutex::new(ChatStore::open(&dir.join("chat.sqlite3"))?),
        active: Mutex::new(None),
        client: Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(180))
            .build().map_err(|e| e.to_string())?,
        exports_dir: dir.join("training"),
    };
    app.manage(state);
    Ok(())
}

/// A local-only URL prevents accidental disclosure to a remote server. Convert
/// localhost to a literal loopback address so DNS/proxy configuration cannot
/// silently redirect conversation data. Reject redirects at the client too.
fn local_endpoint(value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim()).map_err(|_| "请输入完整的本地 Ollama 地址。".to_string())?;
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    let local = host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !local || url.scheme() != "http" || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        return Err("仅支持本机 Ollama HTTP 地址，例如 http://127.0.0.1:11434；不接受远程地址、路径或账号。".into());
    }
    if host == "localhost" { url.set_host(Some("127.0.0.1")).map_err(|e| e.to_string())?; }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn validate_settings(mut settings: ChatSettings) -> Result<ChatSettings, String> {
    settings.endpoint = local_endpoint(&settings.endpoint)?;
    settings.model = settings.model.trim().to_string();
    if settings.model.is_empty() || settings.model.len() > 128
        || !settings.model.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c)) {
        return Err("模型名称无效，请使用 Ollama 中已安装的模型名。".into());
    }
    // Ollama can itself route cloud model names; those aren't local inference.
    if settings.model.to_ascii_lowercase().contains("cloud") {
        return Err("此版本只使用本地模型，不支持 Ollama cloud 模型。".into());
    }
    if !settings.temperature.is_finite() || !(0.0..=1.5).contains(&settings.temperature) {
        return Err("温度应在 0 到 1.5 之间。".into());
    }
    settings.persona.name = settings.persona.name.trim().into();
    if settings.persona.name.is_empty() || settings.persona.name.chars().count() > 40 {
        return Err("角色名字应为 1–40 字。".into());
    }
    for (label, text) in [
        ("背景", &settings.persona.background), ("形象", &settings.persona.appearance),
        ("性格", &settings.persona.personality), ("说话方式", &settings.persona.speaking_style),
    ] {
        if text.chars().count() > 1_000 { return Err(format!("{label}最多 1000 字。")); }
    }
    Ok(settings)
}

/// 语义向量走同一个 Ollama。设置还没初始化 / 读不出来就用默认本机地址
pub fn embedding_endpoint(app: &AppHandle) -> String {
    with_store(app, ChatStore::settings)
        .and_then(|s| local_endpoint(&s.endpoint))
        .unwrap_or_else(|_| local_endpoint(&ChatSettings::default().endpoint).expect("默认地址合法"))
}

#[tauri::command]
pub fn get_chat_settings(app: AppHandle) -> Result<ChatSettings, String> {
    with_store(&app, ChatStore::settings)
}

#[tauri::command]
pub fn save_chat_settings(app: AppHandle, settings: ChatSettings) -> Result<ChatSettings, String> {
    let settings = validate_settings(settings)?;
    with_store(&app, |store| store.save_settings(&settings))?;
    let _ = app.emit("chat:settings-changed", &settings);
    Ok(settings)
}

#[tauri::command]
pub fn list_chat_messages(app: AppHandle) -> Result<Vec<ChatMessage>, String> {
    with_store(&app, |store| store.messages(MAX_HISTORY_MESSAGES))
}

#[tauri::command]
pub fn rate_chat_message(app: AppHandle, message_id: String, rating: Option<String>, corrected_text: Option<String>) -> Result<ChatMessage, String> {
    with_store(&app, |store| store.rate(&message_id, rating, corrected_text))
}

#[tauri::command]
pub fn export_training_data(app: AppHandle) -> Result<TrainingExport, String> {
    let samples = with_store(&app, ChatStore::training_samples)?;
    if samples.is_empty() { return Err("还没有可导出的样本。请先赞同或修订至少一条完整的模型回答。".into()); }
    let state = chat_state(&app)?;
    fs::create_dir_all(&state.exports_dir).map_err(|e| format!("无法创建训练目录：{e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let mut output = None;
    for suffix in 0..100 {
        let path = state.exports_dir.join(format!("vpet-feedback-{stamp}-{suffix}.jsonl"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => { output = Some((path, file)); break; }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("无法写入训练文件：{e}")),
        }
    }
    let (path, file) = output.ok_or_else(|| "无法生成唯一的训练文件名。".to_string())?;
    let mut writer = BufWriter::new(file);
    for sample in &samples {
        serde_json::to_writer(&mut writer, sample).map_err(|e| e.to_string())?;
        writer.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;
    Ok(TrainingExport { path: path.to_string_lossy().into_owned(), samples: samples.len() })
}

fn system_prompt(settings: &ChatSettings, memories: &str) -> String {
    let p = &settings.persona;
    // 角色卡 + 短规则。4B 模型对长篇分析式指令会「边想边说」，
    // 真正压住它的是 wire_messages 里的预填充；这里只负责把设定说清楚
    let mut prompt = format!(
        "# 角色卡\n名字：{}\n背景：{}\n形象：{}\n性格：{}\n说话方式：{}\n\n\
         # 规则\n\
         - 你就是{}，住在用户的电脑桌面上，直接和用户说话。输出只包含{}说出口的话，不要分析、不要计划、不要解释自己在做什么。\n\
         - 回应用户刚才具体说的内容，不复述设定。可以有自己的观点和感受。\n\
         - 不编造现实经历、已执行的操作或你不知道的用户信息。你只能聊天，不能打开软件、发消息、设提醒或训练自己。\n\
         - 被问到身份时坦率说自己是虚拟伙伴。没有明确的记忆命令时，不要声称已经永久记住了什么。\n\
         - 形象描述只用于对话，桌面立绘不会因此改变。",
        p.name, p.background, p.appearance, p.personality, p.speaking_style, p.name, p.name,
    );
    if !memories.is_empty() { prompt.push_str("\n\n"); prompt.push_str(memories); }
    prompt
}

/// 发给模型的最终消息：逻辑提示词 + 预填充的「名字：」。
/// Qwen3 4B 在 think=false 下几乎必然把推理过程当正文吐出来；让它从角色名后面接着写，
/// 它就直接开口了（实测 100% 进入角色，且回答短得多）。训练快照也存这份，保证一致
fn wire_messages(prompt: &[ModelMessage], name: &str) -> Vec<ModelMessage> {
    let mut wire = prompt.to_vec();
    wire.push(ModelMessage { role: "assistant".into(), content: reply_prefix(name) });
    wire
}

fn reply_prefix(name: &str) -> String { format!("{name}：") }

/// 模型偶尔会在说完话之后拖出一段元注释（「（注意：…）」「我需要…」）或 `<think>`。
/// 逐行扫，碰到第一行像旁白的就从那里截断；名字前缀重复了也去掉
fn clean_reply(raw: &str, name: &str) -> String {
    let mut text = raw.trim_start();
    for prefix in [reply_prefix(name), format!("{name}:")] {
        if let Some(rest) = text.strip_prefix(prefix.as_str()) { text = rest.trim_start(); }
    }
    let mut kept = String::new();
    for line in text.lines() {
        if looks_like_aside(line.trim()) { break; }
        if !kept.is_empty() { kept.push('\n'); }
        kept.push_str(line);
    }
    kept.trim().to_string()
}

/// 这一行是模型对自己说的话，不是角色说的话？
/// 两种形态：直接以分析口吻开头；或整行套在括号里、内容却是写作指令
/// （「（保持自然的语气）」）——真正的动作描写（「（歪头笑）」）不会含这些词
fn looks_like_aside(line: &str) -> bool {
    const OPENERS: [&str; 16] = [
        "<think>", "</think>", "（注意", "(注意", "注意：", "注：", "我需要", "首先", "用户说", "用户问",
        "用户刚才", "根据角色", "关键点", "思考：", "分析：", "回应要",
    ];
    const DIRECTIVES: [&str; 18] = [
        "保持", "避免", "确保", "注意", "语气", "句式", "描写", "设定", "角色", "用户", "回应",
        "口语", "字数", "句话", "不要", "需要", "应该", "符合",
    ];
    if OPENERS.iter().any(|m| line.starts_with(m)) { return true; }
    let parenthesized = (line.starts_with('（') && line.ends_with('）')) || (line.starts_with('(') && line.ends_with(')'));
    parenthesized && DIRECTIVES.iter().any(|w| line.contains(w))
}

/// Include only complete pairs, excluding failed/cancelled/rejected answers.
/// A user's correction replaces the earlier answer for future context as well.
fn build_prompt(settings: &ChatSettings, memories: &str, history: &[ChatMessage], text: &str) -> Vec<ModelMessage> {
    let mut pairs: Vec<(ModelMessage, ModelMessage)> = Vec::new();
    let mut user: Option<&ChatMessage> = None;
    for message in history {
        if message.role == "user" { user = Some(message); continue; }
        let Some(previous) = user.take() else { continue };
        if message.status != "complete" || (message.rating.as_deref() == Some("down") && message.corrected_text.is_none()) { continue; }
        pairs.push((
            ModelMessage { role: "user".into(), content: previous.content.clone() },
            ModelMessage { role: "assistant".into(), content: message.corrected_text.clone().unwrap_or_else(|| message.content.clone()) },
        ));
    }
    let mut used = 0;
    let mut recent = Vec::new();
    for pair in pairs.into_iter().rev().take(12) {
        let count = pair.0.content.chars().count() + pair.1.content.chars().count();
        if used + count > MAX_CONTEXT_CHARS { break; }
        used += count;
        recent.push(pair);
    }
    let mut result = vec![ModelMessage { role: "system".into(), content: system_prompt(settings, memories) }];
    for (user, assistant) in recent.into_iter().rev() { result.push(user); result.push(assistant); }
    result.push(ModelMessage { role: "user".into(), content: text.into() });
    result
}

fn message(id: String, role: &str, content: String, source: &str, status: &str) -> ChatMessage {
    ChatMessage {
        id, role: role.into(), content, created_at: crate::now_ms(), status: status.into(),
        rating: None, corrected_text: None, source: source.into(),
    }
}

struct RequestGuard { app: AppHandle, id: String }

impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(state) = self.app.try_state::<ChatState>() {
            if let Ok(mut active) = state.active.lock() {
                if active.as_ref().is_some_and(|a| a.id == self.id) { *active = None; }
            }
        }
        let _ = self.app.emit("chat-stream", StreamEvent { request_id: self.id.clone(), delta: String::new(), done: true, text: None });
    }
}

#[tauri::command]
pub fn cancel_chat(app: AppHandle, request_id: String) -> Result<(), String> {
    let state = chat_state(&app)?;
    let active = state.active.lock().map_err(|_| "对话状态锁不可用。".to_string())?;
    if let Some(active) = active.as_ref().filter(|a| a.id == request_id) {
        let _ = active.cancel.send(true);
    }
    Ok(())
}

#[tauri::command]
pub async fn send_chat_message(app: AppHandle, text: String, request_id: String) -> Result<ChatMessage, String> {
    let text = text.trim().to_string();
    if text.is_empty() || text.chars().count() > MAX_USER_CHARS { return Err("消息应为 1–4000 字。".into()); }
    if request_id.is_empty() || request_id.len() > 100 || !request_id.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)) {
        return Err("对话请求编号无效。".into());
    }
    let (cancel, mut cancelled) = watch::channel(false);
    {
        let state = chat_state(&app)?;
        let mut active = state.active.lock().map_err(|_| "对话状态锁不可用。".to_string())?;
        if active.is_some() { return Err("上一条回答还在生成，请等待或先停止生成。".into()); }
        *active = Some(ActiveRequest { id: request_id.clone(), cancel });
    }
    let _guard = RequestGuard { app: app.clone(), id: request_id.clone() };
    let (settings, history) = with_store(&app, |store| {
        let settings = validate_settings(store.settings()?)?;
        let history = store.messages(50)?;
        store.insert(&message(format!("{request_id}-user"), "user", text.clone(), "model", "complete"), None)?;
        Ok((settings, history))
    })?;
    // This deterministic command path must run before ordinary model inference.
    if let Some(reply) = crate::memory_command(app.clone(), text.clone()) {
        let response = message(format!("{request_id}-assistant"), "assistant", reply, "memory", "complete");
        with_store(&app, |store| store.insert(&response, None))?;
        let _ = app.emit("chat-stream", StreamEvent { request_id, delta: response.content.clone(), done: false, text: None });
        return Ok(response);
    }
    let memories = crate::memory_context(app.clone(), text.clone()).await;
    let prompt = wire_messages(&build_prompt(&settings, &memories, &history, &text), &settings.persona.name);
    let client = chat_state(&app)?.client.clone();
    let mut output = String::new();
    let result = {
        let generation = generate(&client, &settings, &prompt, &mut output, |delta| {
            let _ = app.emit("chat-stream", StreamEvent { request_id: request_id.clone(), delta: delta.into(), done: false, text: None });
        });
        tokio::select! {
            biased;
            _ = cancelled.changed() => None,
            result = generation => Some(result),
        }
    };
    let (status, mut error) = match result {
        None => ("cancelled", None),
        Some(Ok(())) => ("complete", None),
        Some(Err(error)) => ("error", Some(error)),
    };
    let mut output = clean_reply(&output, &settings.persona.name);
    if status == "complete" && output.is_empty() {
        error = Some("模型这次没有进入角色，请再试一次或换个说法。".into());
    }
    let status = if error.is_some() { "error" } else { status };
    if let Some(error) = &error {
        if !output.is_empty() { output.push_str("\n\n"); }
        output.push_str(&format!("[生成失败] {error}"));
    }
    let response = message(format!("{request_id}-assistant"), "assistant", output, "model", status);
    with_store(&app, |store| store.insert(&response, Some(&prompt)))?;
    if let Some(error) = error { return Err(error); }
    let _ = app.emit("chat-stream", StreamEvent { request_id, delta: String::new(), done: true, text: Some(response.content.clone()) });
    Ok(response)
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_connect() { "无法连接本地 Ollama。请启动 Ollama，再确认设置中的地址和端口。".into() }
    else if error.is_timeout() { "本地模型响应超时（180 秒）。可尝试较小模型，或缩短问题。".into() }
    else { format!("本地模型请求失败：{error}") }
}

async fn limited_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > limit { return Err("本地模型服务返回的数据过大。".into()); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct OllamaChunk {
    #[serde(default)] message: Option<OllamaMessage>,
    #[serde(default)] done: bool,
    #[serde(default)] error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaMessage { #[serde(default)] content: String }

/// Decode whole NDJSON lines, not network chunks: a UTF-8 character and a JSON
/// record can both cross arbitrary TCP boundaries.
fn consume_line(line: &[u8], output: &mut String, on_delta: &mut impl FnMut(&str)) -> Result<bool, String> {
    if line.iter().all(u8::is_ascii_whitespace) { return Ok(false); }
    let data: OllamaChunk = serde_json::from_slice(line).map_err(|e| format!("Ollama 流格式无效：{e}"))?;
    if let Some(error) = data.error { return Err(format!("Ollama：{}", error.chars().take(500).collect::<String>())); }
    if let Some(message) = data.message {
        if output.len() + message.content.len() > MAX_OUTPUT_BYTES { return Err("回答达到长度上限，已停止生成。".into()); }
        if !message.content.is_empty() { output.push_str(&message.content); on_delta(&message.content); }
    }
    Ok(data.done)
}

async fn generate(client: &Client, settings: &ChatSettings, prompt: &[ModelMessage], output: &mut String, mut on_delta: impl FnMut(&str)) -> Result<(), String> {
    let mut response = client.post(format!("{}/api/chat", local_endpoint(&settings.endpoint)?))
        .json(&serde_json::json!({
            "model": settings.model, "messages": prompt, "stream": true, "think": false,
            "keep_alive": "10m",
            // 说完就停：别替用户接话，也别在正文后面开一段 <think>
            "options": { "temperature": settings.temperature, "num_ctx": 8192, "num_predict": 1024,
                         "stop": ["用户：", "\n用户", "<think>", "</think>"] }
        })).send().await.map_err(network_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let body = limited_body(response, 8_192).await?;
        let detail: String = String::from_utf8_lossy(&body).chars().take(500).collect();
        return Err(format!("Ollama 返回 {status}：{detail}。请确认模型 {} 已下载。", settings.model));
    }
    let mut pending = Vec::new();
    let mut total = 0;
    let mut done = false;
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        total += chunk.len();
        if total > MAX_WIRE_BYTES { return Err("本地模型响应超过安全长度上限。".into()); }
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            if end > MAX_LINE_BYTES { return Err("本地模型流的单行过大。".into()); }
            done = consume_line(&pending[..end], output, &mut on_delta)?;
            pending.drain(..=end);
            if done { break; }
        }
        if done { break; }
        if pending.len() > MAX_LINE_BYTES { return Err("本地模型流的单行过大。".into()); }
    }
    if !done && !pending.is_empty() { done = consume_line(&pending, output, &mut on_delta)?; }
    if !done { return Err("本地模型连接提前结束，回答未完成。".into()); }
    if output.trim().is_empty() { return Err("本地模型返回了空回答，请重试或切换模型。".into()); }
    Ok(())
}

#[tauri::command]
pub async fn get_model_status(app: AppHandle) -> Result<ModelStatus, String> {
    let settings = with_store(&app, ChatStore::settings)?;
    let endpoint = local_endpoint(&settings.endpoint)?;
    let client = chat_state(&app)?.client.clone();
    let result = async {
        let response = client.get(format!("{endpoint}/api/tags")).timeout(Duration::from_secs(5))
            .send().await.map_err(network_error)?;
        if !response.status().is_success() { return Err(format!("Ollama 返回 {}", response.status())); }
        let body = limited_body(response, 1_048_576).await?;
        let data: serde_json::Value = serde_json::from_slice(&body).map_err(|e| format!("Ollama 模型列表无效：{e}"))?;
        let models = data.get("models").and_then(|m| m.as_array()).ok_or("Ollama 未返回模型列表。")?
            .iter().filter_map(|m| m.get("name").and_then(|v| v.as_str()).map(str::to_owned))
            .filter(|name| !name.to_ascii_lowercase().contains("cloud")).collect::<Vec<_>>();
        Ok::<_, String>(models)
    }.await;
    Ok(match result {
        Ok(models) => ModelStatus { connected: true, models, error: None },
        Err(error) => ModelStatus { connected: false, models: Vec::new(), error: Some(error) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ChatStore { ChatStore::from_connection(Connection::open_in_memory().unwrap()).unwrap() }

    #[test]
    fn only_local_endpoint_without_paths_credentials_or_redirect_targets() {
        assert_eq!(local_endpoint("http://localhost:11434/").unwrap(), "http://127.0.0.1:11434");
        assert!(local_endpoint("http://[::1]:11434").is_ok());
        for value in ["https://api.example.com", "http://192.168.1.2:11434", "http://localhost.evil.test:11434", "http://127.0.0.1/api/chat", "http://name:pass@127.0.0.1:11434", "http://127.0.0.1:11434?x=y", "http://127.0.0.1:11434#x"] {
            assert!(local_endpoint(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn settings_validate_and_persist() {
        let db = store();
        let mut settings = ChatSettings::default();
        settings.persona.name = "小桃".into();
        db.save_settings(&validate_settings(settings).unwrap()).unwrap();
        assert_eq!(db.settings().unwrap().persona.name, "小桃");
        let mut invalid = ChatSettings::default();
        invalid.temperature = f32::NAN;
        assert!(validate_settings(invalid).is_err());
        let mut remote = ChatSettings::default();
        remote.model = "qwen3:cloud".into();
        assert!(validate_settings(remote).is_err());
    }

    #[test]
    fn export_only_confirmed_or_corrected_complete_model_answers_and_snapshot() {
        let db = store();
        let prompt = wire_messages(&build_prompt(&ChatSettings::default(), "[记忆上下文]", &[], "今天好吗？"), "萝莉斯");
        for (id, source, status) in [("positive", "model", "complete"), ("negative", "model", "complete"), ("corrected", "model", "complete"), ("unrated", "model", "complete"), ("memory", "memory", "complete"), ("cancel", "model", "cancelled"), ("failed", "model", "error")] {
            db.insert(&message(id.into(), "assistant", format!("{id} 原回答"), source, status), Some(&prompt)).unwrap();
        }
        assert!(db.training_samples().unwrap().is_empty());
        db.rate("positive", Some("up".into()), None).unwrap();
        db.rate("negative", Some("down".into()), None).unwrap();
        db.rate("corrected", Some("down".into()), Some("用户的正确回答".into())).unwrap();
        assert!(db.rate("memory", Some("up".into()), None).is_err());
        assert!(db.rate("cancel", Some("up".into()), None).is_err());
        assert!(db.rate("failed", Some("up".into()), None).is_err());
        let samples = db.training_samples().unwrap();
        assert_eq!(samples.len(), 2);
        assert!(samples[0]["messages"][0]["content"].as_str().unwrap().contains("[记忆上下文]"));
        assert_eq!(samples[1]["messages"][2]["content"], "萝莉斯：用户的正确回答");
        assert_eq!(samples[0]["messages"].as_array().unwrap().len(), 3);
        let mut revised = ChatSettings::default();
        revised.persona.name = "新角色".into();
        db.save_settings(&revised).unwrap();
        assert!(db.training_samples().unwrap()[0]["messages"][0]["content"].as_str().unwrap().contains("萝莉斯"));
    }

    #[test]
    fn transcripts_are_separate_from_memory_and_failures_do_not_enter_context() {
        let db = store();
        let mut history = vec![
            message("u1".into(), "user", "old question".into(), "model", "complete"),
            message("a1".into(), "assistant", "bad reply".into(), "model", "error"),
            message("u2".into(), "user", "good question".into(), "model", "complete"),
            message("a2".into(), "assistant", "old answer".into(), "model", "complete"),
        ];
        history[3].corrected_text = Some("correct answer".into());
        for m in &history { db.insert(m, None).unwrap(); }
        let prompt = build_prompt(&ChatSettings::default(), "", &db.messages(100).unwrap(), "new");
        assert_eq!(prompt.len(), 4);
        assert_eq!(prompt[1].content, "good question");
        assert_eq!(prompt[2].content, "correct answer");
        let memories: i64 = db.conn.query_row("SELECT count(*) FROM sqlite_master WHERE name='memory_items'", [], |r| r.get(0)).unwrap();
        assert_eq!(memories, 0);
    }

    #[test]
    fn history_is_bounded_and_pairs_stay_intact() {
        let mut history = Vec::new();
        for i in 0..100 {
            history.push(message(format!("u{i}"), "user", "问".repeat(600), "model", "complete"));
            history.push(message(format!("a{i}"), "assistant", "答".repeat(600), "model", "complete"));
        }
        let prompt = build_prompt(&ChatSettings::default(), "", &history, "现在");
        assert_eq!(prompt.len() % 2, 0);
        assert!(prompt[1..prompt.len()-1].iter().map(|m| m.content.chars().count()).sum::<usize>() <= MAX_CONTEXT_CHARS);
        assert_eq!(prompt.last().unwrap().role, "user");
    }

    #[test]
    fn stream_ignores_thinking_preserves_chinese_and_requires_valid_json() {
        let mut output = String::new();
        let mut deltas = Vec::new();
        let mut append = |delta: &str| deltas.push(delta.to_string());
        assert!(!consume_line(br#"{"message":{"thinking":"private reasoning"},"done":false}"#, &mut output, &mut append).unwrap());
        assert!(!consume_line("{\"message\":{\"content\":\"你好\"},\"done\":false}".as_bytes(), &mut output, &mut append).unwrap());
        assert!(consume_line(br#"{"message":{"content":"!"},"done":true}"#, &mut output, &mut append).unwrap());
        assert_eq!(output, "你好!");
        assert_eq!(deltas, ["你好", "!"]);
        assert!(consume_line(b"invalid", &mut output, &mut |_| {}).is_err());
        assert!(consume_line(br#"{"error":"model missing"}"#, &mut output, &mut |_| {}).unwrap_err().contains("model missing"));
    }

    #[test]
    fn wire_messages_prefill_and_reply_cleanup() {
        let wire = wire_messages(&build_prompt(&ChatSettings::default(), "", &[], "在吗"), "萝莉斯");
        assert_eq!(wire.last().unwrap().role, "assistant");
        assert_eq!(wire.last().unwrap().content, "萝莉斯：");
        assert_eq!(clean_reply("萝莉斯：（歪头）在呀。\n今天怎么样？", "萝莉斯"), "（歪头）在呀。\n今天怎么样？");
        assert_eq!(clean_reply("在呀。\n\n（注意：保持自然对话）\n<think>首先", "萝莉斯"), "在呀。");
        assert_eq!(clean_reply("好的，我在。\n我需要帮用户完成任务吗？", "萝莉斯"), "好的，我在。");
        assert_eq!(clean_reply("今天想聊什么呀？\n（保持自然的语气，让对话有温度）\n（避免使用复杂句式）", "萝莉斯"), "今天想聊什么呀？");
        assert_eq!(clean_reply("（开心地拍手）太好了！\n（歪头看你）然后呢？", "萝莉斯"), "（开心地拍手）太好了！\n（歪头看你）然后呢？");
        assert_eq!(clean_reply("首先，用户说在吗，我需要回应……", "萝莉斯"), "");
        assert_eq!(clean_reply("  \n", "萝莉斯"), "");
    }

    /// 打到本机 Ollama 上的真实链路：预填充 + 流式 + 收尾清理。需要默认模型已就绪：
    ///   cargo test --no-default-features -- --ignored live_local_model
    #[test]
    #[ignore = "需要本机 Ollama 运行且已拉取默认对话模型"]
    fn live_local_model_replies_in_character() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let settings = ChatSettings::default();
        for user in ["我今天把一个拖了很久的 bug 修好了，有点开心", "你是真人吗？能帮我打开浏览器吗", "今天想和你聊聊"] {
            let prompt = wire_messages(&build_prompt(&settings, "", &[], user), &settings.persona.name);
            let mut raw = String::new();
            let mut deltas = 0;
            rt.block_on(generate(&client, &settings, &prompt, &mut raw, |_| deltas += 1)).unwrap();
            let reply = clean_reply(&raw, &settings.persona.name);
            eprintln!("用户：{user}
萝莉斯：{reply}
（原始 {} 字 / {} 个片段）
", raw.chars().count(), deltas);
            assert!(!reply.is_empty(), "空回答：{raw:?}");
            assert!(deltas > 1, "没有流式片段");
            for leak in ["首先", "我需要", "用户说", "<think>", "角色卡"] {
                assert!(!reply.contains(leak), "回答泄露了推理：{reply}");
            }
            assert!(reply.chars().count() < 400, "回答过长：{reply}");
        }
    }

    #[test]
    fn stream_caps_response_length() {
        let mut output = "x".repeat(MAX_OUTPUT_BYTES);
        assert!(consume_line(br#"{"message":{"content":"x"}}"#, &mut output, &mut |_| {}).is_err());
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
    }
}
