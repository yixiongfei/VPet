//! 用户偏好权重（docs/07 roadmap 2.10）。
//!
//! 「多工作一点」「少玩会儿」这类要求，和「现在去睡觉」不是一回事：
//! 前者是**持续的倾向**，后者是**一次性的请求**（那条走 `obey::judge`）。
//!
//! 关键的设计约束：偏好**不是优先级阶梯上的新一级**。
//! 它只在「作息」这一层里重排和越界，排不过「生理急需」，也排不过「到点该睡该吃」。
//! 所以「她饿了就不听你的」「让她多工作也不会饿死自己」这两条是**结构保证的**，
//! 不需要另写规则去兜——这正是当初不肯把状态机换成一张阈值表的理由。
//!
//! 每条偏好带半衰期：你随口说的一句「今天多干点」，不该绑架她一辈子。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 正到这个程度，她会越出时段主动做（「多工作」→ 晚上也想干活）
pub const SPILL: f32 = 0.5;
/// 负到这个程度，作息层直接跳过这一类（「少玩点」）
pub const SUPPRESS: f32 = -0.5;
/// 衰减到这个绝对值以下就当没有了，顺手从表里删掉
const NEGLIGIBLE: f32 = 0.05;
/// 没说多久就按这个半衰期算：两小时。一句随口的话，管半天
pub const DEFAULT_HALF_LIFE: f32 = 120.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bias {
    /// 正 = 多做，负 = 少做
    pub weight: f32,
    /// 半衰期（分钟）
    pub half_life: f32,
    /// 已经过去多少分钟
    pub age: f32,
}

impl Bias {
    /// 此刻的有效权重。指数衰减——用半衰期而不是「还剩多少分钟」，
    /// 是因为衰减要平滑：到点突然归零会让她的行为在某一秒突变
    pub fn current(&self) -> f32 {
        if self.half_life <= 0.0 {
            return self.weight;
        }
        self.weight * 0.5f32.powf(self.age / self.half_life)
    }
}

/// tag → 偏好。一个 tag 只留一条：再说一次「多工作」是覆盖，不是叠加
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Biases(HashMap<String, Bias>);

impl Biases {
    pub fn set(&mut self, tag: &str, weight: f32, half_life: f32) {
        self.0.insert(
            tag.to_string(),
            Bias {
                weight: weight.clamp(-2.0, 2.0),
                half_life: if half_life > 0.0 { half_life } else { DEFAULT_HALF_LIFE },
                age: 0.0,
            },
        );
    }

    pub fn clear(&mut self, tag: &str) -> bool {
        self.0.remove(tag).is_some()
    }

    pub fn clear_all(&mut self) {
        self.0.clear();
    }

    pub fn weight(&self, tag: &str) -> f32 {
        self.0.get(tag).map(Bias::current).unwrap_or(0.0)
    }

    /// 时间流逝。衰减到可以忽略的直接删掉，免得表越积越长
    pub fn decay(&mut self, minutes: f32) {
        for b in self.0.values_mut() {
            b.age += minutes;
        }
        self.0.retain(|_, b| b.current().abs() >= NEGLIGIBLE);
    }

    /// 权重最高的那一条（只看正的）。用来回答「什么都不缺的时候做点啥」
    pub fn strongest(&self) -> Option<(&str, f32)> {
        self.0
            .iter()
            .map(|(t, b)| (t.as_str(), b.current()))
            .filter(|(_, w)| *w >= SPILL)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// 给面板看：现在生效的偏好，按权重从大到小
    pub fn list(&self) -> Vec<BiasView> {
        let mut v: Vec<_> = self
            .0
            .iter()
            .map(|(tag, b)| BiasView {
                tag: tag.clone(),
                weight: b.current(),
                half_life: b.half_life,
                age: b.age,
            })
            .collect();
        v.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiasView {
    pub tag: String,
    pub weight: f32,
    pub half_life: f32,
    pub age: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 半衰期到了刚好剩一半() {
        let mut b = Biases::default();
        b.set("work", 1.0, 60.0);
        assert!((b.weight("work") - 1.0).abs() < 1e-6);
        b.decay(60.0);
        assert!((b.weight("work") - 0.5).abs() < 1e-3);
        b.decay(60.0);
        assert!((b.weight("work") - 0.25).abs() < 1e-3);
    }

    #[test]
    fn 衰减干净了就自动删掉() {
        let mut b = Biases::default();
        b.set("play", -1.0, 30.0);
        b.decay(300.0);
        assert!(b.is_empty(), "衰减到可忽略还占着表：{:?}", b.list());
    }

    #[test]
    fn 同一个_tag_是覆盖不是叠加() {
        let mut b = Biases::default();
        b.set("work", 1.0, 60.0);
        b.set("work", 0.3, 60.0);
        assert!((b.weight("work") - 0.3).abs() < 1e-6);
        assert_eq!(b.list().len(), 1);
    }

    #[test]
    fn 权重有上限() {
        let mut b = Biases::default();
        b.set("work", 999.0, 60.0);
        assert!(b.weight("work") <= 2.0, "用户喊得再大声也不能压过生理");
    }

    #[test]
    fn strongest_只看正偏置() {
        let mut b = Biases::default();
        b.set("play", -1.0, 60.0);
        assert!(b.strongest().is_none(), "「少玩点」不该被当成「去玩」");
        b.set("study", 1.0, 60.0);
        assert_eq!(b.strongest().map(|(t, _)| t), Some("study"));
    }

    #[test]
    fn 没写半衰期给个默认值() {
        let mut b = Biases::default();
        b.set("work", 1.0, 0.0);
        b.decay(DEFAULT_HALF_LIFE);
        assert!((b.weight("work") - 0.5).abs() < 1e-3);
    }
}
