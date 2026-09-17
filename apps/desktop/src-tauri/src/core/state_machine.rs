//! 宠物状态机（docs/03 §7、docs/05 §5）。
//!
//! 三层分工，谁都不越界：
//!
//! ```text
//!   状态机（本文件）   状态与合法转移、数值不变量、什么时候该换一件事做
//!        ↑ 仲裁
//!   驱力（decide）     生理急需 > 作息到点 > 心情/经济需求 > 兜底
//!        ↑ 查
//!   数据表（actions.toml）  每件事的消耗 / 收益 / 门槛 / 冷却 / 时段
//! ```
//!
//! `reduce` 是纯函数：不读时钟、不做 IO。时间和「现在几点」都从外面以
//! `Event::Tick { minutes, hour }` 喂进来——所以「凌晨三点她在干嘛」「饭点会不会吃饭」
//! 这类行为可以在单元测试里瞬间验证，不用真等到半夜。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::actions::{ActionDef, Catalog};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl Activity {
    /// 过场动画，不该被新的决策打断（正在把饭往嘴里送，不能突然去上班）
    pub fn is_transient(self) -> bool {
        matches!(self, Activity::Eating | Activity::Drinking)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Happy,
    Nomal,
    PoorCondition,
    Ill,
}

/// 正在做的事。Body 用 `graph` 挑动画，也能直接显示「正在：文案」
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRef {
    pub id: String,
    pub name: String,
    pub graph: String,
    /// 为什么选了它。让数值变动有迹可循
    pub reason: String,
}

/// 发给 Body 的 `pet:state` 载荷，字段与 packages/shared/src/pet.ts 的 zod schema 一一对应
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub activity: Activity,
    pub mood: Mood,
    pub strength: f32,
    pub feeling: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub money: f32,
    pub exp: f32,
    pub level: u32,
    pub action: Option<ActionRef>,
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
            money: 0.0,
            exp: 0.0,
            level: 0,
            action: None,
            updated_at: 0,
        }
    }
}

/// Core 内部的完整状态 = 发给 Body 的那部分 + 记账。
/// 记账不进线上契约：Body 不需要知道「这一轮已经赚了多少」。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pet {
    pub state: PetState,
    /// 当前动作已经做了多少分钟
    pub elapsed: f32,
    /// 这一轮已赚到的钱/经验，用来算完成奖励
    pub earned: f32,
    /// 动作 id → 还要等多少分钟才能再做
    pub cooldowns: HashMap<String, f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    Head,
    Body,
    Raise,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// 时间流逝。`hour` 是当前钟点（0–24），决定「到点该做什么」
    Tick { minutes: f32, hour: f32 },
    Touched(Touch),
    /// 调试用：直接改数值，用来验证阈值行为
    Patch {
        strength: Option<f32>,
        feeling: Option<f32>,
        hunger: Option<f32>,
        thirst: Option<f32>,
        money: Option<f32>,
    },
}

/* --- 仲裁用的阈值。这些是「策略」，不是「数据」，所以留在代码里 --- */

/// 低到这个程度就压过作息：半夜也会爬起来吃
const STARVING: f32 = 20.0;
const PARCHED: f32 = 20.0;
const EXHAUSTED: f32 = 15.0;
/// 心情崩到这个程度，再忙也得歇一会儿——人不会把自己磨到彻底麻木还接着干
const MISERABLE: f32 = 15.0;
/// 没到饭点但也该补一口了
const HUNGRY: f32 = 35.0;
const THIRSTY: f32 = 35.0;
/// 心情低于此就想找点乐子
const SAD: f32 = 40.0;
/// 钱少于此就该去挣了
const BROKE: f32 = 80.0;

const HAPPY_FEELING: f32 = 70.0;
const NOMAL_FEELING: f32 = 40.0;
const POOR_STRENGTH: f32 = 20.0;

const FEELING_HEAD: f32 = 1.0;
const FEELING_BODY: f32 = 0.5;
/// 被拎起来不太舒服，但也不惩罚（docs/01：情绪只正向放大）
const FEELING_RAISE: f32 = 0.0;
const STRENGTH_PER_TOUCH: f32 = 0.2;

/// 关掉期间最多按这么久补算。
///
/// 产品判断不是技术判断：离线衰减的目的是「感觉时间过去了」，不是「你冷落了我」。
/// 满饱腹跑到零只要几小时，上限必须明显短于它，否则每天早上打开都是一只饿到脱力的
/// 宠物——那是愧疚感机制，docs/01 写明情绪引擎只正向放大、不惩罚。
const MAX_CATCHUP_MIN: f32 = 2.0 * 60.0;

/// 经验到等级。等级解锁更赚钱的活，也直接给一点收入加成——
/// 「学习提高赚钱效率」这条因果链要看得见。
pub fn level_for(exp: f32) -> u32 {
    (exp.max(0.0) / 100.0).sqrt() as u32
}

/// 干活效率，移植自原版 MainLogic.cs:280-335：吃饱喝足才有高效率。
/// 返回值域 0.4–1.0
pub fn efficiency(s: &PetState) -> f32 {
    let one = |v: f32| {
        if v <= 25.0 {
            0.2 // 低状态低效率
        } else if v >= 60.0 {
            0.5
        } else {
            0.4
        }
    };
    one(s.hunger) + one(s.thirst)
}

/// 收入系数。原版：`addmoney = TimePass × MoneyBase × (2×efficiency − 0.5)`，
/// 我们额外乘一个等级加成，让「学习 → 赚更多」这条因果直接可见。
pub fn earn_multiplier(s: &PetState) -> f32 {
    let base = (2.0 * efficiency(s) - 0.5).max(0.0);
    base * (1.0 + s.level as f32 * 0.02)
}

/// 状态机主体。不设置 `updated_at`——那是时钟，由调用方填
pub fn reduce(cat: &Catalog, p: &Pet, e: &Event) -> Pet {
    let mut n = p.clone();
    match e {
        Event::Tick { minutes, hour } => tick(cat, &mut n, minutes.max(0.0), *hour),
        Event::Touched(t) => {
            n.state.feeling += match t {
                Touch::Head => FEELING_HEAD,
                Touch::Body => FEELING_BODY,
                Touch::Raise => FEELING_RAISE,
            };
            n.state.strength -= STRENGTH_PER_TOUCH;
            clamp(&mut n.state);
        }
        Event::Patch {
            strength,
            feeling,
            hunger,
            thirst,
            money,
        } => {
            if let Some(v) = strength {
                n.state.strength = *v;
            }
            if let Some(v) = feeling {
                n.state.feeling = *v;
            }
            if let Some(v) = hunger {
                n.state.hunger = *v;
            }
            if let Some(v) = thirst {
                n.state.thirst = *v;
            }
            if let Some(v) = money {
                n.state.money = *v;
            }
            clamp(&mut n.state);
        }
    }
    n.state.mood = derive_mood(&n.state);
    n
}

fn tick(cat: &Catalog, n: &mut Pet, minutes: f32, hour: f32) {
    // 1. 冷却倒数
    n.cooldowns.retain(|_, left| {
        *left -= minutes;
        *left > 0.0
    });

    // 2. 当前动作把数值往前推
    let current = n
        .state
        .action
        .as_ref()
        .and_then(|a| cat.get(&a.id))
        .unwrap_or_else(|| cat.fallback());
    let d = current.per_min;
    n.state.strength += d.strength * minutes;
    n.state.hunger += d.hunger * minutes;
    n.state.thirst += d.thirst * minutes;
    n.state.feeling += d.feeling * minutes;

    let mult = earn_multiplier(&n.state);
    let money = current.earns.money * mult * minutes;
    let exp = current.earns.exp * mult * minutes;
    n.state.money += money;
    n.state.exp += exp;
    n.earned += money + exp;

    let finished_id = current.id.clone();
    let finish_bonus = current.finish;
    let duration = current.duration;
    let cooldown = current.cooldown;
    let earns_money = current.earns.money > 0.0;

    clamp(&mut n.state);
    n.state.level = level_for(n.state.exp);
    n.elapsed += minutes;

    // 3. 做完了就结算，然后挑下一件事
    let done = duration > 0.0 && n.elapsed >= duration;
    if done {
        if finish_bonus > 0.0 && n.earned > 0.0 {
            let bonus = n.earned * finish_bonus;
            if earns_money {
                n.state.money += bonus;
            } else {
                n.state.exp += bonus;
                n.state.level = level_for(n.state.exp);
            }
        }
        if cooldown > 0.0 {
            n.cooldowns.insert(finished_id, cooldown);
        }
        n.state.action = None;
        n.elapsed = 0.0;
        n.earned = 0.0;
        clamp(&mut n.state);
    }

    // 4. 过场动画不许打断；其余情况没事做或该换事做时重新决策
    let busy = n.state.activity.is_transient() && !done;
    if !busy && (n.state.action.is_none() || should_switch(cat, n, hour)) {
        let chosen = decide(cat, &n.state, &n.cooldowns, hour);
        adopt(n, chosen);
    }
}

/// 正在做的事还合不合适。只在「明显不该再做了」时才打断，
/// 否则她会每一拍都换一件事做，看起来像多动症。
fn should_switch(cat: &Catalog, n: &Pet, hour: f32) -> bool {
    let Some(cur) = n.state.action.as_ref().and_then(|a| cat.get(&a.id)) else {
        return true;
    };
    // 生理急需和情绪崩溃压过一切
    if n.state.hunger < STARVING || n.state.thirst < PARCHED || n.state.strength < EXHAUSTED {
        return !cur.has_tag("need") && !cur.has_tag("sleep");
    }
    if n.state.feeling < MISERABLE {
        return !cur.has_tag("cheer");
    }
    // 排了时段的事，过了点就收手（下班了就别干了）
    if cur.is_scheduled() && !cur.fits_hour(hour) {
        return true;
    }
    // 前置条件掉下去了（累到干不动这活）
    !meets(cur, &n.state)
}

/// 挑下一件事。优先级从上到下，第一个满足的胜出。
///
/// 这个顺序就是「像人一样生活」的定义：
/// 生理急需 → 到点该做的 → 没到点但需要的 → 兜底发呆。
pub fn decide<'a>(
    cat: &'a Catalog,
    s: &PetState,
    cd: &HashMap<String, f32>,
    hour: f32,
) -> (&'a ActionDef, &'static str) {
    let ok = |a: &ActionDef| meets(a, s) && !cd.contains_key(&a.id);
    // 急需时连冷却都不管——真饿了不会因为「刚吃过」就饿着
    let urgent = |a: &ActionDef| meets(a, s);

    // 1. 生理急需：压过作息，半夜也会爬起来
    if s.hunger < STARVING {
        if let Some(a) = best(cat, "eat", urgent) {
            return (a, "饿得受不了");
        }
    }
    if s.thirst < PARCHED {
        if let Some(a) = best(cat, "drink", urgent) {
            return (a, "渴得受不了");
        }
    }
    if s.strength < EXHAUSTED {
        if let Some(a) = best(cat, "sleep", urgent) {
            return (a, "累垮了");
        }
    }
    if s.feeling < MISERABLE {
        if let Some(a) = best(cat, "cheer", |a| meets(a, s)) {
            return (a, "实在撑不住了");
        }
    }

    // 2. 作息：到点了就做该做的事，按「睡 > 吃 > 正事 > 玩」排
    for (tag, why) in [
        ("sleep", "到点睡觉"),
        ("eat", "到饭点了"),
        ("work", "上班时间"),
        ("study", "该学习了"),
        ("play", "该放松一下"),
    ] {
        if let Some(a) = best(cat, tag, |a| a.is_scheduled() && a.fits_hour(hour) && ok(a)) {
            return (a, why);
        }
    }

    // 3. 没到点，但身上的数值提出了要求
    if s.hunger < HUNGRY {
        if let Some(a) = best(cat, "eat", ok) {
            return (a, "有点饿了");
        }
    }
    if s.thirst < THIRSTY {
        if let Some(a) = best(cat, "drink", ok) {
            return (a, "有点渴了");
        }
    }
    if s.feeling < SAD {
        if let Some(a) = best(cat, "cheer", ok) {
            return (a, "心情不好，找点乐子");
        }
    }
    if s.money < BROKE {
        if let Some(a) = best(cat, "work", ok) {
            return (a, "钱不够了");
        }
    }

    // 4. 什么都不缺，发呆
    (cat.fallback(), "没什么事")
}

/// 同一标签里挑「最好」的一个：优先级看收益，收益一样看等级门槛高的
/// （门槛高的通常更强，能做就做）
fn best<'a>(
    cat: &'a Catalog,
    tag: &str,
    pred: impl Fn(&ActionDef) -> bool,
) -> Option<&'a ActionDef> {
    cat.all()
        .iter()
        .filter(|a| a.has_tag(tag) && pred(a))
        .max_by(|x, y| {
            let score = |a: &ActionDef| a.earns.money + a.earns.exp / 10.0 + a.requires.level as f32;
            score(x).partial_cmp(&score(y)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn meets(a: &ActionDef, s: &PetState) -> bool {
    let r = a.requires;
    s.level >= r.level
        && s.strength >= r.min_strength
        && s.hunger >= r.min_hunger
        && s.thirst >= r.min_thirst
        && s.strength <= r.max_strength
}

fn adopt(n: &mut Pet, (a, reason): (&ActionDef, &'static str)) {
    let same = n.state.action.as_ref().is_some_and(|c| c.id == a.id);
    if same {
        return;
    }
    n.state.activity = a.activity;
    n.state.action = Some(ActionRef {
        id: a.id.clone(),
        name: a.name.clone(),
        graph: a.graph.clone(),
        reason: reason.to_string(),
    });
    n.elapsed = 0.0;
    n.earned = 0.0;
}

/// 把「上次记录到现在」这段离线时间一次性补上。
///
/// 这是 `reduce` 保持纯函数换来的直接好处：补两小时和跑两小时走的是同一段代码。
pub fn catch_up(cat: &Catalog, p: &Pet, now_ms: i64, hour: f32) -> Pet {
    let minutes = (now_ms - p.state.updated_at).max(0) as f32 / 60_000.0;
    if minutes < 1.0 {
        return p.clone();
    }
    reduce(
        cat,
        p,
        &Event::Tick {
            minutes: minutes.min(MAX_CATCHUP_MIN),
            hour,
        },
    )
}

/// docs/05 §5：feeling ≥ 70 → Happy；≥ 40 → Nomal；< 40 或 strength < 20 → PoorCondition。
/// Ill 要「连续 3 天 PoorCondition」，得在流水上做跨天统计，留给 Phase 7 的 Mood Engine。
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
    s.money = s.money.max(0.0);
    s.exp = s.exp.max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        Catalog::load()
    }

    /// 跑 n 分钟，每分钟一拍，钟点跟着走
    fn run(cat: &Catalog, mut p: Pet, minutes: u32, start_hour: f32) -> Pet {
        for i in 0..minutes {
            let hour = (start_hour + i as f32 / 60.0) % 24.0;
            p = reduce(cat, &p, &Event::Tick { minutes: 1.0, hour });
        }
        p
    }

    fn acting(p: &Pet) -> String {
        p.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default()
    }

    #[test]
    fn 半夜会去睡觉() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 23.5);
        assert_eq!(p.state.activity, Activity::Sleeping, "凌晨该睡了");
    }

    #[test]
    fn 上班时间会去工作() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 10.0);
        assert_eq!(p.state.activity, Activity::Working, "上午十点该干活");
    }

    #[test]
    fn 到饭点会吃饭而不用等饿() {
        let c = cat();
        // 饱腹还满着，按阈值逻辑根本不会吃——但到点了就该吃
        let p = run(&c, Pet::default(), 1, 12.0);
        assert_eq!(p.state.activity, Activity::Eating, "中午十二点该吃饭");
        assert_eq!(acting(&p), "meal");
    }

    #[test]
    fn 饿极了半夜也会爬起来吃() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 5.0;
        let p = run(&c, p, 1, 3.0); // 凌晨三点
        assert_eq!(p.state.activity, Activity::Eating, "生理急需要压过作息");
        assert_eq!(acting(&p), "snack");
    }

    #[test]
    fn 晚上会学习或者玩不会上班() {
        let c = cat();
        let p = run(&c, Pet::default(), 5, 20.0);
        assert_ne!(p.state.activity, Activity::Working, "晚上八点不该还在上班");
    }

    #[test]
    fn 吃完饱腹真的回来了() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 10.0;
        let before = p.state.hunger;
        let p = run(&c, p, 4, 12.0);
        assert!(p.state.hunger > before + 20.0, "实际 {}", p.state.hunger);
    }

    #[test]
    fn 工作会赚钱() {
        let c = cat();
        let p = run(&c, Pet::default(), 30, 10.0);
        assert!(p.state.money > 0.0, "上午干了半小时该有收入");
    }

    #[test]
    fn 学习会涨经验和等级() {
        let c = cat();
        let mut p = Pet::default();
        p.state.money = 9999.0; // 不缺钱，免得跑去工作
        let p = run(&c, p, 40, 20.0);
        assert!(p.state.exp > 0.0, "晚上学习该涨经验");
        assert!(p.state.level > 0, "经验够了该升级");
    }

    #[test]
    fn 吃饱喝足才有高效率() {
        let full = PetState {
            hunger: 90.0,
            thirst: 90.0,
            ..Default::default()
        };
        let empty = PetState {
            hunger: 10.0,
            thirst: 10.0,
            ..Default::default()
        };
        assert!(efficiency(&full) > efficiency(&empty));
        assert!(earn_multiplier(&full) > earn_multiplier(&empty) * 2.0);
    }

    #[test]
    fn 等级越高赚得越多() {
        let low = PetState {
            level: 0,
            ..Default::default()
        };
        let high = PetState {
            level: 30,
            ..Default::default()
        };
        assert!(
            earn_multiplier(&high) > earn_multiplier(&low),
            "学习提高赚钱效率这条因果要成立"
        );
    }

    #[test]
    fn 等级不够的活干不了() {
        let c = cat();
        let s = PetState::default(); // level 0
        let cd = HashMap::new();
        let (a, _) = decide(&c, &s, &cd, 10.0);
        assert!(a.requires.level <= s.level, "挑中了做不了的活：{}", a.name);
    }

    #[test]
    fn 冷却中的动作不会被重复挑中() {
        let c = cat();
        let s = PetState::default();
        let mut cd = HashMap::new();
        let (first, _) = decide(&c, &s, &cd, 10.0);
        cd.insert(first.id.clone(), 30.0);
        let (second, _) = decide(&c, &s, &cd, 10.0);
        assert_ne!(first.id, second.id, "冷却没生效");
    }

    /// 一天二十四小时都排了事（这本身是对的——人本来就有作息），
    /// 所以要测「需求驱动」这一层，得先把当下排定的事按掉
    fn 只剩需求驱动(hour: f32) -> HashMap<String, f32> {
        let c = cat();
        c.all()
            .iter()
            .filter(|a| a.is_scheduled() && a.fits_hour(hour))
            .map(|a| (a.id.clone(), 60.0))
            .collect()
    }

    #[test]
    fn 心情差会去找乐子() {
        let c = cat();
        let s = PetState {
            feeling: 10.0,
            money: 9999.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert!(a.has_tag("cheer"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 没钱会去工作() {
        let c = cat();
        let s = PetState {
            money: 0.0,
            feeling: 80.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert!(a.has_tag("work"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 什么都不缺就发呆() {
        let c = cat();
        let s = PetState {
            money: 9999.0,
            feeling: 90.0,
            ..Default::default()
        };
        let (a, _) = decide(&c, &s, &只剩需求驱动(2.0), 2.0);
        assert_eq!(a.id, "rest");
    }

    #[test]
    fn 心情崩了会中断工作去歇一会儿() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 5.0;
        // 上午十点本该上班
        p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        let a = p.state.action.as_ref().unwrap();
        assert!(
            c.get(&a.id).unwrap().has_tag("cheer"),
            "心情只剩 5 还在 {}（{}）",
            a.name,
            a.reason
        );
    }

    #[test]
    fn 上班时段心情不会被磨到见底() {
        let c = cat();
        let mut p = Pet::default();
        let mut worst = 100.0f32;
        for i in 0..(10 * 60) {
            let hour = (9.0 + i as f32 / 60.0) % 24.0;
            p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour });
            worst = worst.min(p.state.feeling);
        }
        assert!(worst > 0.0, "从早九点干到晚七点，心情被磨到了 {worst}");
    }

    #[test]
    fn 半夜心情再差也是先睡觉() {
        // 作息压过心情调节：凌晨两点不该爬起来打游戏
        let c = cat();
        let s = PetState {
            feeling: 25.0, // 低于 SAD 但没到崩溃，作息说了算
            money: 9999.0,
            ..Default::default()
        };
        let (a, why) = decide(&c, &s, &HashMap::new(), 2.0);
        assert!(a.has_tag("sleep"), "挑了 {}（{why}）", a.name);
    }

    #[test]
    fn 每个决策都带理由() {
        let c = cat();
        let p = run(&c, Pet::default(), 3, 10.0);
        let reason = &p.state.action.as_ref().unwrap().reason;
        assert!(!reason.is_empty(), "数值变动要有迹可循");
    }

    #[test]
    fn 吃饭过程中不会被打断去上班() {
        let c = cat();
        let mut p = Pet::default();
        p.state.hunger = 5.0;
        // 上午十点，本该上班；但饿到不行，先吃
        p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
        assert_eq!(p.state.activity, Activity::Eating);
        // 下一拍还在吃，不能被上班抢走
        p = reduce(&c, &p, &Event::Tick { minutes: 0.5, hour: 10.0 });
        assert_eq!(p.state.activity, Activity::Eating, "过场动画不许打断");
    }

    #[test]
    fn 不会每一拍都换一件事做() {
        let c = cat();
        let mut p = run(&c, Pet::default(), 2, 10.0);
        let first = acting(&p);
        let mut switches = 0;
        for _ in 0..20 {
            p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour: 10.0 });
            if acting(&p) != first {
                switches += 1;
            }
        }
        assert!(switches < 5, "二十分钟换了 {switches} 次，像多动症");
    }

    #[test]
    fn 数值永远不越界() {
        let c = cat();
        let p = run(&c, Pet::default(), 3000, 0.0);
        let s = &p.state;
        for v in [s.strength, s.feeling, s.hunger, s.thirst] {
            assert!((0.0..=100.0).contains(&v), "越界 {v}");
        }
        assert!(s.money >= 0.0 && s.exp >= 0.0);
    }

    #[test]
    fn 跑一整天不会卡死在某个状态() {
        let c = cat();
        let mut p = Pet::default();
        let mut seen = std::collections::HashSet::new();
        for i in 0..(24 * 60) {
            let hour = (i as f32 / 60.0) % 24.0;
            p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour });
            seen.insert(p.state.activity);
        }
        assert!(seen.len() >= 4, "一整天只出现了 {:?}", seen);
        assert!(seen.contains(&Activity::Sleeping), "一天都没睡觉");
        assert!(seen.contains(&Activity::Eating), "一天都没吃饭");
    }

    #[test]
    fn 摸头涨心情() {
        let c = cat();
        let p = Pet::default();
        let after = reduce(&c, &p, &Event::Touched(Touch::Head));
        assert!(after.state.feeling > p.state.feeling);
        assert!(after.state.strength < p.state.strength, "摸也要消耗一点体力");
    }

    #[test]
    fn 提起不扣心情() {
        let c = cat();
        let p = Pet::default();
        let after = reduce(&c, &p, &Event::Touched(Touch::Raise));
        assert!(
            after.state.feeling >= p.state.feeling,
            "情绪引擎只正向放大（docs/01）"
        );
    }

    #[test]
    fn 心情决定表情() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 80.0;
        assert_eq!(
            reduce(&c, &p, &Event::Touched(Touch::Raise)).state.mood,
            Mood::Happy
        );
        p.state.feeling = 20.0;
        assert_eq!(
            reduce(&c, &p, &Event::Touched(Touch::Raise)).state.mood,
            Mood::PoorCondition
        );
    }

    #[test]
    fn 体力过低直接算状态差() {
        let c = cat();
        let mut p = Pet::default();
        p.state.feeling = 100.0;
        p.state.strength = POOR_STRENGTH - 1.0;
        let s = reduce(&c, &p, &Event::Touched(Touch::Raise));
        assert_eq!(s.state.mood, Mood::PoorCondition, "体力见底时心情再好也是状态差");
    }

    #[test]
    fn 离线期间照样会饿() {
        let c = cat();
        let p = Pet::default();
        let after = catch_up(&c, &p, 2 * 60 * 60 * 1000, 10.0);
        assert!(after.state.hunger < p.state.hunger, "关掉期间也该掉饱腹");
    }

    #[test]
    fn 离线太久不会把宠物饿死() {
        let c = cat();
        let p = Pet::default();
        let after = catch_up(&c, &p, 3 * 24 * 60 * 60 * 1000, 10.0);
        assert!(after.state.hunger > 0.0, "离线三天回来不该饿到脱力");
        assert_ne!(
            after.state.mood,
            Mood::PoorCondition,
            "长时间没开不该一上来就是坏心情（docs/01）"
        );
    }

    #[test]
    fn 刚存过就重启不会重复扣() {
        let c = cat();
        let mut p = Pet::default();
        p.state.updated_at = 1_000;
        p.state.hunger = 50.0;
        let after = catch_up(&c, &p, 1_500, 10.0);
        assert_eq!(after.state.hunger, 50.0);
    }

    #[test]
    fn 秒级tick和分钟级tick等价() {
        let c = cat();
        let by_min = run(&c, Pet::default(), 10, 3.0);
        let mut by_sec = Pet::default();
        for i in 0..600 {
            let hour = (3.0 + i as f32 / 3600.0) % 24.0;
            by_sec = reduce(
                &c,
                &by_sec,
                &Event::Tick {
                    minutes: 1.0 / 60.0,
                    hour,
                },
            );
        }
        assert!((by_min.state.hunger - by_sec.state.hunger).abs() < 1.0);
    }

    #[test]
    fn 序列化的字段名和前端契约一致() {
        let json = serde_json::to_string(&PetState::default()).unwrap();
        for key in [
            "activity", "mood", "strength", "feeling", "hunger", "thirst", "money", "exp", "level",
            "action", "updatedAt",
        ] {
            assert!(json.contains(key), "缺字段 {key}：{json}");
        }
        assert!(json.contains("\"idle\""));
        assert!(json.contains("\"nomal\""));
    }
}


#[cfg(test)]
mod 一天的生活 {
    use super::*;

    #[test]
    #[ignore = "不是断言，是拿来看她一天怎么过的"]
    fn 打印作息表() {
        let c = Catalog::load();
        let mut p = Pet::default();
        let mut last = String::new();
        for i in 0..(24 * 60) {
            let hour = i as f32 / 60.0;
            p = reduce(&c, &p, &Event::Tick { minutes: 1.0, hour });
            let now = p.state.action.as_ref().map(|a| a.id.clone()).unwrap_or_default();
            if now != last {
                let a = p.state.action.as_ref().unwrap();
                println!(
                    "{:02}:{:02}  {:<6} {:<12} 体力{:3.0} 心情{:3.0} 饱腹{:3.0} 水{:3.0} 钱{:5.0} Lv{}",
                    i / 60, i % 60, a.name, a.reason,
                    p.state.strength, p.state.feeling, p.state.hunger, p.state.thirst,
                    p.state.money, p.state.level
                );
                last = now;
            }
        }
    }
}
