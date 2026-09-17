//! 宠物状态机（docs/03 §7、docs/05 §5）。
//!
//! `reduce` 是纯函数：同样的状态 + 同样的事件永远得到同样的新状态，不读时钟、不做 IO。
//! 时间从外面以 `Event::Tick { minutes }` 喂进来——这样「四小时后会饿」这种行为
//! 可以在单元测试里瞬间验证，不用真等四小时。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    Idle,
    Working,
    Break,
    Studying,
    Sleeping,
    Playing,
    Eating,
    Drinking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Happy,
    Nomal,
    PoorCondition,
    Ill,
}

/// 发给 Body 的 `pet:state` 载荷，字段与 packages/shared/src/pet.ts 的 zod schema 一一对应
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub activity: Activity,
    pub mood: Mood,
    pub strength: f32,
    pub feeling: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub updated_at: i64,
}

impl Default for PetState {
    fn default() -> Self {
        Self {
            activity: Activity::Idle,
            mood: Mood::Nomal,
            strength: 100.0,
            feeling: 60.0,
            hunger: 100.0,
            thirst: 100.0,
            updated_at: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    Head,
    Body,
    Raise,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// 时间流逝。分钟数用浮点，便于用秒级 tick 驱动
    Tick { minutes: f32 },
    Touched(Touch),
    /// 吃 / 喝 完成，把饱腹或水补回去
    Consumed,
    /// 调试用：直接改数值，用来验证阈值行为
    Patch {
        strength: Option<f32>,
        feeling: Option<f32>,
        hunger: Option<f32>,
        thirst: Option<f32>,
    },
}

/* --- 数值（docs/05 §5）。都是每分钟的量 --- */

const HUNGER_PER_MIN: f32 = 0.4; // 满饱腹撑约 4 小时
const THIRST_PER_MIN: f32 = 0.6; // 满水约 2.8 小时

/// 低于这个值就去吃 / 喝
pub const HUNGRY_AT: f32 = 30.0;
pub const THIRSTY_AT: f32 = 30.0;

const EAT_RESTORE: f32 = 45.0;
const DRINK_RESTORE: f32 = 55.0;

/// 摸一下给多少心情。原版还有每小时上限，那属于 Phase 7 的 Mood Engine
const FEELING_HEAD: f32 = 1.0;
const FEELING_BODY: f32 = 0.5;
/// 被拎起来不太舒服，但也不惩罚（docs/01 三条原则：情绪只正向放大）
const FEELING_RAISE: f32 = 0.0;
const STRENGTH_PER_TOUCH: f32 = 0.2;

const HAPPY_FEELING: f32 = 70.0;
const NOMAL_FEELING: f32 = 40.0;
const POOR_STRENGTH: f32 = 20.0;

/// 每分钟的体力变化，按当前在干什么
fn strength_rate(a: Activity) -> f32 {
    match a {
        Activity::Working | Activity::Studying => -0.3,
        Activity::Playing => -0.2,
        Activity::Sleeping => 1.0,
        Activity::Idle | Activity::Break => 0.5,
        // 吃喝只是过场，几秒钟的事，不单独算
        Activity::Eating | Activity::Drinking => 0.0,
    }
}

/// 状态机主体。不设置 `updated_at`——那是时钟，由调用方填
pub fn reduce(s: &PetState, e: &Event) -> PetState {
    let mut n = *s;
    match e {
        Event::Tick { minutes } => {
            let m = minutes.max(0.0);
            n.strength += strength_rate(s.activity) * m;
            n.hunger -= HUNGER_PER_MIN * m;
            n.thirst -= THIRST_PER_MIN * m;
            clamp(&mut n);
            n.activity = auto_activity(&n);
        }
        Event::Touched(t) => {
            n.feeling += match t {
                Touch::Head => FEELING_HEAD,
                Touch::Body => FEELING_BODY,
                Touch::Raise => FEELING_RAISE,
            };
            n.strength -= STRENGTH_PER_TOUCH;
            clamp(&mut n);
        }
        Event::Consumed => {
            match s.activity {
                Activity::Eating => n.hunger += EAT_RESTORE,
                Activity::Drinking => n.thirst += DRINK_RESTORE,
                _ => {}
            }
            n.activity = Activity::Idle;
            clamp(&mut n);
        }
        Event::Patch {
            strength,
            feeling,
            hunger,
            thirst,
        } => {
            if let Some(v) = strength {
                n.strength = *v;
            }
            if let Some(v) = feeling {
                n.feeling = *v;
            }
            if let Some(v) = hunger {
                n.hunger = *v;
            }
            if let Some(v) = thirst {
                n.thirst = *v;
            }
            clamp(&mut n);
        }
    }
    n.mood = derive_mood(&n);
    n
}

/// 饿了 / 渴了就自己去吃喝。
///
/// **只从 Idle / Break 触发**：专注（Working / Studying）不打断，睡觉不叫醒，
/// 已经在吃喝的不重入。等专注结束回到空闲，该补的自然会补上。
fn auto_activity(s: &PetState) -> Activity {
    if !matches!(s.activity, Activity::Idle | Activity::Break) {
        return s.activity;
    }
    let hungry = s.hunger < HUNGRY_AT;
    let thirsty = s.thirst < THIRSTY_AT;
    match (hungry, thirsty) {
        // 都低就先解决更急的那个
        (true, true) if s.thirst < s.hunger => Activity::Drinking,
        (true, _) => Activity::Eating,
        (false, true) => Activity::Drinking,
        _ => s.activity,
    }
}

/// docs/05 §5：feeling ≥ 70 → Happy；≥ 40 → Nomal；< 40 或 strength < 20 → PoorCondition。
/// Ill 要「连续 3 天 PoorCondition」，得先有持久化（roadmap 2.1），这里先不产出。
fn derive_mood(s: &PetState) -> Mood {
    if s.strength < POOR_STRENGTH || s.feeling < NOMAL_FEELING {
        Mood::PoorCondition
    } else if s.feeling >= HAPPY_FEELING {
        Mood::Happy
    } else {
        Mood::Nomal
    }
}

fn clamp(s: &mut PetState) {
    for v in [
        &mut s.strength,
        &mut s.feeling,
        &mut s.hunger,
        &mut s.thirst,
    ] {
        *v = v.clamp(0.0, 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 连续 tick n 分钟，每次一分钟——和真实 ticker 的累积方式一致
    fn run(mut s: PetState, minutes: u32) -> PetState {
        for _ in 0..minutes {
            s = reduce(&s, &Event::Tick { minutes: 1.0 });
        }
        s
    }

    #[test]
    fn 时间流逝会饿会渴() {
        let s = run(PetState::default(), 60);
        assert!((s.hunger - (100.0 - 60.0 * HUNGER_PER_MIN)).abs() < 0.5);
        assert!((s.thirst - (100.0 - 60.0 * THIRST_PER_MIN)).abs() < 0.5);
    }

    #[test]
    fn 数值不会越界() {
        let s = run(PetState::default(), 10_000);
        assert!(s.hunger >= 0.0 && s.thirst >= 0.0);
        assert!(s.strength <= 100.0 && s.feeling <= 100.0);
    }

    #[test]
    fn 饿了会自己去吃() {
        let s = PetState {
            hunger: HUNGRY_AT + 0.1,
            ..Default::default()
        };
        let s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Eating);
    }

    #[test]
    fn 渴了会自己去喝() {
        let s = PetState {
            thirst: THIRSTY_AT + 0.1,
            ..Default::default()
        };
        let s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Drinking);
    }

    #[test]
    fn 又饿又渴先解决更急的() {
        let s = PetState {
            hunger: 25.0,
            thirst: 10.0,
            ..Default::default()
        };
        assert_eq!(auto_activity(&s), Activity::Drinking);
        let s = PetState {
            hunger: 10.0,
            thirst: 25.0,
            ..Default::default()
        };
        assert_eq!(auto_activity(&s), Activity::Eating);
    }

    #[test]
    fn 专注时不打断去吃饭() {
        let s = PetState {
            activity: Activity::Working,
            hunger: 1.0,
            thirst: 1.0,
            ..Default::default()
        };
        let s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Working);
    }

    #[test]
    fn 睡觉时不会被叫醒去吃饭() {
        let s = PetState {
            activity: Activity::Sleeping,
            hunger: 1.0,
            ..Default::default()
        };
        let s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Sleeping);
    }

    #[test]
    fn 吃完补饱腹并回到空闲() {
        let s = PetState {
            activity: Activity::Eating,
            hunger: 10.0,
            ..Default::default()
        };
        let s = reduce(&s, &Event::Consumed);
        assert_eq!(s.activity, Activity::Idle);
        assert!(s.hunger > HUNGRY_AT, "吃完应该高于阈值，否则会立刻再吃");
    }

    #[test]
    fn 吃完不会立刻再次触发() {
        let mut s = PetState {
            hunger: 1.0,
            ..Default::default()
        };
        s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Eating);
        s = reduce(&s, &Event::Consumed);
        s = reduce(&s, &Event::Tick { minutes: 1.0 });
        assert_eq!(s.activity, Activity::Idle, "补完之后该回空闲待着");
    }

    #[test]
    fn 摸头涨心情() {
        let s = PetState {
            feeling: 50.0,
            ..Default::default()
        };
        let after = reduce(&s, &Event::Touched(Touch::Head));
        assert!(after.feeling > s.feeling);
        assert!(after.strength < s.strength, "摸也要消耗一点体力");
    }

    #[test]
    fn 提起不扣心情() {
        let s = PetState {
            feeling: 50.0,
            ..Default::default()
        };
        let after = reduce(&s, &Event::Touched(Touch::Raise));
        assert!(after.feeling >= s.feeling, "情绪引擎只正向放大，不惩罚");
    }

    #[test]
    fn 心情决定表情() {
        let happy = reduce(
            &PetState {
                feeling: 80.0,
                ..Default::default()
            },
            &Event::Tick { minutes: 0.0 },
        );
        assert_eq!(happy.mood, Mood::Happy);

        let poor = reduce(
            &PetState {
                feeling: 20.0,
                ..Default::default()
            },
            &Event::Tick { minutes: 0.0 },
        );
        assert_eq!(poor.mood, Mood::PoorCondition);
    }

    #[test]
    fn 体力过低直接算状态差() {
        let s = reduce(
            &PetState {
                feeling: 100.0,
                strength: POOR_STRENGTH - 1.0,
                ..Default::default()
            },
            &Event::Tick { minutes: 0.0 },
        );
        assert_eq!(s.mood, Mood::PoorCondition, "体力见底时心情再好也是状态差");
    }

    #[test]
    fn 序列化的字段名和前端契约一致() {
        let json = serde_json::to_string(&PetState::default()).unwrap();
        for key in [
            "activity",
            "mood",
            "strength",
            "feeling",
            "hunger",
            "thirst",
            "updatedAt",
        ] {
            assert!(json.contains(key), "缺字段 {key}：{json}");
        }
        assert!(json.contains("\"idle\""));
        assert!(json.contains("\"nomal\""));
    }

    /// 按 lib.rs 心跳的真实调用方式跑一遍：每秒 tick，进入吃喝后演满 N 拍再 Consumed。
    /// 这条覆盖的是「状态机 + 心跳」合起来的契约，不只是 reduce 本身。
    #[test]
    fn 完整一轮_饿了会自己吃完再回到空闲() {
        let mut s = PetState {
            hunger: HUNGRY_AT + 0.5,
            ..Default::default()
        };
        let mut consuming = 0;
        let mut ate = false;
        for _ in 0..600 {
            // 10 分钟
            s = reduce(
                &s,
                &Event::Tick {
                    minutes: 1.0 / 60.0,
                },
            );
            if matches!(s.activity, Activity::Eating | Activity::Drinking) {
                ate = true;
                consuming += 1;
                if consuming >= 3 {
                    s = reduce(&s, &Event::Consumed);
                    consuming = 0;
                }
            } else {
                consuming = 0;
            }
        }
        assert!(ate, "十分钟里应该至少吃/喝过一次");
        assert_eq!(s.activity, Activity::Idle, "吃完该回空闲，不该卡在吃");
        assert!(s.hunger > HUNGRY_AT, "补完应该高于阈值");
    }

    #[test]
    fn 秒级tick和分钟级tick等价() {
        let by_min = run(PetState::default(), 10);
        let mut by_sec = PetState::default();
        for _ in 0..600 {
            by_sec = reduce(
                &by_sec,
                &Event::Tick {
                    minutes: 1.0 / 60.0,
                },
            );
        }
        assert!((by_min.hunger - by_sec.hunger).abs() < 0.5);
    }
}
