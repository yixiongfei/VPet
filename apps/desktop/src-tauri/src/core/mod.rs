//! Core：确定性核心。状态机 / 调度 / 工具 / 权限 / 存储都在这里展开
//! （docs/03 §3）。LLM 只做理解和表达，计时和数值全是这里的确定性代码。

pub mod actions;
pub mod db;
pub mod food;
pub mod pomodoro;
pub mod scheduler;
pub mod state_machine;
