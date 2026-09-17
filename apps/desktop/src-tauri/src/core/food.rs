//! 食物货架：她能买什么、买了回多少。
//!
//! 数据不在这里——123 项食物是 `build-assets` 从原版 `food/*.lps` 转出来的，躺在
//! Body 那边的 manifest 里。Body 启动时用 `set_food_catalog` 推给 Core，和推命中掩码
//! 一个路子。Core 只负责「按需求和钱包挑一样」，挑完把 id 塞进 `action.food`，
//! Body 照着渲染那一个精灵——而不是自己随便抓一个。

use serde::{Deserialize, Serialize};

/// 与 packages/shared/src/manifest.ts 的 `FoodItem` 一一对应
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FoodItem {
    pub id: String,
    pub name: String,
    /// eat / drink / gift —— 配哪段夹心动画
    pub graph: String,
    /// Meal / Snack / Drink / Drug / Gift / Functional
    #[serde(rename = "type")]
    pub kind: String,
    pub strength: f32,
    /// 回多少饱腹
    pub strength_food: f32,
    /// 回多少水
    pub strength_drink: f32,
    pub feeling: f32,
    pub health: f32,
    pub price: f32,
}

/// 挑食物时优先满足哪一项
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Hunger,
    Thirst,
    /// 心情不好，买点好吃的哄自己——所以挑的东西会和单纯充饥时不一样
    Mood,
}

#[derive(Debug, Default, Clone)]
pub struct FoodShelf {
    items: Vec<FoodItem>,
}

impl FoodShelf {
    /// Startup must not depend on the pet WebView finishing its asynchronous asset load.
    pub fn bundled() -> Self {
        let items = serde_json::from_str(include_str!("../../food-catalog.json"))
            .expect("bundled food catalog must contain valid FoodItem records");
        Self { items }
    }

    pub fn gifts(&self) -> Vec<FoodItem> {
        self.items.iter().filter(|item| item.graph == "gift" && item.kind == "Gift")
            .cloned().collect()
    }

    pub fn set(&mut self, items: Vec<FoodItem>) {
        self.items = items;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, id: &str) -> Option<&FoodItem> {
        self.items.iter().find(|f| f.id == id)
    }

    /// 随机挑一件（礼物用）。`seed` 由调用方给——状态机要保持纯函数，
    /// 随机数不能长在里面
    pub fn random(&self, graph: &str, seed: u64) -> Option<&FoodItem> {
        let pool: Vec<&FoodItem> = self
            .items
            .iter()
            .filter(|f| f.graph == graph && f.kind != "Drug")
            .collect();
        if pool.is_empty() {
            return None;
        }
        Some(pool[(seed as usize) % pool.len()])
    }

    /// 按需求和钱包挑一样。买不起就返回 None，调用方自己决定怎么办。
    ///
    /// `graph` 是「吃」还是「喝」，和夹心动画对应。
    /// 排掉 `Drug`——那是原版用来救存档的药，`太阳系` 一口下去体力 −100。
    pub fn pick(&self, graph: &str, need: Need, budget: f32) -> Option<&FoodItem> {
        self.items
            .iter()
            .filter(|f| f.graph == graph && f.kind != "Drug" && f.price <= budget)
            .max_by(|a, b| {
                score(a, need)
                    .partial_cmp(&score(b, need))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// 性价比：主要需求每花一块钱能换回多少。白送的按「不打折」算
fn score(f: &FoodItem, need: Need) -> f32 {
    let gain = match need {
        Need::Hunger => f.strength_food,
        Need::Thirst => f.strength_drink,
        Need::Mood => f.feeling,
    };
    if gain <= 0.0 {
        return f32::MIN;
    }
    gain / f.price.max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_gifts_available_before_webview_starts() {
        let shelf = FoodShelf::bundled();
        assert_eq!(shelf.len(), 123);
        assert_eq!(shelf.gifts().len(), 20);
        for seed in 0..20 {
            let gift = shelf.random("gift", seed).unwrap();
            assert_eq!(gift.kind, "Gift");
            assert!(!gift.name.is_empty());
            assert!(shelf.get(&gift.id).is_some());
        }
    }

    fn item(id: &str, graph: &str, kind: &str, food: f32, drink: f32, feel: f32, price: f32) -> FoodItem {
        FoodItem {
            id: id.into(),
            name: id.into(),
            graph: graph.into(),
            kind: kind.into(),
            strength: 0.0,
            strength_food: food,
            strength_drink: drink,
            feeling: feel,
            health: 0.0,
            price,
        }
    }

    fn shelf() -> FoodShelf {
        let mut s = FoodShelf::default();
        s.set(vec![
            item("cheap", "eat", "Snack", 20.0, 0.0, 1.0, 5.0),
            item("feast", "eat", "Meal", 60.0, 0.0, 2.0, 100.0),
            item("cake", "eat", "Snack", 10.0, 0.0, 50.0, 30.0),
            item("water", "drink", "Drink", 0.0, 40.0, 0.0, 2.0),
            item("poison", "eat", "Drug", 90.0, 0.0, 0.0, 1.0),
        ]);
        s
    }

    #[test]
    fn 随机挑礼物不会挑到空也不会挑到药() {
        let mut s = shelf();
        s.set(vec![
            item("g1", "gift", "Gift", 0.0, 0.0, 100.0, 500.0),
            item("g2", "gift", "Gift", 0.0, 0.0, 200.0, 900.0),
            item("bad", "gift", "Drug", 0.0, 0.0, 0.0, 1.0),
        ]);
        let mut seen = std::collections::HashSet::new();
        for seed in 0..20u64 {
            let f = s.random("gift", seed).unwrap();
            assert_ne!(f.kind, "Drug");
            seen.insert(f.id.clone());
        }
        assert!(seen.len() > 1, "二十次都挑到同一件，等于没随机");
    }

    #[test]
    fn 货架空了送不出礼物() {
        assert!(FoodShelf::default().random("gift", 0).is_none());
    }

    #[test]
    fn 买不起就返回空() {
        assert!(shelf().pick("eat", Need::Hunger, 1.0).is_none());
    }

    #[test]
    fn 饿的时候挑性价比高的充饥() {
        let s = shelf();
        let f = s.pick("eat", Need::Hunger, 999.0).unwrap();
        assert_eq!(f.id, "cheap", "20 饱腹 / 5 块 比 60 / 100 划算");
    }

    #[test]
    fn 心情差的时候挑能哄自己的() {
        let s = shelf();
        let f = s.pick("eat", Need::Mood, 999.0).unwrap();
        assert_eq!(f.id, "cake", "同样的钱，这时候该买蛋糕不是买饭");
    }

    #[test]
    fn 渴了只在饮料里挑() {
        let s = shelf();
        let f = s.pick("drink", Need::Thirst, 999.0).unwrap();
        assert_eq!(f.graph, "drink");
    }

    #[test]
    fn 永远不会去吃药() {
        let s = shelf();
        for budget in [1.0, 10.0, 999.0] {
            let picked = s.pick("eat", Need::Hunger, budget);
            assert!(
                picked.is_none_or(|f| f.kind != "Drug"),
                "挑到药了（预算 {budget}）"
            );
        }
    }

    #[test]
    fn 钱包决定档次() {
        let s = shelf();
        // 只买得起便宜的
        assert_eq!(s.pick("eat", Need::Hunger, 6.0).unwrap().id, "cheap");
        // 有钱了也还是挑性价比，不是挑最贵
        assert_eq!(s.pick("eat", Need::Hunger, 999.0).unwrap().id, "cheap");
    }
}
