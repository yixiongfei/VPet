//! 全链路集成测试：真 ONNX 推理 → 真 sqlite-vec → 真检索排序。
//!
//! 单元测试证明每一段自己是对的，这一段证明**接缝**是对的——
//! 归一化的约定（L2 归一 ↔ `distance_metric=cosine`）、维度的约定
//! （模型实测维度 ↔ 向量表宽度）、生命周期的约定（删了要同时删向量）
//! 这三处都跨模块，任何一边单独看都正确，接错了才出事。
//!
//! 云端没有 HuggingFace，所以用**合成模型**：一张随机词向量表，外加把
//! 「作/息」两个字的行改成和「晚/上」相同——等于把语义打成桩。
//! 这样「稠密能召回字面够不着的东西」这一条就能被单独隔离出来验证，
//! 不用赌一个真模型的行为。
//!
//! 缺 `ORT_DYLIB_PATH` 或 `VPET_TEST_MODEL` 时整组跳过，不算失败。

#![cfg(all(test, feature = "_onnx"))]

use std::collections::HashMap;
use std::path::Path;

use super::db::Db;
use super::embed::{cosine, Embedder};
use super::memory::{
    draft, retrieve, retrieve_hybrid, BigramRetriever, MemoryItem, MemoryType, Retriever, Source,
    Status, TOP_K,
};

const NOW: i64 = 1_760_000_000_000;

fn setup() -> Option<(Embedder, Db)> {
    let dir = std::env::var_os("VPET_TEST_MODEL")?;
    std::env::var_os("ORT_DYLIB_PATH")?;
    let e = Embedder::load(Path::new(&dir)).expect("给了模型目录却加载失败");
    let db = Db::open_in_memory().unwrap();
    db.ensure_vec_table(e.dim(), e.version()).unwrap();
    Some((e, db))
}

fn write(db: &Db, e: &Embedder, id: &str, content: &str, kind: MemoryType) -> MemoryItem {
    let m = draft(
        id.into(), content.into(), kind, 70.0, 1.0,
        Source::UserExplicit, None, e.version(), NOW,
    );
    db.put_memory(&m).unwrap();
    db.put_embedding(id, &e.embed_one(content).unwrap(), e.version()).unwrap();
    m
}

fn dense_for(db: &Db, e: &Embedder, query: &str) -> HashMap<String, f32> {
    let q = e.embed_query(query).unwrap();
    db.knn(&q, 24).unwrap().into_iter().collect()
}

#[test]
fn 稠密检索救回字面结构上够不着的() {
    let Some((e, db)) = setup() else { return };
    write(&db, &e, "h", "晚上上班白天学习", MemoryType::Habit);

    let items = db.all_memories().unwrap();
    let r = BigramRetriever;

    // 「作息」和这条记忆零共字 —— 字面法的余弦恒为 0，不是精度问题是结构问题
    assert_eq!(r.similarity("作息", "晚上上班白天学习"), 0.0);
    assert!(retrieve(&items, "作息", &r, NOW, TOP_K).is_empty(), "字面居然召回了？");

    // 走一遍真链路：embed_query → sqlite-vec KNN → 融合排序
    let dense = dense_for(&db, &e, "作息");
    assert!(dense.contains_key("h"), "向量索引没召回：{dense:?}");
    let hits = retrieve_hybrid(&items, "作息", &r, Some(&dense), NOW, TOP_K);
    assert_eq!(hits.len(), 1, "稠密候选没能进结果");
    assert_eq!(hits[0].lexical, 0.0);
    assert!(hits[0].dense.unwrap() > 0.5, "稠密分太低：{:?}", hits[0].dense);
}

#[test]
fn 存进去的向量和现算的一致() {
    // 归一化的约定要两边对上：Embedder 出 L2 归一向量，
    // 向量表声明 distance_metric=cosine，所以 1 − distance 就该等于现算的余弦
    let Some((e, db)) = setup() else { return };
    write(&db, &e, "a", "我晚上工作", MemoryType::Habit);
    let v = e.embed_one("我晚上工作").unwrap();
    let hits = db.knn(&v, 5).unwrap();
    assert_eq!(hits[0].0, "a");
    assert!((hits[0].1 - cosine(&v, &v)).abs() < 1e-3, "索引余弦 {} 和现算对不上", hits[0].1);
}

#[test]
fn 忘记之后稠密也召不回来了() {
    let Some((e, db)) = setup() else { return };
    write(&db, &e, "h", "晚上上班白天学习", MemoryType::Habit);
    assert!(dense_for(&db, &e, "作息").contains_key("h"));

    db.set_memory_status("h", Status::Deleted, NOW).unwrap();
    db.delete_embedding("h").unwrap();

    assert!(dense_for(&db, &e, "作息").is_empty(), "向量索引里还留着");
    let items = db.all_memories().unwrap();
    assert_eq!(items.len(), 1, "行要留着当审计");
    assert!(retrieve_hybrid(&items, "作息", &BigramRetriever, None, NOW, TOP_K).is_empty());
}

#[test]
fn 更新内容会让旧向量作废() {
    // 这是最容易漏的一处：内容改了但向量没重算，检索就会按**上一版的意思**命中。
    // 用户纠正了信息，系统却还按旧的理解找东西，比没找到更糟
    let Some((e, db)) = setup() else { return };
    write(&db, &e, "h", "晚上上班白天学习", MemoryType::Habit);
    assert!(dense_for(&db, &e, "作息").contains_key("h"));

    let mut changed = db.all_memories().unwrap()[0].clone();
    changed.content = "喜欢喝咖啡".into();
    db.put_memory(&changed).unwrap();
    db.put_embedding("h", &e.embed_one(&changed.content).unwrap(), e.version()).unwrap();

    let d = dense_for(&db, &e, "作息");
    // 还在索引里（就一条），但相似度应该塌下来
    assert!(d.get("h").copied().unwrap_or(0.0) < 0.5, "旧向量没被换掉：{d:?}");
}

#[test]
fn 换模型会重建索引并清空版本号() {
    let Some((e, db)) = setup() else { return };
    write(&db, &e, "a", "我晚上工作", MemoryType::Habit);
    assert_eq!(db.memories_needing_embedding(e.version()).unwrap().len(), 0);

    // 换一个「模型」（维度也变了）
    assert!(db.ensure_vec_table(e.dim() + 1, "another-model").unwrap());
    assert!(db.knn(&vec![0.0; e.dim() + 1], 5).unwrap().is_empty());
    assert_eq!(
        db.memories_needing_embedding("another-model").unwrap().len(),
        1,
        "版本号没清空，后台就不会重算"
    );
}

#[test]
fn 批量补算和逐条算结果一致() {
    let Some((e, db)) = setup() else { return };
    let texts = ["我", "晚上工作白天学习考研", "喜欢喝咖啡不打扰"];
    for (i, t) in texts.iter().enumerate() {
        let m = draft(format!("m{i}"), (*t).into(), MemoryType::Profile, 70.0, 1.0,
                      Source::UserExplicit, None, "", NOW);
        db.put_memory(&m).unwrap();
    }
    // 走补算那条路：一次一批，padding 到批内最长
    let todo = db.memories_needing_embedding(e.version()).unwrap();
    assert_eq!(todo.len(), 3);
    let batch: Vec<&str> = todo.iter().map(|m| m.content.as_str()).collect();
    let vecs = e.embed_batch(&batch).unwrap();
    for (m, v) in todo.iter().zip(&vecs) {
        db.put_embedding(&m.id, v, e.version()).unwrap();
        // 逐条算应当得到同一个向量——否则 mask 加权池化写错了
        let single = e.embed_one(&m.content).unwrap();
        assert!(cosine(v, &single) > 0.9999, "「{}」批量≠单条", m.content);
    }
    assert!(db.memories_needing_embedding(e.version()).unwrap().is_empty());
}
