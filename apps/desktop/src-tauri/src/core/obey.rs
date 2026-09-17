//! 服从判定（docs/07 roadmap 2.8）。
//!
//! 用户让她做一件事，她**不一定照做**。这是「有身体的角色」和「遥控车」的分界线：
//! 遥控车收到指令就执行；人会先掂量一下——我现在饿不饿、心情好不好、跟你熟不熟、
//! 你让我做的这件事本身有多难受。
//!
//! 所以这里不是 `if 用户说了 then 执行`，而是一个**对数几率模型**：
//!
//! ```text
//!   logit = 基线(乖巧懂事)
//!         + 好感度贡献        慢变量，天级，决定「关系底色」
//!         + 心情贡献          快变量，分钟级，决定「此刻愿不愿意」
//!         - 生理冲突          你让我做的事解决不了我现在最急的需求
//!         - 动作代价          这件事本身让我掉多少心情
//!   p(服从) = σ(logit)
//! ```
//!
//! 用 logistic 而不是硬阈值，是因为要的正是「有时听有时不听」；而且 σ 的输入就是
//! log-odds，每个系数都能单独解释、单独调，不会互相纠缠。
//!
//! **拒绝的理由是白送的**：`conflict` 里贡献最大的那一项就是她拒绝的原因，
//! 直接映射到台词，不需要生成。这和 `ActionRef.reason` 是同一个套路。
//!
//! 随机数从外面喂（`roll`），和 `Event::Tick` 喂时间是同一个理由：
//! 保持纯函数，测试里能精确控制「这次掷出 0.9」。

use serde::{Deserialize, Serialize};

use super::actions::ActionDef;
use super::state_machine::{
    meets, PetState, EXHAUSTED, HUNGRY, MISERABLE, SAD, STARVING, THIRSTY,
};

/* --- 参数。全是「策略」，集中在这里方便调手感 --- */

/// 基线：她是个乖巧懂事的女儿，什么都不缺的时候默认就偏向听话。
/// σ(1.4) ≈ 0.80
const BASE: f32 = 1.4;
/// 好感度满 → +1.0，好感度归零 → −1.0
const K_AFF: f32 = 2.0;
/// 心情满 → +0.8，心情归零 → −0.8
const K_MOOD: f32 = 1.6;
/// 生理冲突的分量。调大 = 更「不舒服就谁的话都不听」
const K_CONFLICT: f32 = 3.0;
/// 动作代价的归一化尺度：全程掉 40 点心情算 1.0 的代价
const COST_SCALE: f32 = 40.0;
/// 反复逼她的代价。压力每 +1，服从的 log-odds 掉这么多
const K_PRESSURE: f32 = 0.6;

/// 她被逼到什么程度就开始有负面反馈
pub const PRESSURE_HURT: f32 = 2.0;

/// 她为什么不干。取 `conflict` 里贡献最大的那一项
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Refusal {
    /// 根本做不到：等级不够 / 前置条件不满足。这不是不听话
    Impossible,
    /// 压根没有这件事可做
    Unknown,
    Hungry,
    Thirsty,
    Tired,
    Sad,
    /// 没有哪一项特别突出，就是不太想
    Reluctant,
}

impl Refusal {
    /// 拒绝的台词。理由 → 话，一一对应，不需要生成模型
    pub fn line(self) -> &'static str {
        match self {
            Refusal::Impossible => "我现在还做不来这个……",
            Refusal::Unknown => "这个我不会呀。",
            Refusal::Hungry => "我好饿，吃完饭再去好不好？",
            Refusal::Thirsty => "我先喝口水，马上就去。",
            Refusal::Tired => "我有点累了，让我歇一会儿嘛。",
            Refusal::Sad => "我现在心情不太好……等下再说好不好。",
            Refusal::Reluctant => "唔……我不太想动，就一会儿。",
        }
    }
}

/// 判定结果。`p` 一并带出来，是为了让面板能显示「刚才那次有 63% 会听」——
/// 概率系统不给人看就成了玄学
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub obey: bool,
    /// 服从概率，0–1
    pub p: f32,
    /// 她要做/被要求的那件事
    pub action: String,
    pub refusal: Option<Refusal>,
    /// 直接能显示在气泡里的一句话
    pub say: String,
}

impl Verdict {
    pub fn refused(action: &str, r: Refusal, p: f32) -> Self {
        Self {
            obey: false,
            p,
            action: action.into(),
            refusal: Some(r),
            say: r.line().into(),
        }
    }
}

/// 某个维度「有多急」，0（不急）–1（见底）。
/// 分母用的是「该去处理它」的阈值，不是 100——饱腹 60 不叫饿
fn urgency(v: f32, threshold: f32) -> f32 {
    ((threshold - v) / threshold).clamp(0.0, 1.0)
}

/// 这个动作全程能把某个维度补回多少，归一化到 0–1。
/// 消耗它的动作（`per_min` 为负）一律 0：工作解决不了饿
fn relief(per_min: f32, duration: f32, threshold: f32) -> f32 {
    // duration = 0 是「一直做下去」（发呆、玩），按一小时估
    let dur = if duration > 0.0 { duration } else { 60.0 };
    (per_min * dur / threshold).clamp(0.0, 1.0)
}

/// 做这件事本身有多难受：只看它掉多少心情。
/// 完全数据驱动——不需要在 actions.toml 里另加「厌恶度」字段，
/// `per_min.feeling` 已经说明了一切
fn cost(a: &ActionDef) -> f32 {
    let dur = if a.duration > 0.0 { a.duration } else { 60.0 };
    (-a.per_min.feeling * dur / COST_SCALE).max(0.0)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// 掂量一次：用户要她做 `a`，她干不干。
///
/// `roll` 是外部喂进来的 0–1 随机数；`pressure` 是最近被拒绝后又反复要求的次数。
pub fn judge(a: &ActionDef, s: &PetState, pressure: f32, roll: f32) -> Verdict {
    if !meets(a, s) {
        return Verdict::refused(&a.name, Refusal::Impossible, 0.0);
    }

    // 四个维度：现在多急 × 这件事有多帮不上忙 × 权重
    let dims: [(Refusal, f32, f32, f32); 4] = [
        (
            Refusal::Hungry,
            urgency(s.hunger, HUNGRY),
            relief(a.per_min.hunger, a.duration, HUNGRY),
            1.0,
        ),
        (
            Refusal::Thirsty,
            urgency(s.thirst, THIRSTY),
            relief(a.per_min.thirst, a.duration, THIRSTY),
            1.0,
        ),
        (
            Refusal::Tired,
            urgency(s.strength, EXHAUSTED * 2.0),
            relief(a.per_min.strength, a.duration, EXHAUSTED * 2.0),
            1.0,
        ),
        (
            // 心情差的时候抗拒弱一点：让她去玩正好对症
            Refusal::Sad,
            urgency(s.feeling, SAD),
            relief(a.per_min.feeling, a.duration, SAD),
            0.8,
        ),
    ];

    let mut conflict = 0.0;
    let mut worst = (Refusal::Reluctant, 0.0f32);
    for (why, urg, rel, w) in dims {
        let c = urg * (1.0 - rel) * w;
        conflict += c;
        if c > worst.1 {
            worst = (why, c);
        }
    }

    let logit = BASE
        + K_AFF * (s.affection / 100.0 - 0.5)
        + K_MOOD * (s.feeling / 100.0 - 0.5)
        - K_CONFLICT * conflict
        - cost(a)
        - K_PRESSURE * pressure;
    let p = sigmoid(logit);

    if roll < p {
        Verdict {
            obey: true,
            p,
            action: a.name.clone(),
            refusal: None,
            say: obey_line(s),
        }
    } else {
        Verdict::refused(&a.name, worst.0, p)
    }
}

/// 答应的时候说什么。心情好就更乐意一点——同一件事，语气不一样
fn obey_line(s: &PetState) -> String {
    if s.feeling >= 70.0 {
        "好呀，这就去！"
    } else if s.feeling < MISERABLE + 10.0 || s.strength < STARVING {
        "……好吧，我去。"
    } else {
        "嗯，知道啦。"
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::actions::Catalog;

    fn st() -> PetState {
        PetState::default()
    }

    /// roll = 0 必服从（只要 p > 0），roll = 1 必拒绝。
    /// 用它把随机性从测试里摘掉，只测「概率有多大」
    fn p_of(id: &str, s: &PetState) -> f32 {
        let cat = Catalog::load();
        judge(cat.get(id).unwrap(), s, 0.0, 1.0).p
    }

    #[test]
    fn 状态良好时她基本听话() {
        let mut s = st();
        s.feeling = 80.0;
        s.affection = 50.0;
        assert!(p_of("work_copy", &s) > 0.7, "p = {}", p_of("work_copy", &s));
    }

    #[test]
    fn 饿的时候不会去工作() {
        let mut s = st();
        s.hunger = 8.0;
        let p = p_of("work_copy", &s);
        assert!(p < 0.3, "饿着还有 {p} 的概率去上班");
    }

    #[test]
    fn 饿的时候让她吃饭反而更听话() {
        let mut s = st();
        s.hunger = 8.0;
        assert!(p_of("meal", &s) > p_of("work_copy", &s) + 0.4);
    }

    #[test]
    fn 拒绝的理由指向真正的短板() {
        let cat = Catalog::load();
        let mut s = st();
        // 18 是刻意挑的：在 work_copy 的 min_thirst(15) 之上——否则判的是
        // 「做不到」而不是「不想做」，测不到理由归因这条路
        s.thirst = 18.0;
        let v = judge(cat.get("work_copy").unwrap(), &s, 0.0, 1.0);
        assert_eq!(v.refusal, Some(Refusal::Thirsty));
        assert!(v.say.contains("水"));
    }

    #[test]
    fn 好感度拉高服从率() {
        let mut lo = st();
        lo.affection = 0.0;
        let mut hi = st();
        hi.affection = 100.0;
        assert!(p_of("work_copy", &hi) - p_of("work_copy", &lo) > 0.2);
    }

    #[test]
    fn 反复逼她会越来越不听() {
        let cat = Catalog::load();
        let s = st();
        let a = cat.get("work_copy").unwrap();
        let calm = judge(a, &s, 0.0, 1.0).p;
        let pushed = judge(a, &s, 3.0, 1.0).p;
        assert!(pushed < calm - 0.2, "{calm} → {pushed}");
    }

    #[test]
    fn 做不到的事不算不听话() {
        let cat = Catalog::load();
        let mut s = st();
        s.strength = 2.0;
        // 体力见底时高强度的活过不了 requires
        let hard = cat
            .all()
            .iter()
            .find(|a| a.requires.min_strength > 10.0)
            .expect("动作表里应有带体力门槛的活");
        let v = judge(hard, &s, 0.0, 0.0);
        assert_eq!(v.refusal, Some(Refusal::Impossible));
        assert!(!v.obey);
    }

    #[test]
    fn 让她休息比让她工作容易答应() {
        let s = st();
        assert!(p_of("rest", &s) > p_of("work_copy", &s));
    }

    #[test]
    fn roll_决定这一次的结果() {
        let cat = Catalog::load();
        let mut s = st();
        s.feeling = 80.0;
        let a = cat.get("rest").unwrap();
        assert!(judge(a, &s, 0.0, 0.01).obey);
        assert!(!judge(a, &s, 0.0, 0.999).obey);
    }
}
