//! 语义向量（docs/07 roadmap 2.9 / Phase 4.1）。
//!
//! 字面检索（`memory::BigramRetriever`）的天花板是**没有共同字的同义表达**：
//! 「作息」和「晚上上班白天学习」说的是一回事，但两个向量正交，余弦恒为 0。
//! 这不是精度问题，是结构问题——计数向量的每个符号占一根独立的轴。
//!
//! embedding 换掉的正是这个结构。两种方法的相似度都能写成同一个二次型
//! `xᵀ M y`，区别只在 M：字面法是 `M = I`（符号两两正交），embedding 是
//! `M = WᵀW`（W 是学出来的低秩投影，非对角元素就是符号之间的语义相关性）。
//!
//! 工程上的四条取舍：
//!
//! 1. **不用 `fastembed`，直接用 `ort` + `tokenizers`。** fastembed 会把
//!    hf-hub、TLS 栈、图像模型一起拖进来，而且强依赖 HuggingFace 可达。
//!    直接用底座少一半依赖，而且模型文件放在本地目录里，完全离线。
//! 2. **模型是可选的。** 目录不存在、文件损坏、ORT 装不上——任何一种情况都
//!    退回字面检索，不影响宠物跑。对一个桌宠来说，「没有语义检索」是降级，
//!    「起不来」是故障。
//! 3. **加载放后台线程。** 几十 MB 的模型冷启动要几百毫秒到几秒，不能挡着
//!    宠物出现在桌面上。
//! 4. **池化方式由模型目录里的 `embed.json` 决定**，不写死。bge 系列要 CLS
//!    池化 + 检索指令前缀，m3e / text2vec 要 mean 池化——同一段代码得都能跑。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 模型目录里的 `embed.json`。没有这个文件就按默认值走
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedConfig {
    /// 写进 `memory_items.embedding_version`。换模型时按它筛出要重算的条目
    pub name: String,
    /// 向量维度。必须和模型实际输出一致，否则向量表建错宽度
    pub dim: usize,
    #[serde(default = "default_max_len")]
    pub max_len: usize,
    #[serde(default)]
    pub pooling: Pooling,
    /// bge 系列检索时要给**查询**加的指令前缀（文档侧不加）。
    /// 这不是可有可无的调味——bge 就是这么训练的，不加会掉点
    #[serde(default)]
    pub query_prefix: String,
}

fn default_max_len() -> usize {
    512
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pooling {
    /// 按 attention_mask 加权平均。m3e / text2vec / 多数 sentence-transformers
    #[default]
    Mean,
    /// 取第 0 个 token。bge 系列官方就是这么用的
    Cls,
}

/// 向后端报告的状态。面板要能告诉用户「现在到底在用哪种检索」——
/// 悄悄降级是最坏的一种降级
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum EmbedState {
    /// 没配模型目录，纯字面检索
    Disabled { reason: String },
    Loading,
    Ready { name: String, dim: usize },
    /// 试过了，起不来。带上原因，别让用户猜
    Failed { reason: String },
}

/// 模型目录：优先 `VPET_EMBED_MODEL`，否则 app 数据目录下的 `embed-model/`
pub fn model_dir(app_data: &Path) -> PathBuf {
    std::env::var_os("VPET_EMBED_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data.join("embed-model"))
}

/// 这个目录看着像不像一个可用的模型
pub fn looks_ready(dir: &Path) -> bool {
    dir.join("model.onnx").is_file() && dir.join("tokenizer.json").is_file()
}

/* ==================== 以下要 ONNX 才编得出来 ==================== */

#[cfg(feature = "_onnx")]
mod onnx {
    use super::*;
    use ort::session::Session;
    use ort::value::Value;
    use tokenizers::Tokenizer;

    pub struct Embedder {
        /// `Session::run` 要 `&mut self`，而 Embedder 是跨线程共享的（Arc）。
        /// 用 Mutex 把推理串起来——批量很小（一次几条），串行不是瓶颈，
        /// 而且 ONNX session 本来就不该并发复制
        session: std::sync::Mutex<Session>,
        tokenizer: Tokenizer,
        cfg: EmbedConfig,
        /// 这个模型声明了哪些输入。BERT 家族要 token_type_ids，别的不一定，
        /// 多喂一个没声明的输入 ORT 会直接报错
        wants_token_type: bool,
        output_name: String,
    }

    impl Embedder {
        pub fn load(dir: &Path) -> Result<Self, String> {
            let model = dir.join("model.onnx");
            let tok = dir.join("tokenizer.json");
            if !model.is_file() || !tok.is_file() {
                return Err(format!("{} 里没有 model.onnx / tokenizer.json", dir.display()));
            }
            let cfg: EmbedConfig = match std::fs::read_to_string(dir.join("embed.json")) {
                Ok(j) => serde_json::from_str(&j).map_err(|e| format!("embed.json 读不懂：{e}"))?,
                // 没有配置文件时给一套保守的默认值，维度稍后从模型实际输出里纠正
                Err(_) => EmbedConfig {
                    name: dir
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "unknown".into()),
                    dim: 0,
                    max_len: default_max_len(),
                    pooling: Pooling::default(),
                    query_prefix: String::new(),
                },
            };

            let mut tokenizer =
                Tokenizer::from_file(&tok).map_err(|e| format!("分词器加载失败：{e}"))?;
            // 定长 padding + 截断：一次喂一批，形状必须齐
            tokenizer
                .with_truncation(Some(tokenizers::TruncationParams {
                    max_length: cfg.max_len,
                    ..Default::default()
                }))
                .map_err(|e| format!("截断参数不对：{e}"))?;
            tokenizer.with_padding(Some(tokenizers::PaddingParams {
                strategy: tokenizers::PaddingStrategy::BatchLongest,
                ..Default::default()
            }));

            let session = Session::builder()
                .map_err(|e| format!("ONNX Runtime 起不来：{e}"))?
                .commit_from_file(&model)
                .map_err(|e| format!("模型加载失败：{e}"))?;

            let wants_token_type = session.inputs().iter().any(|i| i.name() == "token_type_ids");
            let output_name = session
                .outputs()
                .iter()
                .find(|o| o.name() == "last_hidden_state" || o.name() == "sentence_embedding")
                .or_else(|| session.outputs().first())
                .map(|o| o.name().to_string())
                .ok_or_else(|| "模型没有输出".to_string())?;

            let mut me = Self {
                session: std::sync::Mutex::new(session),
                tokenizer,
                cfg,
                wants_token_type,
                output_name,
            };
            // 用一句话实跑一次，把真实维度问出来——写在 embed.json 里的数字可能是错的，
            // 而向量表的宽度一旦建错，后面每次写入都会失败
            let probe = me.embed_batch(&["维度探测"])?;
            let dim = probe.first().map(|v| v.len()).unwrap_or(0);
            if dim == 0 {
                return Err("模型输出维度是 0".into());
            }
            if me.cfg.dim != 0 && me.cfg.dim != dim {
                log::warn!("embed.json 写的维度是 {}，模型实际是 {dim}，以模型为准", me.cfg.dim);
            }
            me.cfg.dim = dim;
            Ok(me)
        }

        pub fn dim(&self) -> usize {
            self.cfg.dim
        }

        pub fn version(&self) -> &str {
            &self.cfg.name
        }

        /// 查询侧要加指令前缀，文档侧不加。这是 bge 的训练方式决定的，不是风格问题
        pub fn embed_query(&self, q: &str) -> Result<Vec<f32>, String> {
            let text = if self.cfg.query_prefix.is_empty() {
                q.to_string()
            } else {
                format!("{}{}", self.cfg.query_prefix, q)
            };
            Ok(self.embed_batch(&[&text])?.into_iter().next().unwrap_or_default())
        }

        pub fn embed_one(&self, text: &str) -> Result<Vec<f32>, String> {
            Ok(self.embed_batch(&[text])?.into_iter().next().unwrap_or_default())
        }

        pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let encodings = self
                .tokenizer
                .encode_batch(texts.to_vec(), true)
                .map_err(|e| format!("分词失败：{e}"))?;
            let batch = encodings.len();
            let len = encodings.first().map(|e| e.get_ids().len()).unwrap_or(0);
            if len == 0 {
                return Ok(vec![Vec::new(); batch]);
            }

            let mut ids = Vec::with_capacity(batch * len);
            let mut mask = Vec::with_capacity(batch * len);
            for e in &encodings {
                ids.extend(e.get_ids().iter().map(|&v| v as i64));
                mask.extend(e.get_attention_mask().iter().map(|&v| v as i64));
            }
            let shape = [batch as i64, len as i64];

            let mut inputs: Vec<(&str, Value)> = vec![
                (
                    "input_ids",
                    Value::from_array((shape, ids.clone()))
                        .map_err(|e| format!("input_ids 装不进去：{e}"))?
                        .into_dyn(),
                ),
                (
                    "attention_mask",
                    Value::from_array((shape, mask.clone()))
                        .map_err(|e| format!("attention_mask 装不进去：{e}"))?
                        .into_dyn(),
                ),
            ];
            if self.wants_token_type {
                inputs.push((
                    "token_type_ids",
                    Value::from_array((shape, vec![0i64; batch * len]))
                        .map_err(|e| format!("token_type_ids 装不进去：{e}"))?
                        .into_dyn(),
                ));
            }

            let mut session = self
                .session
                .lock()
                .map_err(|_| "推理会话被污染了".to_string())?;
            let outputs = session.run(inputs).map_err(|e| format!("推理失败：{e}"))?;
            let out = outputs
                .get(self.output_name.as_str())
                .ok_or_else(|| format!("输出里没有 {}", self.output_name))?;
            let (oshape, data) = out
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("输出取不出来：{e}"))?;

            // rank 3 = [B, L, D]，要自己池化；rank 2 = [B, D]，模型已经池化过了
            let vectors = match oshape.len() {
                3 => {
                    let d = oshape[2] as usize;
                    self.pool(data, &mask, batch, len, d)
                }
                2 => {
                    let d = oshape[1] as usize;
                    (0..batch).map(|b| data[b * d..(b + 1) * d].to_vec()).collect()
                }
                n => return Err(format!("看不懂的输出形状：{n} 维")),
            };
            Ok(vectors.into_iter().map(normalize).collect())
        }

        /// 池化。mean 要按 mask 加权——padding 位置的向量是噪音，
        /// 算进平均里会让长短不一的句子系统性地偏移
        fn pool(&self, data: &[f32], mask: &[i64], batch: usize, len: usize, d: usize) -> Vec<Vec<f32>> {
            (0..batch)
                .map(|b| match self.cfg.pooling {
                    Pooling::Cls => data[b * len * d..b * len * d + d].to_vec(),
                    Pooling::Mean => {
                        let mut acc = vec![0.0f32; d];
                        let mut n = 0.0f32;
                        for t in 0..len {
                            if mask[b * len + t] == 0 {
                                continue;
                            }
                            n += 1.0;
                            let off = (b * len + t) * d;
                            for (i, a) in acc.iter_mut().enumerate() {
                                *a += data[off + i];
                            }
                        }
                        if n > 0.0 {
                            for a in acc.iter_mut() {
                                *a /= n;
                            }
                        }
                        acc
                    }
                })
                .collect()
        }
    }

    /// L2 归一化。归一化之后余弦就等于内积，向量索引那边也能直接用
    /// `distance_metric=cosine`，两边的度量才是同一个
    fn normalize(mut v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 1e-12 {
            for x in v.iter_mut() {
                *x /= n;
            }
        }
        v
    }
}

#[cfg(feature = "_onnx")]
pub use onnx::Embedder;

/// 余弦。两边都归一化过的话就是内积，但这里不假设，老老实实算
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < 1e-12 || nb < 1e-12 {
        0.0
    } else {
        (dot / (na * nb)).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 余弦算得对() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0, "维度不同应当直接判 0");
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0, "零向量不该算出 NaN");
    }

    #[test]
    fn 缺文件的目录不算就绪() {
        assert!(!looks_ready(Path::new("/nonexistent-vpet-model")));
    }

    #[test]
    fn 环境变量能覆盖模型目录() {
        // 不改全局环境，只验证默认路径的拼法
        let d = model_dir(Path::new("/tmp/appdata"));
        assert!(d.ends_with("embed-model") || std::env::var_os("VPET_EMBED_MODEL").is_some());
    }
}

/// 真跑一次推理的测试。需要两样东西，任一缺失就跳过（不算失败）：
///   ORT_DYLIB_PATH      指向 libonnxruntime.so
///   VPET_TEST_MODEL     指向含 model.onnx / tokenizer.json 的目录
///
/// 云端没有 HuggingFace，所以 CI 里用的是一个**合成模型**（Gather 一张随机
/// 词向量表）。它当然没有语义，但链路上要验的东西一样也不少：会话能不能起、
/// 输入形状对不对、mask 加权池化算得对不对、归一化之后模是不是 1。
#[cfg(all(test, feature = "_onnx"))]
mod live {
    use super::*;

    fn embedder() -> Option<Embedder> {
        let dir = std::env::var_os("VPET_TEST_MODEL")?;
        std::env::var_os("ORT_DYLIB_PATH")?;
        match Embedder::load(Path::new(&dir)) {
            Ok(e) => Some(e),
            Err(e) => panic!("模型目录给了却加载失败：{e}"),
        }
    }

    #[test]
    fn 能推理且向量是归一化的() {
        let Some(e) = embedder() else { return };
        let v = e.embed_one("我晚上工作白天学习").unwrap();
        assert_eq!(v.len(), e.dim(), "维度和自报的对不上");
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-4, "没归一化，模是 {n}");
    }

    #[test]
    fn 同一句话两次结果一致() {
        let Some(e) = embedder() else { return };
        let a = e.embed_one("我晚上工作").unwrap();
        let b = e.embed_one("我晚上工作").unwrap();
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-5, "推理不确定");
    }

    #[test]
    fn 批量和单条算出来一样() {
        let Some(e) = embedder() else { return };
        // 批量会 padding 到最长的那条。mask 加权池化做对了，结果才和单条一致——
        // 这条测试专门抓「把 padding 算进平均」这个经典 bug
        let texts = ["我", "我晚上工作白天学习考研"];
        let batch = e.embed_batch(&texts).unwrap();
        for (i, t) in texts.iter().enumerate() {
            let single = e.embed_one(t).unwrap();
            assert!(
                cosine(&batch[i], &single) > 0.9999,
                "「{t}」批量和单条不一致：cos = {}",
                cosine(&batch[i], &single)
            );
        }
    }

    #[test]
    fn 不同句子向量不同() {
        let Some(e) = embedder() else { return };
        let a = e.embed_one("我晚上工作").unwrap();
        let b = e.embed_one("考研").unwrap();
        assert!(cosine(&a, &b) < 0.999, "什么句子都映射到同一个点");
    }
}

#[cfg(all(test, feature = "_onnx"))]
mod cfg_probe {
    /// 守住一件容易静默失效的事：`dep:ort` 会**抑制**同名隐式 feature，
    /// 所以 `#[cfg(feature = \"ort\")]` 永远是假的——onnx 模块会被整块编掉，
    /// 而且不报任何错。这条测试保证「开了 embed 就一定有 Embedder」
    #[test]
    fn 开了_embed_就该有_embedder() {
        let _ = super::Embedder::load(std::path::Path::new("/nonexistent"));
    }
}
