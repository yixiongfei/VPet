//! 番茄钟（docs/07 roadmap 2.4、docs/03 §7）。
//!
//! 相位机：专注 → 短休 → 专注 → … → 每 N 个长休。和计时器一样不自己读时钟，
//! 时间从外面喂进来，所以「第四个番茄钟之后是不是长休」能在测试里瞬间验证。
//!
//! 它不直接改宠物的数值——只把相位告诉状态机，由状态机决定播什么动画。
//! 番茄钟管「现在该专注还是该歇」，状态机管「身上的数值怎么变」，两件事。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "专注",
            Phase::ShortBreak => "短休",
            Phase::LongBreak => "长休",
        }
    }
}

/// 节奏。默认 25/5/15、四个一轮，可以改（docs/02 L0「可配置节奏」）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rhythm {
    pub focus_min: f32,
    pub short_break_min: f32,
    pub long_break_min: f32,
    /// 几个专注之后来一次长休
    pub per_long_break: u32,
}

impl Default for Rhythm {
    fn default() -> Self {
        Self {
            focus_min: 25.0,
            short_break_min: 5.0,
            long_break_min: 15.0,
            per_long_break: 4,
        }
    }
}

impl Rhythm {
    fn minutes(&self, p: Phase) -> f32 {
        match p {
            Phase::Focus => self.focus_min,
            Phase::ShortBreak => self.short_break_min,
            Phase::LongBreak => self.long_break_min,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pomodoro {
    /// None = 没在跑
    running: Option<Running>,
    pub rhythm: Rhythm,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Running {
    phase: Phase,
    /// 当前相位还剩多少秒
    remaining_sec: f32,
    /// 已经完成了几个专注
    completed: u32,
}

/// 相位换了。Core 据此发 `pomodoro:phase` 并把宠物切到对应活动
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseChange {
    pub from: Phase,
    pub to: Phase,
    /// 到这一刻为止完成了几个专注
    pub completed: u32,
}

/// 发给 Body 的每秒心跳
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tick {
    pub phase: Phase,
    pub remaining_sec: f32,
    pub completed: u32,
}

impl Pomodoro {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn snapshot(&self) -> Option<Tick> {
        self.running.map(|r| Tick {
            phase: r.phase,
            remaining_sec: r.remaining_sec,
            completed: r.completed,
        })
    }

    /// 从一个专注开始。已经在跑就当没看见——重复点不该把进度清零
    pub fn start(&mut self) -> Option<PhaseChange> {
        if self.running.is_some() {
            return None;
        }
        self.running = Some(Running {
            phase: Phase::Focus,
            remaining_sec: self.rhythm.focus_min * 60.0,
            completed: 0,
        });
        Some(PhaseChange {
            from: Phase::Focus,
            to: Phase::Focus,
            completed: 0,
        })
    }

    /// 停掉。返回这一轮完成了几个专注
    pub fn stop(&mut self) -> Option<u32> {
        self.running.take().map(|r| r.completed)
    }

    /// 走 `sec` 秒。相位换了就返回怎么换的
    pub fn advance(&mut self, sec: f32) -> Option<PhaseChange> {
        let r = self.running.as_mut()?;
        r.remaining_sec -= sec.max(0.0);
        if r.remaining_sec > 0.0 {
            return None;
        }
        let from = r.phase;
        if from == Phase::Focus {
            r.completed += 1;
        }
        let to = match from {
            // 攒够一轮就长休
            Phase::Focus if r.completed % self.rhythm.per_long_break.max(1) == 0 => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            _ => Phase::Focus,
        };
        r.phase = to;
        r.remaining_sec = self.rhythm.minutes(to) * 60.0;
        Some(PhaseChange {
            from,
            to,
            completed: r.completed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一直走到相位变，返回变化；走太久还没变就是有 bug
    fn run_to_change(p: &mut Pomodoro) -> PhaseChange {
        for _ in 0..100_000 {
            if let Some(c) = p.advance(1.0) {
                return c;
            }
        }
        panic!("走了十万秒都没换相位");
    }

    #[test]
    fn 开始就是专注() {
        let mut p = Pomodoro::default();
        assert!(!p.is_running());
        p.start();
        assert_eq!(p.snapshot().unwrap().phase, Phase::Focus);
        assert_eq!(p.snapshot().unwrap().remaining_sec, 25.0 * 60.0);
    }

    #[test]
    fn 重复点开始不会把进度清零() {
        let mut p = Pomodoro::default();
        p.start();
        p.advance(600.0);
        let left = p.snapshot().unwrap().remaining_sec;
        assert!(p.start().is_none(), "已经在跑了不该再返回一次变化");
        assert_eq!(p.snapshot().unwrap().remaining_sec, left, "进度被清了");
    }

    #[test]
    fn 专注完转短休() {
        let mut p = Pomodoro::default();
        p.start();
        let c = run_to_change(&mut p);
        assert_eq!((c.from, c.to), (Phase::Focus, Phase::ShortBreak));
        assert_eq!(c.completed, 1);
    }

    #[test]
    fn 第四个专注之后是长休() {
        let mut p = Pomodoro::default();
        p.start();
        let mut phases = Vec::new();
        for _ in 0..8 {
            phases.push(run_to_change(&mut p).to);
        }
        assert_eq!(
            phases,
            vec![
                Phase::ShortBreak, Phase::Focus,
                Phase::ShortBreak, Phase::Focus,
                Phase::ShortBreak, Phase::Focus,
                Phase::LongBreak, Phase::Focus,
            ],
            "25/5 四个一轮的标准节奏"
        );
    }

    #[test]
    fn 节奏可以改() {
        let mut p = Pomodoro {
            rhythm: Rhythm {
                focus_min: 50.0,
                short_break_min: 10.0,
                long_break_min: 30.0,
                per_long_break: 2,
            },
            ..Default::default()
        };
        p.start();
        assert_eq!(p.snapshot().unwrap().remaining_sec, 3000.0);
        run_to_change(&mut p); // 专注1 → 短休
        run_to_change(&mut p); // 短休 → 专注2
        let c = run_to_change(&mut p);
        assert_eq!(c.to, Phase::LongBreak, "两个一轮就该长休了");
    }

    #[test]
    fn 停下来会报完成了几个() {
        let mut p = Pomodoro::default();
        p.start();
        run_to_change(&mut p);
        assert_eq!(p.stop(), Some(1));
        assert!(!p.is_running());
        assert_eq!(p.stop(), None, "已经停了再停就没有了");
    }

    #[test]
    fn 没跑的时候走时间什么也不会发生() {
        let mut p = Pomodoro::default();
        assert!(p.advance(9999.0).is_none());
        assert!(p.snapshot().is_none());
    }

    #[test]
    fn 一拍跨过整个相位也只算换一次() {
        let mut p = Pomodoro::default();
        p.start();
        // 离线补算那种大步长
        let c = p.advance(25.0 * 60.0 + 10.0).unwrap();
        assert_eq!(c.to, Phase::ShortBreak);
        assert!(p.snapshot().unwrap().remaining_sec > 0.0, "新相位得有剩余时间");
    }

    #[test]
    fn 存下来再读回去还在原地() {
        let mut p = Pomodoro::default();
        p.start();
        p.advance(123.0);
        let json = serde_json::to_string(&p).unwrap();
        let back: Pomodoro = serde_json::from_str(&json).unwrap();
        assert_eq!(back.snapshot(), p.snapshot());
    }
}
