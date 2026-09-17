//! 本地存储（docs/07 roadmap 2.1）。
//!
//! 迁移用 SQLite 自带的 `user_version`：`MIGRATIONS` 是一个只增不改的数组，
//! 下标就是版本号，启动时把还没跑过的依次跑掉。不引迁移框架，二十行够了。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

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
];

/// 流水最多留这么多条，超了就从最旧的删。一分钟一条也能存一个多月
const MAX_LOG_ROWS: i64 = 50_000;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
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
        let db = Self {
            conn: Connection::open_in_memory()?,
        };
        db.init()?;
        Ok(db)
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
    use crate::core::state_machine::{Activity, Mood, PetState};

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
                action: None,
                updated_at: 1_700_000_000_000,
            },
            elapsed: 12.0,
            earned: 5.0,
            cooldowns: [("work_copy".to_string(), 20.0)].into_iter().collect(),
        };
        db.record_pet_state(&p).unwrap();
        let back = db.latest_pet_state().unwrap().unwrap();
        assert_eq!(back.state, p.state);
        assert_eq!(back.cooldowns, p.cooldowns, "冷却也要跟着重启活下来");
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
