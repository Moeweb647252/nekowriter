//! SeaORM entity modules, one file per table (spec/02 §2).

pub mod agent_inbox;
pub mod agent_messages;
pub mod agent_sessions;
pub mod agent_tool_calls;
pub mod app_settings;
pub mod appearances;
pub mod book_scopes;
pub mod books;
pub mod change_log;
pub mod chapter_feedback;
pub mod chapter_versions;
pub mod chapters;
pub mod character_designs;
#[allow(clippy::module_inception)]
pub mod entities;

pub mod foreshadowing;
pub mod foreshadowing_entities;
pub mod index_meta;
pub mod knowledge_chunks;
pub mod knowledge_entries;
pub mod knowledge_scopes;
pub mod memory_note_entities;
pub mod memory_notes;
pub mod model_profiles;
pub mod prompt_overrides;
pub mod providers;
pub mod relations;
pub mod research_tasks;
pub mod review_reports;
pub mod search_docs;
pub mod setting_docs;
pub mod setting_proposals;
pub mod timeline_event_entities;
pub mod timeline_events;
pub mod tracks;
