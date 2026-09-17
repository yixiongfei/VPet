//! 工具层：Brain 能对这个系统做的全部事情（docs/03 §5–6、roadmap 2.5）。
//!
//! 这一层存在的理由是**收窄**，不是方便。Phase 3 的 LLM 只能通过 `run_tool` 动这个
//! 系统：每次调用先过权限门，再执行，最后落一条审计。LLM 拿不到别的入口。
//!
//! 三个部件各管一件事：
//!   - `ToolDef`      工具长什么样。`executor` 只有 Rust 认识，不发给 LLM
//!   - `PermissionGate` 准不准做。身体机能免授权，其余查授权表
//!   - `AuditLog`     做过什么。**不提供删除接口**——审计能被抹掉就没有意义

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

/// 调用是用户直接要求的，还是她自己主动发起的。
/// 主动发起的权限门槛更高——docs/03 §6
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    User,
    Proactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permission {
    /// 如 `pet.state` / `fs.write` / `kb.read`
    pub scope: String,
    pub level: Level,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: String,
    /// 给 LLM 看的
    pub description: String,
    /// JSON Schema，给 LLM 生成参数用
    pub input_schema: Value,
    pub permission: Permission,
    /// 身体机能（计时器 / 番茄钟 / 看自己的状态）免授权——
    /// 她动自己的身体不需要经过谁同意
    #[serde(default)]
    pub body_mechanic: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default = "user_origin")]
    pub origin: Origin,
}

fn user_origin() -> Origin {
    Origin::User
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    /// denied / invalid_input / upstream / timeout / unknown_tool
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
}

impl ToolResult {
    pub fn ok(call_id: &str, data: Value) -> Self {
        Self {
            call_id: call_id.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(call_id: &str, code: &str, message: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            ok: false,
            data: None,
            error: Some(ToolError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

/* ------------------------------------------------------------------ */

/// 授权表。key 是 scope，越具体的越优先（`fs.write:D:\x` 盖过 `fs.write`）
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PermissionGate {
    grants: HashMap<String, Decision>,
}

impl PermissionGate {
    pub fn set(&mut self, scope: &str, decision: Decision) {
        self.grants.insert(scope.to_string(), decision);
    }

    pub fn grants(&self) -> &HashMap<String, Decision> {
        &self.grants
    }

    /// docs/03 §6 的判定顺序：
    /// 身体机能直接放行 → 查最具体的授权 → 默认表（读/写 = 问，执行 = 拒）
    /// → 主动发起的把 Allow 降成 Ask（只读除外）
    pub fn check(&self, tool: &ToolDef, origin: Origin) -> Decision {
        if tool.body_mechanic {
            return Decision::Allow;
        }
        let explicit = self.most_specific(&tool.permission.scope);
        let base = explicit.unwrap_or(match tool.permission.level {
            Level::Read | Level::Write => Decision::Ask,
            Level::Execute => Decision::Deny,
        });
        // 她自己想干的事，门槛高一档
        if origin == Origin::Proactive
            && base == Decision::Allow
            && tool.permission.level != Level::Read
        {
            return Decision::Ask;
        }
        base
    }

    /// `fs.write:D:\x\y` 命中 `fs.write:D:\x\y` > `fs.write` 的顺序
    fn most_specific(&self, scope: &str) -> Option<Decision> {
        let mut best: Option<(usize, Decision)> = None;
        for (k, v) in &self.grants {
            let hit = scope == k || scope.starts_with(&format!("{k}:")) || scope.starts_with(&format!("{k}."));
            if hit && best.as_ref().is_none_or(|(len, _)| k.len() > *len) {
                best = Some((k.len(), *v));
            }
        }
        best.map(|(_, d)| d)
    }
}

/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub at: i64,
    pub tool: String,
    pub origin: Origin,
    pub decision: Decision,
    pub ok: bool,
    /// 输入摘要，截断过——审计是给人看的，不是给机器回放的
    pub input: String,
    pub summary: String,
}

/// 只进不出。**没有删除接口**，这是故意的：审计能被抹掉就没有意义
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

/// 内存里留多少条。再多的去翻 `pet_state_log` 那样的流水表
const MAX_ENTRIES: usize = 500;
/// 输入摘要截断长度
const INPUT_CAP: usize = 200;

impl AuditLog {
    pub fn record(&mut self, e: AuditEntry) {
        self.entries.push(e);
        if self.entries.len() > MAX_ENTRIES {
            let cut = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..cut);
        }
    }

    /// 最近的在前
    pub fn recent(&self, n: usize) -> Vec<AuditEntry> {
        self.entries.iter().rev().take(n).cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn summarize_input(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() <= INPUT_CAP {
        return s;
    }
    let cut: String = s.chars().take(INPUT_CAP).collect();
    format!("{cut}…")
}

/* ------------------------------------------------------------------ */

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

/// 内建工具表（roadmap 2.5）。真正的执行在 lib.rs——那里才够得着 app state
pub fn builtin_tools() -> Vec<ToolDef> {
    let body = |name: &str, desc: &str, schema: Value, level: Level| ToolDef {
        name: name.into(),
        description: desc.into(),
        input_schema: schema,
        permission: Permission {
            scope: format!("pet.{name}"),
            level,
        },
        body_mechanic: true,
    };
    vec![
        body(
            "create_timer",
            "排一个计时器，到点提醒。duration 写成 10s / 25m / 1h",
            obj(
                json!({
                    "duration": { "type": "string", "description": "多久之后，如 25m" },
                    "label": { "type": "string", "description": "到点说什么" },
                    "repeat": { "type": "boolean", "description": "是否循环" }
                }),
                &["duration"],
            ),
            Level::Write,
        ),
        body(
            "cancel_timer",
            "取消一个计时器",
            obj(json!({ "id": { "type": "string" } }), &["id"]),
            Level::Write,
        ),
        body(
            "list_timers",
            "看看现在排了哪些计时器",
            obj(json!({}), &[]),
            Level::Read,
        ),
        body(
            "start_pomodoro",
            "开始一个番茄钟，专注 25 分钟",
            obj(json!({}), &[]),
            Level::Write,
        ),
        body(
            "stop_pomodoro",
            "停掉番茄钟",
            obj(json!({}), &[]),
            Level::Write,
        ),
        body(
            "request_action",
            "请她去做某件事：target 写动作 id（work_copy）或大类 tag（work / study / play / rest / eat / drink）。\
             她不一定答应——要看此刻的心情、身体状态和对你的好感度",
            obj(
                json!({
                    "target": { "type": "string", "description": "动作 id 或 tag，如 work / rest" }
                }),
                &["target"],
            ),
            Level::Write,
        ),
        body(
            "get_pet_state",
            "看看她现在的状态：在做什么、体力心情饱腹口渴、钱和等级",
            obj(json!({}), &[]),
            Level::Read,
        ),
        ToolDef {
            name: "set_permission".into(),
            description: "改某个权限范围的授权：allow / ask / deny".into(),
            input_schema: obj(
                json!({
                    "scope": { "type": "string" },
                    "decision": { "type": "string", "enum": ["allow", "ask", "deny"] }
                }),
                &["scope", "decision"],
            ),
            permission: Permission {
                scope: "permission.write".into(),
                level: Level::Write,
            },
            // 改权限本身必须经过用户点头，不然 LLM 可以自己给自己发通行证
            body_mechanic: false,
        },
    ]
}

pub fn find<'a>(tools: &'a [ToolDef], name: &str) -> Option<&'a ToolDef> {
    tools.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(scope: &str, level: Level, body: bool) -> ToolDef {
        ToolDef {
            name: "t".into(),
            description: String::new(),
            input_schema: json!({}),
            permission: Permission {
                scope: scope.into(),
                level,
            },
            body_mechanic: body,
        }
    }

    #[test]
    fn 身体机能免授权() {
        let g = PermissionGate::default();
        let t = tool("pet.create_timer", Level::Write, true);
        assert_eq!(g.check(&t, Origin::User), Decision::Allow);
        assert_eq!(
            g.check(&t, Origin::Proactive),
            Decision::Allow,
            "她自己想定个闹钟也不用问谁"
        );
    }

    #[test]
    fn 没授权时读写要问执行直接拒() {
        let g = PermissionGate::default();
        assert_eq!(g.check(&tool("fs.read", Level::Read, false), Origin::User), Decision::Ask);
        assert_eq!(g.check(&tool("fs.write", Level::Write, false), Origin::User), Decision::Ask);
        assert_eq!(
            g.check(&tool("shell.run", Level::Execute, false), Origin::User),
            Decision::Deny,
            "没明说过的执行类一律拒"
        );
    }

    #[test]
    fn 越具体的授权越优先() {
        let mut g = PermissionGate::default();
        g.set("fs.write", Decision::Deny);
        g.set("fs.write:D:\\obsidian", Decision::Allow);
        assert_eq!(
            g.check(&tool("fs.write:D:\\obsidian", Level::Write, false), Origin::User),
            Decision::Allow
        );
        assert_eq!(
            g.check(&tool("fs.write:C:\\Windows", Level::Write, false), Origin::User),
            Decision::Deny,
            "别的路径还是按上级的拒"
        );
    }

    #[test]
    fn 她自己发起的写操作要再问一次() {
        let mut g = PermissionGate::default();
        g.set("kb.write", Decision::Allow);
        let t = tool("kb.write", Level::Write, false);
        assert_eq!(g.check(&t, Origin::User), Decision::Allow);
        assert_eq!(
            g.check(&t, Origin::Proactive),
            Decision::Ask,
            "用户说过可以写，不等于她半夜自己想写也可以"
        );
    }

    #[test]
    fn 她自己发起的只读不用再问() {
        let mut g = PermissionGate::default();
        g.set("kb.read", Decision::Allow);
        let t = tool("kb.read", Level::Read, false);
        assert_eq!(g.check(&t, Origin::Proactive), Decision::Allow);
    }

    #[test]
    fn 改权限这件事本身必须经过用户() {
        let t = find(&builtin_tools(), "set_permission").unwrap().clone();
        assert!(
            !t.body_mechanic,
            "改权限要是免授权，LLM 就能自己给自己发通行证"
        );
        assert_eq!(PermissionGate::default().check(&t, Origin::User), Decision::Ask);
    }

    #[test]
    fn 内建工具都有名字和描述() {
        for t in builtin_tools() {
            assert!(!t.name.is_empty());
            assert!(!t.description.is_empty(), "{} 没写描述，LLM 不知道怎么用", t.name);
            assert!(t.input_schema.get("type").is_some(), "{} 的 schema 不完整", t.name);
        }
    }

    #[test]
    fn 工具定义发出去时不带执行细节() {
        let json = serde_json::to_string(&builtin_tools()).unwrap();
        assert!(!json.contains("executor"), "executor 不该出现在给 LLM 的定义里");
    }

    #[test]
    fn 审计只进不出() {
        let mut a = AuditLog::default();
        for i in 0..3 {
            a.record(AuditEntry {
                at: i,
                tool: "create_timer".into(),
                origin: Origin::User,
                decision: Decision::Allow,
                ok: true,
                input: "{}".into(),
                summary: "排了一个".into(),
            });
        }
        assert_eq!(a.len(), 3);
        // 类型上就没有删除接口——这条测试是写给未来的人看的
        assert_eq!(a.recent(2).len(), 2);
        assert_eq!(a.recent(2)[0].at, 2, "最近的在前");
    }

    #[test]
    fn 审计不会无限涨() {
        let mut a = AuditLog::default();
        for i in 0..(MAX_ENTRIES + 100) {
            a.record(AuditEntry {
                at: i as i64,
                tool: "x".into(),
                origin: Origin::User,
                decision: Decision::Allow,
                ok: true,
                input: String::new(),
                summary: String::new(),
            });
        }
        assert_eq!(a.len(), MAX_ENTRIES);
        assert_eq!(a.recent(1)[0].at, (MAX_ENTRIES + 99) as i64, "留下的该是最近的");
    }

    #[test]
    fn 超长输入会被截断() {
        let long = json!({ "text": "啊".repeat(1000) });
        let s = summarize_input(&long);
        assert!(s.chars().count() <= INPUT_CAP + 1, "实际 {} 字", s.chars().count());
        assert!(s.ends_with('…'));
    }
}
