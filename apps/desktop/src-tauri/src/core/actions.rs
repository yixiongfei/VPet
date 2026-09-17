//! 动作表：她能做的每件事，和做这件事对数值的影响。
//!
//! 表本身在 `actions.toml`，编译时嵌进二进制。所有数值集中在那一个文件里，
//! 这样「为什么心情掉了」永远能指到具体某一行，而不是散落在代码的 match 分支里。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::state_machine::Activity;

const TABLE: &str = include_str!("../../actions.toml");

/// 每分钟的数值变化。正数 = 涨，负数 = 掉
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
pub struct PerMin {
    #[serde(default)]
    pub strength: f32,
    #[serde(default)]
    pub hunger: f32,
    #[serde(default)]
    pub thirst: f32,
    #[serde(default)]
    pub feeling: f32,
}

/// 每分钟的产出。还要再乘效率系数
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
pub struct Earns {
    #[serde(default)]
    pub money: f32,
    #[serde(default)]
    pub exp: f32,
}

/// 前置条件。不满足就不会被选中。
///
/// `Default` 必须手写：`#[derive(Default)]` 会把 `max_strength` 设成 0.0，
/// 于是所有没写 `requires` 的条目（睡觉、吃饭、喝水）都会因为「体力 ≤ 0」永远选不中。
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct Requires {
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub min_strength: f32,
    #[serde(default)]
    pub min_hunger: f32,
    #[serde(default)]
    pub min_thirst: f32,
    /// 上限型条件：体力高于这个值就不做（睡觉用——不累就别睡）
    #[serde(default = "f32_max")]
    pub max_strength: f32,
}

fn f32_max() -> f32 {
    f32::MAX
}

impl Default for Requires {
    fn default() -> Self {
        Self {
            level: 0,
            min_strength: 0.0,
            min_hunger: 0.0,
            min_thirst: 0.0,
            max_strength: f32::MAX,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    pub id: String,
    pub name: String,
    pub activity: Activity,
    /// 对应的动画名，发给 Body 用来挑 clip
    pub graph: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 适合在一天中的哪些时段做，`[[9,12],[14,18]]`。空 = 任何时候都行。
    /// 跨夜写成 `[[23,7]]`
    #[serde(default)]
    pub hours: Vec<(f32, f32)>,
    /// 一次做多久（分钟）。0 = 不限时
    #[serde(default)]
    pub duration: f32,
    #[serde(default)]
    pub cooldown: f32,
    #[serde(default)]
    pub per_min: PerMin,
    #[serde(default)]
    pub earns: Earns,
    /// 做完时按这一轮已赚到的量额外再给一份
    #[serde(default)]
    pub finish: f32,
    #[serde(default)]
    pub requires: Requires,
}

impl ActionDef {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// 这个点适不适合做这件事。没写时段就是全天都行
    pub fn fits_hour(&self, hour: f32) -> bool {
        if self.hours.is_empty() {
            return true;
        }
        self.hours.iter().any(|&(from, to)| {
            if from <= to {
                hour >= from && hour < to
            } else {
                // 跨夜，如 23 → 7
                hour >= from || hour < to
            }
        })
    }

    /// 有没有排定的时段（用来区分「到点了」和「随时能做」）
    pub fn is_scheduled(&self) -> bool {
        !self.hours.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct RawTable {
    action: Vec<ActionDef>,
}

pub struct Catalog {
    actions: Vec<ActionDef>,
    by_id: HashMap<String, usize>,
}

impl Catalog {
    /// 读内置的动作表。表写错了就是开发期的 bug，直接 panic 比带着半张表跑下去好
    pub fn load() -> Self {
        Self::parse(TABLE).expect("actions.toml 解析失败")
    }

    pub fn parse(toml_src: &str) -> Result<Self, toml::de::Error> {
        let raw: RawTable = toml::from_str(toml_src)?;
        let by_id = raw
            .action
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.clone(), i))
            .collect();
        Ok(Self {
            actions: raw.action,
            by_id,
        })
    }

    pub fn get(&self, id: &str) -> Option<&ActionDef> {
        self.by_id.get(id).map(|&i| &self.actions[i])
    }

    pub fn all(&self) -> &[ActionDef] {
        &self.actions
    }

    /// 兜底动作：什么都不满足时做这个
    pub fn fallback(&self) -> &ActionDef {
        self.get("rest").unwrap_or(&self.actions[0])
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy() -> ActionDef {
        ActionDef {
            id: "x".into(),
            name: "x".into(),
            activity: Activity::Idle,
            graph: "default".into(),
            tags: vec![],
            hours: vec![],
            duration: 0.0,
            cooldown: 0.0,
            per_min: PerMin::default(),
            earns: Earns::default(),
            finish: 0.0,
            requires: Requires::default(),
        }
    }

    #[test]
    fn 内置表能解析() {
        let cat = Catalog::load();
        assert!(cat.all().len() >= 10, "实际 {} 条", cat.all().len());
    }

    #[test]
    fn id不重复() {
        let cat = Catalog::load();
        let mut ids: Vec<&str> = cat.all().iter().map(|a| a.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "动作表里有重复 id");
    }

    #[test]
    fn 每个标签组都至少有一条无门槛的() {
        let cat = Catalog::load();
        for tag in ["work", "study", "play"] {
            let entry = cat
                .all()
                .iter()
                .find(|a| a.has_tag(tag) && a.requires.level == 0);
            assert!(entry.is_some(), "{tag} 没有 0 级就能做的，新宠物会卡住");
        }
    }

    #[test]
    fn 兜底动作存在且不限时() {
        let cat = Catalog::load();
        let f = cat.fallback();
        assert_eq!(f.duration, 0.0, "兜底动作必须不限时，否则会没事可做");
        assert_eq!(f.requires.level, 0);
    }

    #[test]
    fn 工作赚钱学习涨经验玩回心情() {
        let cat = Catalog::load();
        for a in cat.all() {
            if a.has_tag("work") {
                assert!(a.earns.money > 0.0, "{} 是工作却不赚钱", a.name);
            }
            if a.has_tag("study") {
                assert!(a.earns.exp > 0.0, "{} 是学习却不涨经验", a.name);
            }
            if a.has_tag("cheer") {
                assert!(a.per_min.feeling > 0.0, "{} 标了 cheer 却不回心情", a.name);
            }
        }
    }

    #[test]
    fn 跨夜时段判定正确() {
        let night = ActionDef {
            hours: vec![(23.0, 7.0)],
            ..dummy()
        };
        assert!(night.fits_hour(23.5), "半夜该在窗口内");
        assert!(night.fits_hour(3.0));
        assert!(!night.fits_hour(12.0), "中午不该在夜间窗口内");
        assert!(!night.fits_hour(7.0), "上界是开区间");
    }

    #[test]
    fn 没写时段就是全天可做() {
        let any = dummy();
        for h in 0..24 {
            assert!(any.fits_hour(h as f32));
        }
        assert!(!any.is_scheduled());
    }

    #[test]
    fn 一日三餐三个窗口都命中() {
        let cat = Catalog::load();
        let meal = cat.get("meal").unwrap();
        for h in [7.5, 12.0, 18.0] {
            assert!(meal.fits_hour(h), "{h} 点该是饭点");
        }
        for h in [10.0, 15.0, 22.0] {
            assert!(!meal.fits_hour(h), "{h} 点不该是饭点");
        }
    }

    #[test]
    fn 没写前置条件的动作任何状态都能做() {
        // 踩过的坑：derive(Default) 把 max_strength 变成 0.0，
        // 于是睡觉/吃饭/喝水这些没写 requires 的条目永远选不中
        let cat = Catalog::load();
        for id in ["sleep", "meal", "drink", "snack", "rest"] {
            let a = cat.get(id).unwrap();
            assert_eq!(a.requires.max_strength, f32::MAX, "{id} 的体力上限被默认成 0 了");
        }
    }

    #[test]
    fn 表写错了会报错而不是默默吃掉() {
        assert!(Catalog::parse("[[action]]\nid = \"坏条目\"").is_err());
    }
}
