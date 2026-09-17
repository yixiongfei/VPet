//! 计时器与调度（docs/07 roadmap 2.3）。
//!
//! 不引 tokio：进程里已经有一条每秒一拍的心跳（`spawn_pet_clock`），
//! 再起一套调度机制只会多一个要对齐的时钟。到期判断是纯函数，时间由调用方喂进来。
//!
//! 持久化在 `timers` 表。重启时**过期的补发一次**，没到期的重新挂上——
//! 「关掉电脑再打开，二十五分钟前定的闹钟还算数」是这东西唯一值得做的理由。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    pub id: String,
    pub label: String,
    /// 下次触发的时刻（Unix 毫秒）
    pub due_at: i64,
    /// 周期计时器的间隔；None = 响一次就没了
    pub repeat_ms: Option<i64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Scheduler {
    timers: Vec<Timer>,
    /// 自增序号，拼出不重复的 id
    seq: u64,
}

impl Scheduler {
    pub fn load(timers: Vec<Timer>, seq: u64) -> Self {
        Self { timers, seq }
    }

    pub fn list(&self) -> &[Timer] {
        &self.timers
    }

    pub fn len(&self) -> usize {
        self.timers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// 排一个计时器。`now_ms` 由调用方给——这里不读时钟
    pub fn add(&mut self, label: &str, delay_ms: i64, repeat: bool, now_ms: i64) -> Timer {
        self.seq += 1;
        let t = Timer {
            id: format!("t{}", self.seq),
            label: label.to_string(),
            due_at: now_ms + delay_ms.max(0),
            repeat_ms: repeat.then_some(delay_ms.max(1)),
        };
        self.timers.push(t.clone());
        t
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        let before = self.timers.len();
        self.timers.retain(|t| t.id != id);
        self.timers.len() != before
    }

    pub fn cancel_all(&mut self) -> usize {
        let n = self.timers.len();
        self.timers.clear();
        n
    }

    /// 取出到期的。周期的重新排到下一次，一次性的删掉。
    ///
    /// 周期计时器的下一次从**现在**起算，不是从原定时刻起算——补发一次就够了，
    /// 关机一天不该开机就连响一百次。
    pub fn take_due(&mut self, now_ms: i64) -> Vec<Timer> {
        let mut fired = Vec::new();
        self.timers.retain_mut(|t| {
            if t.due_at > now_ms {
                return true;
            }
            fired.push(t.clone());
            match t.repeat_ms {
                Some(every) => {
                    t.due_at = now_ms + every;
                    true
                }
                None => false,
            }
        });
        fired
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }
}

/// `"10s"` `"25m"` `"1h"` `"90"`（光数字按秒算）→ 毫秒。
/// 这是给 Brain 用的——LLM 写出来的时长就长这样
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = match s.char_indices().find(|(_, c)| c.is_alphabetic()) {
        Some((i, _)) => (&s[..i], &s[i..]),
        None => (s, "s"),
    };
    let n: f64 = num.trim().parse().ok()?;
    if n < 0.0 {
        return None;
    }
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "ms" => 1.0,
        "s" | "sec" | "secs" | "second" | "seconds" => 1_000.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
        "h" | "hr" | "hour" | "hours" => 3_600_000.0,
        "d" | "day" | "days" => 86_400_000.0,
        _ => return None,
    };
    Some((n * mult) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn 到点才响() {
        let mut s = Scheduler::default();
        s.add("十秒后", 10_000, false, NOW);
        assert!(s.take_due(NOW + 9_999).is_empty(), "没到点不该响");
        assert_eq!(s.take_due(NOW + 10_000).len(), 1);
    }

    #[test]
    fn 一次性的响完就没了() {
        let mut s = Scheduler::default();
        s.add("一次", 1_000, false, NOW);
        s.take_due(NOW + 1_000);
        assert!(s.is_empty());
    }

    #[test]
    fn 周期的会重新排上() {
        let mut s = Scheduler::default();
        s.add("每分钟", 60_000, true, NOW);
        assert_eq!(s.take_due(NOW + 60_000).len(), 1);
        assert_eq!(s.len(), 1, "周期计时器不该被删掉");
        assert_eq!(s.list()[0].due_at, NOW + 120_000);
    }

    #[test]
    fn 关机一天不会开机连响一百次() {
        let mut s = Scheduler::default();
        s.add("每分钟", 60_000, true, NOW);
        // 一天后才开机
        let day = NOW + 86_400_000;
        assert_eq!(s.take_due(day).len(), 1, "补发一次就够");
        assert_eq!(s.list()[0].due_at, day + 60_000, "下一次从现在起算");
    }

    #[test]
    fn 过期的一次性计时器开机会补发() {
        let mut s = Scheduler::default();
        s.add("早该响了", 1_000, false, NOW);
        let fired = s.take_due(NOW + 999_999);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].label, "早该响了");
    }

    #[test]
    fn 可以取消() {
        let mut s = Scheduler::default();
        let t = s.add("算了", 10_000, false, NOW);
        assert!(s.cancel(&t.id));
        assert!(!s.cancel(&t.id), "取消不存在的应该返回 false");
        assert!(s.take_due(NOW + 99_999).is_empty());
    }

    #[test]
    fn id不重复() {
        let mut s = Scheduler::default();
        let ids: Vec<String> = (0..50).map(|_| s.add("x", 1, false, NOW).id).collect();
        let mut uniq = ids.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len());
    }

    #[test]
    fn 多个计时器各响各的() {
        let mut s = Scheduler::default();
        s.add("快的", 1_000, false, NOW);
        s.add("慢的", 10_000, false, NOW);
        assert_eq!(s.take_due(NOW + 1_000)[0].label, "快的");
        assert_eq!(s.len(), 1);
        assert_eq!(s.take_due(NOW + 10_000)[0].label, "慢的");
    }

    #[test]
    fn 时长能按人写的方式解析() {
        assert_eq!(parse_duration("10s"), Some(10_000));
        assert_eq!(parse_duration("25m"), Some(1_500_000));
        assert_eq!(parse_duration("1h"), Some(3_600_000));
        assert_eq!(parse_duration("500ms"), Some(500));
        assert_eq!(parse_duration("90"), Some(90_000), "光给数字按秒算");
        assert_eq!(parse_duration(" 2 minutes "), Some(120_000));
    }

    #[test]
    fn 时长写错了不瞎猜() {
        for bad in ["", "abc", "10 光年", "-5s"] {
            assert_eq!(parse_duration(bad), None, "{bad} 不该被解析出来");
        }
    }

    #[test]
    fn 存下来再读回去还是那些计时器() {
        let mut s = Scheduler::default();
        s.add("a", 1_000, false, NOW);
        s.add("b", 2_000, true, NOW);
        let json = serde_json::to_string(&s).unwrap();
        let back: Scheduler = serde_json::from_str(&json).unwrap();
        assert_eq!(back.list(), s.list());
        assert_eq!(back.seq(), s.seq(), "序号也要存，否则重启后 id 会撞");
    }
}
