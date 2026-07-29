//! 剪贴板历史存储。
//!
//! 平台无关：桌面监听进程与 Android 输入法都通过本库读写同构的 SQLite
//! 历史库。内容按 SHA-256 去重，重复复制只刷新时间与次数；未置顶条目
//! 按天数与总量双重上限清理。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use sha2::{Digest, Sha256};

/// 单条文本上限（字节），超过则拒绝入库
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// 单条图片上限（字节）
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// 条目类别。数据库中以整数存储，顺序不可变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    Text = 0,
    Image = 1,
    Files = 2,
}

impl ClipKind {
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(ClipKind::Text),
            1 => Some(ClipKind::Image),
            2 => Some(ClipKind::Files),
            _ => None,
        }
    }
}

/// 历史条目。`text` 语义随类别变化：文本内容 / 图片描述（暂空）/
/// 换行分隔的文件路径列表。图片位图数据单独经 [`ClipboardStore::image_data`] 取。
#[derive(Debug, Clone)]
pub struct ClipEntry {
    pub id: i64,
    pub kind: ClipKind,
    pub text: String,
    pub source_app: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub use_count: i64,
    pub pinned: bool,
    /// 图片数据字节数（非图片为 0），列表场景避免加载大 BLOB
    pub data_size: i64,
}

/// 留存策略：两条上限同时生效，置顶条目不受清理影响。
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub max_age_days: u32,
    pub max_entries: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy {
            max_age_days: 90,
            max_entries: 2000,
        }
    }
}

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

pub struct ClipboardStore {
    conn: Connection,
}

/// 毫秒级时间戳，仅用于展示与留存策略；列表排序依赖 touch_seq。
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hash_content(kind: ClipKind, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([kind as u8]);
    hasher.update(payload);
    hasher.finalize().into()
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<ClipEntry> {
    Ok(ClipEntry {
        id: row.get(0)?,
        kind: ClipKind::from_i64(row.get(1)?).unwrap_or(ClipKind::Text),
        text: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        source_app: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        use_count: row.get(6)?,
        pinned: row.get::<_, i64>(7)? != 0,
        data_size: row.get(8)?,
    })
}

const ENTRY_COLUMNS: &str =
    "id, kind, text, source_app, created_at, updated_at, use_count, pinned, \
     coalesce(length(data), 0)";

impl ClipboardStore {
    /// 打开（不存在则创建）历史库。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        Self::with_connection(conn)
    }

    /// 内存库，供测试与预览场景使用。
    pub fn open_in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clips (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                kind        INTEGER NOT NULL,
                text        TEXT,
                data        BLOB,
                hash        BLOB NOT NULL UNIQUE,
                source_app  TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                use_count   INTEGER NOT NULL DEFAULT 1,
                pinned      INTEGER NOT NULL DEFAULT 0,
                touch_seq   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_clips_order
                ON clips(pinned DESC, touch_seq DESC);",
        )?;
        Ok(ClipboardStore { conn })
    }

    /// 写入文本条目；内容重复时刷新时间与使用次数并返回原条目 id。
    pub fn insert_text(&self, text: &str, source_app: &str) -> Result<Option<i64>> {
        let trimmed_len = text.trim().len();
        if trimmed_len == 0 || text.len() > MAX_TEXT_BYTES {
            return Ok(None);
        }
        self.upsert(ClipKind::Text, Some(text), None, source_app)
            .map(Some)
    }

    /// 写入图片条目，`data` 为自包含位图（BMP/PNG 容器字节）。
    pub fn insert_image(&self, data: &[u8], source_app: &str) -> Result<Option<i64>> {
        if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
            return Ok(None);
        }
        self.upsert(ClipKind::Image, None, Some(data), source_app)
            .map(Some)
    }

    /// 写入文件路径列表条目。
    pub fn insert_files(&self, paths: &[String], source_app: &str) -> Result<Option<i64>> {
        if paths.is_empty() {
            return Ok(None);
        }
        let joined = paths.join("\n");
        self.upsert(ClipKind::Files, Some(&joined), None, source_app)
            .map(Some)
    }

    fn upsert(
        &self,
        kind: ClipKind,
        text: Option<&str>,
        data: Option<&[u8]>,
        source_app: &str,
    ) -> Result<i64> {
        let payload = text
            .map(str::as_bytes)
            .or(data)
            .expect("文本与数据必须提供其一");
        let hash = hash_content(kind, payload);
        let ts = now();

        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM clips WHERE hash = ?1", params![&hash[..]], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE clips SET updated_at = ?1, use_count = use_count + 1,
                                  source_app = ?2,
                                  touch_seq = (SELECT coalesce(max(touch_seq), 0) + 1 FROM clips)
                 WHERE id = ?3",
                params![ts, source_app, id],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO clips (kind, text, data, hash, source_app, created_at, updated_at,
                                touch_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6,
                     (SELECT coalesce(max(touch_seq), 0) + 1 FROM clips))",
            params![kind as i64, text, data, &hash[..], source_app, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 按置顶优先、时间倒序列出条目。
    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<ClipEntry>> {
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM clips
             ORDER BY pinned DESC, touch_seq DESC LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], entry_from_row)?;
        rows.collect()
    }

    /// 子串搜索（大小写不敏感，中文按原文匹配），仅覆盖文本与文件列表。
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<ClipEntry>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM clips
             WHERE text LIKE ?1 ESCAPE '\\'
             ORDER BY pinned DESC, touch_seq DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pattern, limit], entry_from_row)?;
        rows.collect()
    }

    /// 读取图片条目的位图数据。
    pub fn image_data(&self, id: i64) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT data FROM clips WHERE id = ?1 AND kind = ?2",
                params![id, ClipKind::Image as i64],
                |r| r.get(0),
            )
            .optional()
    }

    pub fn get(&self, id: i64) -> Result<Option<ClipEntry>> {
        let sql = format!("SELECT {ENTRY_COLUMNS} FROM clips WHERE id = ?1");
        self.conn
            .query_row(&sql, params![id], entry_from_row)
            .optional()
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE clips SET pinned = ?1 WHERE id = ?2",
            params![pinned as i64, id],
        )?;
        Ok(n > 0)
    }

    pub fn delete(&self, id: i64) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM clips WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// 清空全部未置顶条目。
    pub fn clear_unpinned(&self) -> Result<usize> {
        self.conn.execute("DELETE FROM clips WHERE pinned = 0", [])
    }

    /// 应用留存策略，返回清理条数。
    pub fn apply_retention(&self, policy: &RetentionPolicy) -> Result<usize> {
        let cutoff = now() - policy.max_age_days as i64 * 24 * 3600 * 1000;
        let by_age = self.conn.execute(
            "DELETE FROM clips WHERE pinned = 0 AND updated_at < ?1",
            params![cutoff],
        )?;
        // 超量部分：跳过最新 max_entries 条后全部删除（置顶不计入淘汰）
        let by_count = self.conn.execute(
            "DELETE FROM clips WHERE pinned = 0 AND id IN (
                SELECT id FROM clips WHERE pinned = 0
                ORDER BY touch_seq DESC LIMIT -1 OFFSET ?1
            )",
            params![policy.max_entries],
        )?;
        Ok(by_age + by_count)
    }

    pub fn count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT count(*) FROM clips", [], |r| r.get(0))
    }
}
