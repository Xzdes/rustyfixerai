// src/modules/mod.rs

// Эта строка делает модуль `llm_interface` публичным внутри крейта,
// чтобы `main.rs` мог его найти и использовать через `use modules::llm_interface::...`
pub mod llm_interface;
pub mod web_agent;
pub mod patch_engine;