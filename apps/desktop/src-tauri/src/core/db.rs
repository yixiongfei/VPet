//! 本地存储（docs/07 roadmap 2.1）。
//!
//! 迁移用 SQLite 自带的 `user_version`：`MIGRATIONS` 是一个只增不改的数组，
//! 下标就是版本号，启动时把还没跑过的依次跑掉。不引迁移框架，二十行够了。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::memory::{MemoryItem, MemoryType, Source, Status};
use super::scheduler::Scheduler;
use super::state_machine::Pet;

/// 只能往后追加，**永远不要改已有的条目**——老库已经按旧内容跑过了
const MIGRATIONS: &[&str] = &[
    // v1：宠物状态流水。存成日志而不是单行，是为了以后能看趋势
    //（docs/07 的 pet_state_log），也为「连续 N 天状态差」这类判断留路。
    r#"
    CREATE TABLE pet_state_log (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        recorded_at INTEGER NOT NULL,
        state      TEXT    NOT NULL
    );
    CREATE INDEX idx_pet_state_log_time ON pet_state_log(recorded_at DESC);
    "#,
    // v2：计时器。整张表就一行 JSON——计时器最多几十个，为它建关系表
    //（id / label / due_at / repeat 四列 + 增删改查）是过度设计
    r#"
    CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v3：长期互动记忆（docs/07 roadmap 2.11）。
    //
    // 这张表**是真相来源**，所以每个字段都摊开成列，不像 `kv` 那样塞 JSON——
    // 记忆要按 status 过滤、按 expires_at 扫、按 importance 排、逐条删，
    // 这些都得让 SQLite 自己能做。向量索引（以后的 sqlite-vec）只是加速器，
    // 丢了能从这张表重建；这张表丢了就真没了。
    //
    // 没有 ON DELETE CASCADE：parent_id 指向被取代的旧版本，
    // 那条链正是「这条记忆是怎么变成今天这样的」的审计记录，不能跟着删。
    r#"
    CREATE TABLE memory_items (
        id                TEXT    PRIMARY KEY,
        content           TEXT    NOT NULL,
        type              TEXT    NOT NULL,
        importance        REAL    NOT NULL DEFAULT 50,
        confidence        REAL    NOT NULL DEFAULT 1.0,
        source            TEXT    NOT NULL,
        pinned            INTEGER NOT NULL DEFAULT 0,
        created_at        INTEGER NOT NULL,
        updated_at        INTEGER NOT NULL,
        last_accessed_at  INTEGER NOT NULL DEFAULT 0,
        access_count      INTEGER NOT NULL DEFAULT 0,
        expires_at        INTEGER,
        status            TEXT    NOT NULL DEFAULT 'active',
        embedding_version TEXT    NOT NULL DEFAULT '',
        parent_id         TEXT
    );
    CREATE INDEX idx_memory_status  ON memory_items(status, updated_at DESC);
    CREATE INDEX idx_memory_expires ON memory_items(expires_at) WHERE expires_at IS NOT NULL;
    "#,
];

/// 流水最多留这么多条，超了就从最旧的删。一分钟一条也能存一个多月
const MAX_LOG_ROWS: i64 = 50_000;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::register_vec_extension();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let db = Self {
            conn: Connection::open(path)?,
        };
        db.init()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::register_vec_extension();
        let db = Self {
            conn: Connection::open_in_memory()?,
        };
        db.init()?;
        Ok(db)
    }

    /// 把 sqlite-vec 注册成自动扩展。**必须在任何连接建立之前调用一次**，
    /// 之后打开的每个连接才带 `vec0`。放在这里而不是 `open()` 里，是因为
    /// `sqlite3_auto_extension` 是进程级的全局状态，重复注册没意义
    pub fn register_vec_extension() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            // 这段 unsafe 是 sqlite-vec 的既定用法：把 C 的初始化函数指针交给
            // SQLite。transmute 只是在两种等价的 C 函数签名之间转换
            let _ = rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite_vec::sqlite3_vec_init as *const ())));
        });
    }

    fn init(&self) -> rusqlite::Result<()> {
        // WAL：崩溃或断电时更不容易把库写坏
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "foreign_keys", true)?;
        self.migrate()
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let version: i64 =
            self.conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            self.conn.execute_batch(sql)?;
            self.conn
                .pragma_update(None, "user_version", (i + 1) as i64)?;
            log::info!("数据库迁移到 v{}", i + 1);
        }
        Ok(())
    }

    /// 记一条状态流水。存的是整个 `Pet`（含冷却和当前进度），这样重启之后
    /// 「刚吃过所以一会儿不吃」这类信息也还在。整体存 JSON——字段以后会加，
    /// 不想每加一个就来一次迁移
    pub fn record_pet_state(&self, p: &Pet) -> rusqlite::Result<()> {
        let json = serde_json::to_string(p).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(e))
        })?;
        self.conn.execute(
            "INSERT INTO pet_state_log (recorded_at, state) VALUES (?1, ?2)",
            (p.state.updated_at, json),
        )?;
        self.conn.execute(
            "DELETE FROM pet_state_log WHERE id <= (
                 SELECT MAX(id) - ?1 FROM pet_state_log
             )",
            [MAX_LOG_ROWS],
        )?;
        Ok(())
    }

    /// 最近一条状态。没有（第一次启动）或者存的内容读不回来时返回 None
    pub fn latest_pet_state(&self) -> rusqlite::Result<Option<Pet>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM pet_state_log ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| match serde_json::from_str(&j) {
            Ok(s) => Some(s),
            Err(e) => {
                // 旧版本写的格式读不回来不该拦住启动，当新宠物开始就是了
                log::warn!("读不回上次的状态，按新状态启动: {e}");
                None
            }
        }))
    }

    /// 通用键值。目前只放调度器，以后 settings 之类也走这里；
    /// 真需要按字段查询了再单独建表
    fn put(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )?;
        Ok(())
    }

    fn take(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    /* ---------- 长期记忆（docs/07 roadmap 2.11） ---------- */

    /// 写一条。id 冲突就整条覆盖——`plan_write` 已经决定了是新增还是更新，
    /// 「更新」在那边就是把旧 id 原样带回来，所以这里一个 upsert 够了
    pub fn put_memory(&self, m: &MemoryItem) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO memory_items
               (id, content, type, importance, confidence, source, pinned,
                created_at, updated_at, last_accessed_at, access_count,
                expires_at, status, embedding_version, parent_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(id) DO UPDATE SET
               content = excluded.content,
               type = excluded.type,
               importance = excluded.importance,
               confidence = excluded.confidence,
               source = excluded.source,
               pinned = excluded.pinned,
               updated_at = excluded.updated_at,
               expires_at = excluded.expires_at,
               status = excluded.status,
               embedding_version = excluded.embedding_version,
               parent_id = excluded.parent_id",
            rusqlite::params![
                m.id, m.content, m.kind.as_str(), m.importance, m.confidence,
                m.source.as_str(), m.pinned as i64,
                m.created_at, m.updated_at, m.last_accessed_at, m.access_count as i64,
                m.expires_at, m.status.as_str(), m.embedding_version, m.parent_id,
            ],
        )?;
        Ok(())
    }

    /// 全部读出来，**含已删除的**——调用方要按 status 过滤。
    /// 不在 SQL 里过滤是故意的：面板要能看到归档和删除的（那是审计），
    /// 而「什么能进模型上下文」由 `MemoryItem::usable` 统一裁决，只此一处
    pub fn all_memories(&self) -> rusqlite::Result<Vec<MemoryItem>> {
        let mut st = self.conn.prepare(
            "SELECT id, content, type, importance, confidence, source, pinned,
                    created_at, updated_at, last_accessed_at, access_count,
                    expires_at, status, embedding_version, parent_id
             FROM memory_items ORDER BY updated_at DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(MemoryItem {
                id: r.get(0)?,
                content: r.get(1)?,
                kind: MemoryType::parse(&r.get::<_, String>(2)?).unwrap_or(MemoryType::Profile),
                importance: r.get(3)?,
                confidence: r.get(4)?,
                source: Source::parse(&r.get::<_, String>(5)?).unwrap_or(Source::Inferred),
                pinned: r.get::<_, i64>(6)? != 0,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
                last_accessed_at: r.get(9)?,
                access_count: r.get::<_, i64>(10)? as u32,
                expires_at: r.get(11)?,
                status: Status::parse(&r.get::<_, String>(12)?).unwrap_or(Status::Active),
                embedding_version: r.get(13)?,
                parent_id: r.get(14)?,
            })
        })?;
        rows.collect()
    }

    /// 改状态。**删除也走这里**——软删除，行留着当审计，
    /// 但所有检索入口都按 status 过滤，所以它再也进不了模型上下文
    pub fn set_memory_status(&self, id: &str, status: Status, now: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE memory_items SET status = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, status.as_str(), now],
        )?;
        Ok(n > 0)
    }

    pub fn set_memory_pinned(&self, id: &str, pinned: bool, now: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE memory_items SET pinned = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, pinned as i64, now],
        )?;
        Ok(n > 0)
    }

    pub fn set_memory_importance(&self, id: &str, importance: f32, now: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE memory_items SET importance = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, importance.clamp(0.0, 100.0), now],
        )?;
        Ok(n > 0)
    }

    /// 命中一次就记一笔。使用频次占排序权重的 0.10——
    /// 没有这一笔，「常被用到的记忆更可能再被用到」就是空话
    pub fn touch_memories(&self, ids: &[String], now: i64) -> rusqlite::Result<()> {
        for id in ids {
            self.conn.execute(
                "UPDATE memory_items
                 SET access_count = access_count + 1, last_accessed_at = ?2
                 WHERE id = ?1",
                rusqlite::params![id, now],
            )?;
        }
        Ok(())
    }

    /* ---------- 向量索引（sqlite-vec） ----------
     *
     * 这张表**不是真相来源**，只是加速器：删掉它、换个模型重建，记忆一条不少。
     * 所以它不进 MIGRATIONS——迁移是只增不改的，而这张表的**宽度取决于模型维度**，
     * 换模型就得重建。用「按需建表 + 维度写在 kv 里」比硬塞进迁移序列诚实得多。
     */

    /// 确保向量表存在，且宽度/模型和当前的一致。不一致就整张重建——
    /// 不同模型的向量在同一个空间里比较是没有意义的，留着比删了更危险
    pub fn ensure_vec_table(&self, dim: usize, version: &str) -> rusqlite::Result<bool> {
        // 宽度要拼进 SQL（虚拟表的列宽不能用占位符），所以先验一遍，别让它成为注入口
        if dim == 0 || dim > 8192 {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "维度 {dim} 不合理"
            )));
        }
        let want = format!("{version}/{dim}");
        let have = self.take("vec_meta")?;
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vec_memories'",
            [],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if exists && have.as_deref() == Some(want.as_str()) {
            return Ok(false);
        }
        log::info!("重建向量索引：{:?} → {want}", have);
        self.conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS vec_memories;
             CREATE VIRTUAL TABLE vec_memories USING vec0(
                 memory_id TEXT PRIMARY KEY,
                 embedding float[{dim}] distance_metric=cosine
             );"
        ))?;
        // 旧向量作废，把所有条目的版本清空，后台会重新算
        self.conn
            .execute("UPDATE memory_items SET embedding_version = ''", [])?;
        self.put("vec_meta", &want)?;
        Ok(true)
    }

    /// 把所有条目的向量版本清空，逼后台重算一遍。
    /// 用在「我怀疑索引和记忆对不上」的手动重建路径上
    pub fn clear_embedding_versions(&self) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE memory_items SET embedding_version = ''", [])?;
        self.conn.execute("DELETE FROM vec_memories", [])?;
        Ok(())
    }

    pub fn has_vec_table(&self) -> bool {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vec_memories'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// 存一条向量，并把这条记忆标成「已按 version 算过」。
    /// 两件事必须一起成功——否则会出现「版本说算过了但索引里没有」这种查不出来的洞
    pub fn put_embedding(&self, id: &str, v: &[f32], version: &str) -> rusqlite::Result<()> {
        let bytes: &[u8] = bytemuck::cast_slice(v);
        self.conn.execute("DELETE FROM vec_memories WHERE memory_id = ?1", [id])?;
        self.conn.execute(
            "INSERT INTO vec_memories(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![id, bytes],
        )?;
        self.conn.execute(
            "UPDATE memory_items SET embedding_version = ?2 WHERE id = ?1",
            rusqlite::params![id, version],
        )?;
        Ok(())
    }

    /// 删向量。docs §4.5 明写了「忘记」要**从向量索引中删除**——
    /// 记忆表里留一行当审计是可以的，但它不能再被检索到，两件事不冲突
    pub fn delete_embedding(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM vec_memories WHERE memory_id = ?1", [id])?;
        self.conn.execute(
            "UPDATE memory_items SET embedding_version = '' WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// 最近邻。返回 (记忆 id, 余弦)。
    /// 建表时声明了 `distance_metric=cosine`，vec0 给的 distance = 1 − cos
    pub fn knn(&self, q: &[f32], k: usize) -> rusqlite::Result<Vec<(String, f32)>> {
        if !self.has_vec_table() || q.is_empty() {
            return Ok(Vec::new());
        }
        let bytes: &[u8] = bytemuck::cast_slice(q);
        let mut st = self.conn.prepare(
            "SELECT memory_id, distance FROM vec_memories
             WHERE embedding MATCH ?1 AND k = ?2
             ORDER BY distance",
        )?;
        let rows = st.query_map(rusqlite::params![bytes, k as i64], |r| {
            let id: String = r.get(0)?;
            let d: f64 = r.get(1)?;
            Ok((id, (1.0 - d) as f32))
        })?;
        rows.collect()
    }

    /// 还没按当前模型算过向量的活跃记忆。后台补算就照着这个名单来
    pub fn memories_needing_embedding(&self, version: &str) -> rusqlite::Result<Vec<MemoryItem>> {
        Ok(self
            .all_memories()?
            .into_iter()
            .filter(|m| m.status == Status::Active && m.embedding_version != version)
            .collect())
    }

    pub fn save_scheduler(&self, s: &Scheduler) -> rusqlite::Result<()> {
        let json = serde_json::to_string(s)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.put("scheduler", &json)
    }

    /// 读回计时器。读不回来就当没有——几个闹钟丢了，不该拦住启动
    pub fn load_scheduler(&self) -> Scheduler {
        match self.take("scheduler") {
            Ok(Some(j)) => serde_json::from_str(&j).unwrap_or_else(|e| {
                log::warn!("读不回计时器: {e}");
                Scheduler::default()
            }),
            _ => Scheduler::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bias::Biases;
    use crate::core::memory::MemoryItem;
    use crate::core::state_machine::{Activity, Mood, PetState};

    /* ---------- 长期记忆 ---------- */

    fn mem(id: &str, content: &str) -> MemoryItem {
        crate::core::memory::draft(
            id.into(), content.into(), MemoryType::Habit, 70.0, 0.9,
            Source::UserExplicit, None, "bigram-v1", 1_760_000_000_000,
        )
    }

    #[test]
    fn 记忆存得进读得回() {
        let db = Db::open_in_memory().unwrap();
        let m = mem("m1", "用户晚上上班白天学习");
        db.put_memory(&m).unwrap();
        let back = db.all_memories().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], m, "存回来的和存进去的不是同一条");
    }

    #[test]
    fn 同一个_id_是覆盖不是新增() {
        let db = Db::open_in_memory().unwrap();
        db.put_memory(&mem("m1", "旧的说法")).unwrap();
        let mut newer = mem("m1", "新的说法");
        newer.updated_at += 1000;
        db.put_memory(&newer).unwrap();
        let back = db.all_memories().unwrap();
        assert_eq!(back.len(), 1, "纠正旧信息时新增了一条");
        assert_eq!(back[0].content, "新的说法");
    }

    #[test]
    fn 删除是软删除_行还在但不可用() {
        let db = Db::open_in_memory().unwrap();
        db.put_memory(&mem("m1", "用户晚上上班")).unwrap();
        let now = 1_760_000_000_000;
        assert!(db.set_memory_status("m1", Status::Deleted, now).unwrap());
        let back = db.all_memories().unwrap();
        assert_eq!(back.len(), 1, "行应该留着当审计");
        assert_eq!(back[0].status, Status::Deleted);
        assert!(!back[0].usable(now), "删掉的还能进上下文");
    }

    #[test]
    fn 删不存在的记忆不报错只回_false() {
        let db = Db::open_in_memory().unwrap();
        assert!(!db.set_memory_status("nope", Status::Deleted, 0).unwrap());
    }

    #[test]
    fn 命中会累加使用次数() {
        let db = Db::open_in_memory().unwrap();
        db.put_memory(&mem("m1", "甲")).unwrap();
        db.touch_memories(&["m1".to_string()], 999).unwrap();
        db.touch_memories(&["m1".to_string()], 1000).unwrap();
        let back = db.all_memories().unwrap();
        assert_eq!(back[0].access_count, 2);
        assert_eq!(back[0].last_accessed_at, 1000);
    }

    #[test]
    fn 置顶和重要性改得动() {
        let db = Db::open_in_memory().unwrap();
        db.put_memory(&mem("m1", "甲")).unwrap();
        db.set_memory_pinned("m1", true, 1).unwrap();
        db.set_memory_importance("m1", 100.0, 2).unwrap();
        let back = db.all_memories().unwrap();
        assert!(back[0].pinned);
        assert_eq!(back[0].importance, 100.0);
    }

    /// 端到端走一遍 docs §9 的验收 1 / 2 / 5：
    /// 说「记住…」→ 落库 → 问相关问题能检索到 → 说「忘记…」→ 再也检索不到 → 但行还在（审计）
    #[test]
    fn 端到端_记住之后能用_忘记之后不再用() {
        use crate::core::memory::{
            draft, parse_command, plan_write, retrieve, render_context, BigramRetriever, Command,
            Retriever, WriteOutcome, TOP_K,
        };
        let db = Db::open_in_memory().unwrap();
        let r = BigramRetriever;
        let now = 1_760_000_000_000;

        // 1. 用户说「记住…」
        let Some(Command::Remember { content, kind, importance, ttl_ms }) =
            parse_command("记住：我晚上上班，白天学习")
        else {
            panic!("没解析出记忆命令")
        };
        let cand = draft("m1".into(), content, kind, importance, 1.0,
                         Source::UserExplicit, ttl_ms.map(|t| now + t), r.version(), now);
        let WriteOutcome::Created { item } = plan_write(&db.all_memories().unwrap(), cand, &r, now)
        else {
            panic!("第一条应当是新增")
        };
        db.put_memory(&item).unwrap();

        // 2. 问相关问题能检索到，且成文里有它
        let hits = retrieve(&db.all_memories().unwrap(), "我白天学习该怎么安排", &r, now, TOP_K);
        assert_eq!(hits.len(), 1, "记住了却检索不到");
        db.touch_memories(&[hits[0].item.id.clone()], now).unwrap();
        assert!(render_context(&hits).contains("白天学习"));

        // 3. 闲聊不会再往库里加东西
        assert!(parse_command("今天天气不错").is_none());
        assert_eq!(db.all_memories().unwrap().len(), 1);

        // 4. 用户说「忘记…」
        let Some(Command::Forget { query }) = parse_command("忘记我晚上上班") else {
            panic!("没解析出忘记命令")
        };
        let all = db.all_memories().unwrap();
        let target = all
            .iter()
            .map(|m| (m, r.similarity(&query, &m.content)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(m, _)| m.id.clone())
            .unwrap();
        db.set_memory_status(&target, Status::Deleted, now).unwrap();

        // 5. 再也进不了上下文，但行还在
        let after = db.all_memories().unwrap();
        assert_eq!(after.len(), 1, "软删除应当留下行当审计");
        assert!(
            retrieve(&after, "我白天学习该怎么安排", &r, now, TOP_K).is_empty(),
            "删掉的记忆还在进上下文"
        );
        assert!(render_context(&retrieve(&after, "白天学习", &r, now, TOP_K)).is_empty());
    }

    /* ---------- 向量索引 ---------- */

    fn unit(v: [f32; 4]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    #[test]
    fn 向量表能建能查_余弦方向对() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.ensure_vec_table(4, "test-v1").unwrap(), "第一次应当是新建");
        assert!(!db.ensure_vec_table(4, "test-v1").unwrap(), "同版本同维度不该重建");

        for (id, v) in [("a", [1.0, 0.0, 0.0, 0.0]), ("b", [0.0, 1.0, 0.0, 0.0]),
                        ("c", [0.9, 0.1, 0.0, 0.0])] {
            db.put_memory(&mem(id, id)).unwrap();
            db.put_embedding(id, &unit(v), "test-v1").unwrap();
        }
        let hits = db.knn(&unit([1.0, 0.0, 0.0, 0.0]), 3).unwrap();
        assert_eq!(hits[0].0, "a");
        assert!((hits[0].1 - 1.0).abs() < 1e-3, "自己和自己的余弦不是 1：{}", hits[0].1);
        assert_eq!(hits[1].0, "c", "排序不对：{hits:?}");
        assert!(hits[2].1 < 0.1, "正交的余弦该接近 0：{}", hits[2].1);
    }

    #[test]
    fn 换模型或换维度会整张重建并清空版本号() {
        let db = Db::open_in_memory().unwrap();
        db.ensure_vec_table(4, "old-v1").unwrap();
        db.put_memory(&mem("a", "甲")).unwrap();
        db.put_embedding("a", &unit([1.0, 0.0, 0.0, 0.0]), "old-v1").unwrap();
        assert_eq!(db.all_memories().unwrap()[0].embedding_version, "old-v1");

        // 不同模型的向量在同一个空间里比较没有意义，留着比删了更危险
        assert!(db.ensure_vec_table(8, "new-v1").unwrap(), "维度变了却没重建");
        assert!(db.knn(&vec![0.0; 8], 5).unwrap().is_empty(), "旧向量没清掉");
        assert_eq!(
            db.all_memories().unwrap()[0].embedding_version, "",
            "版本号没清空，后台就不会去重算"
        );
    }

    #[test]
    fn 待补算的名单只含活跃且版本不符的() {
        let db = Db::open_in_memory().unwrap();
        db.ensure_vec_table(4, "v1").unwrap();
        db.put_memory(&mem("a", "甲")).unwrap();
        db.put_memory(&mem("b", "乙")).unwrap();
        let mut gone = mem("c", "丙");
        gone.status = Status::Deleted;
        db.put_memory(&gone).unwrap();

        db.put_embedding("a", &unit([1.0, 0.0, 0.0, 0.0]), "v1").unwrap();
        let todo = db.memories_needing_embedding("v1").unwrap();
        let ids: Vec<&str> = todo.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["b"], "名单不对：{ids:?}");
    }

    #[test]
    fn 忘记要把向量也删掉() {
        let db = Db::open_in_memory().unwrap();
        db.ensure_vec_table(4, "v1").unwrap();
        db.put_memory(&mem("a", "甲")).unwrap();
        let v = unit([1.0, 0.0, 0.0, 0.0]);
        db.put_embedding("a", &v, "v1").unwrap();
        assert_eq!(db.knn(&v, 5).unwrap().len(), 1);

        // docs §4.5：忘记 = 标记删除 + 从向量索引中删除
        db.set_memory_status("a", Status::Deleted, 0).unwrap();
        db.delete_embedding("a").unwrap();
        assert!(db.knn(&v, 5).unwrap().is_empty(), "删了还能被最近邻召回");
        assert_eq!(db.all_memories().unwrap().len(), 1, "行要留着当审计");
    }

    #[test]
    fn 重复写同一条向量是覆盖不是堆积() {
        let db = Db::open_in_memory().unwrap();
        db.ensure_vec_table(4, "v1").unwrap();
        db.put_memory(&mem("a", "甲")).unwrap();
        db.put_embedding("a", &unit([1.0, 0.0, 0.0, 0.0]), "v1").unwrap();
        db.put_embedding("a", &unit([0.0, 1.0, 0.0, 0.0]), "v1").unwrap();
        let hits = db.knn(&unit([0.0, 1.0, 0.0, 0.0]), 5).unwrap();
        assert_eq!(hits.len(), 1, "同一条记忆在索引里有两份");
        assert!((hits[0].1 - 1.0).abs() < 1e-3);
    }

    #[test]
    fn 没建表时最近邻返回空而不是报错() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.knn(&[1.0, 0.0], 5).unwrap().is_empty());
    }

    #[test]
    fn 荒唐的维度会被挡住() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.ensure_vec_table(0, "v").is_err());
        assert!(db.ensure_vec_table(99999, "v").is_err());
    }

    #[test]
    fn 迁移可以重复跑() {
        let db = Db::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.migrate().unwrap();
        let v: i64 = db
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
    }

    #[test]
    fn 计时器能存能读() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.load_scheduler().is_empty(), "空库不该有计时器");
        let mut sc = Scheduler::default();
        sc.add("二十五分钟后叫我", 1_500_000, false, 1_700_000_000_000);
        db.save_scheduler(&sc).unwrap();
        let back = db.load_scheduler();
        assert_eq!(back.list(), sc.list());
        assert_eq!(back.seq(), sc.seq());
    }

    #[test]
    fn 计时器存坏了不拦启动() {
        let db = Db::open_in_memory().unwrap();
        db.put("scheduler", "不是json").unwrap();
        assert!(db.load_scheduler().is_empty());
    }

    #[test]
    fn 空库读不到状态() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.latest_pet_state().unwrap().is_none());
    }

    #[test]
    fn 存了能原样读回来() {
        let db = Db::open_in_memory().unwrap();
        let p = Pet {
            state: PetState {
                activity: Activity::Sleeping,
                mood: Mood::Happy,
                strength: 42.5,
                feeling: 77.0,
                hunger: 33.0,
                thirst: 11.0,
                money: 120.0,
                exp: 900.0,
                level: 3,
                affection: 63.5,
                action: None,
                updated_at: 1_700_000_000_000,
            },
            elapsed: 12.0,
            earned: 5.0,
            cooldowns: [("work_copy".to_string(), 20.0)].into_iter().collect(),
            pinned_tag: None,
            pressure: 1.0,
            touch_budget: 7.0,
            last_verdict: None,
            biases: {
                let mut b = Biases::default();
                b.set("work", 1.0, 120.0);
                b
            },
        };
        db.record_pet_state(&p).unwrap();
        let back = db.latest_pet_state().unwrap().unwrap();
        assert_eq!(back.state, p.state);
        assert_eq!(back.cooldowns, p.cooldowns, "冷却也要跟着重启活下来");
        assert_eq!(back.biases, p.biases, "用户偏好也要跟着重启活下来");
    }

    #[test]
    fn 读到的是最新那条() {
        let db = Db::open_in_memory().unwrap();
        for hunger in [90.0, 80.0, 70.0] {
            let mut p = Pet::default();
            p.state.hunger = hunger;
            p.state.updated_at = hunger as i64;
            db.record_pet_state(&p).unwrap();
        }
        assert_eq!(db.latest_pet_state().unwrap().unwrap().state.hunger, 70.0);
    }

    #[test]
    fn 存坏了的内容不会拦住启动() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO pet_state_log (recorded_at, state) VALUES (1, '不是json')",
                [],
            )
            .unwrap();
        assert!(db.latest_pet_state().unwrap().is_none());
    }

    #[test]
    fn 流水不会无限涨() {
        let db = Db::open_in_memory().unwrap();
        for i in 0..(MAX_LOG_ROWS + 200) {
            let mut p = Pet::default();
            p.state.updated_at = i;
            db.record_pet_state(&p).unwrap();
        }
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM pet_state_log", [], |r| r.get(0))
            .unwrap();
        assert!(n <= MAX_LOG_ROWS + 1, "实际 {n} 条");
    }
}
