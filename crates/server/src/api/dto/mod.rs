//! Request and response DTOs (spec/01 §4: separate from entities, exported with ts-rs).

use serde::Deserialize;

/// `Option<Option<T>>` with PATCH wire semantics: an absent field leaves the column unchanged,
/// an explicit `null` clears it (spec/10 §2.4). Shared by every patch DTO with nullable fields.
pub(crate) fn deserialize_some<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

pub mod books;
pub mod canon;
pub mod changes;
pub mod chapters;
pub mod content_ops;
pub mod goals;
pub mod knowledge;
pub mod prompts;
pub mod proposals;
pub mod providers;
pub mod research;
pub mod reviews;
pub mod search;
pub mod sessions;
pub mod settings;
pub mod settings_materials;
pub mod skills;
pub mod tracks;

pub use books::{
    BookAgentStart, BookAgentStarted, BookDto, BookInput, BookOverviewDto, BookPatch,
    BookProgressDto, BookSummaryDto, ChapterPointerDto, CreateBookResponse, StatusCountDto,
    TrackProgressDto,
};
pub use chapters::{
    ApproveChapterResponse, ChapterContentPatch, ChapterContentResult, ChapterContentTextPut,
    ChapterDto, ChapterInput, ChapterListItemDto, ChapterPatch, ChapterReorderInput,
    ChapterVersionDto, ChapterVersionMetaDto, ChapterWriteInput, ChapterWriteResponse,
    ManualVersionInput,
};
pub use content_ops::{ContentOp, ContentOpInput};
pub use goals::{CanonCountsDto, GoalNodeDto, GoalNodeInput, GoalQuery, GoalTargetDto};
pub use prompts::{
    PromptEntryDto, PromptSaveInput, PromptVariableDocDto, PromptVariableFieldDocDto,
};
pub use proposals::{ProposalDto, ProposalPreviewDto, ProposalQuery};
pub use research::{
    KnowledgeCheckInput, KnowledgeCheckResponse, ResearchRunResponse, ResearchTaskCreate,
    ResearchTaskDto, ResearchTaskPatch, ResearchTaskQuery,
};
pub use reviews::{
    ChapterFeedbackDto, RereviewInput, ReviewReportDto, ReviewSessionResponse, ReviewsResponse,
    SendBackInput,
};
pub use search::{SearchHitDto, SearchTestInput, SearchTestResponse};
pub use sessions::{
    DebugEchoDto, DebugEchoInput, MessageBlockDto, MessageDto, MessagePageDto, MessageWithCallsDto,
    ResumeInput, RetryingDto, SessionAnswerInput, SessionDto, SessionMessageInput, SessionPatch,
    SessionSnapshotDto, ToolCallDto,
};
pub use settings::{
    AppSettingsDto, AppSettingsPatch, EmbeddingSettingsDto, FetchVia, SearchProvider,
    SearchSettingsDto, SearchSettingsPatch,
};
pub use settings_materials::{
    CharacterDto, CharacterInput, CharacterPatch, CharacterReorderInput, SettingDocDto,
    SettingDocInput,
};
pub use tracks::{TrackDto, TrackInput, TrackPatch};
