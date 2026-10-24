//! Request and response DTOs (spec/01 §4: separate from entities, exported with ts-rs).

pub mod books;
pub mod canon;
pub mod changes;
pub mod chapters;
pub mod content_ops;
pub mod prompts;
pub mod providers;
pub mod reviews;
pub mod sessions;
pub mod settings;
pub mod settings_materials;
pub mod tracks;

pub use books::{
    BookDto, BookInput, BookOverviewDto, BookPatch, BookProgressDto, BookSummaryDto,
    ChapterPointerDto, CreateBookResponse, StatusCountDto, TrackProgressDto,
};
pub use chapters::{
    ApproveChapterResponse, ChapterContentPatch, ChapterContentResult, ChapterContentTextPut,
    ChapterDto, ChapterInput, ChapterListItemDto, ChapterPatch, ChapterReorderInput,
    ChapterVersionDto, ChapterVersionMetaDto, ChapterWriteInput, ChapterWriteResponse,
    ManualVersionInput,
};
pub use content_ops::{ContentOp, ContentOpInput};
pub use prompts::{
    PromptEntryDto, PromptSaveInput, PromptVariableDocDto, PromptVariableFieldDocDto,
};
pub use reviews::{
    ChapterFeedbackDto, RereviewInput, ReviewReportDto, ReviewSessionResponse, ReviewsResponse,
    SendBackInput,
};
pub use sessions::{
    DebugEchoDto, DebugEchoInput, MessageBlockDto, MessageDto, MessagePageDto, MessageWithCallsDto,
    ResumeInput, RetryingDto, SessionAnswerInput, SessionDto, SessionMessageInput, SessionPatch,
    SessionSnapshotDto, ToolCallDto,
};
pub use settings::{
    AppSettingsDto, AppSettingsPatch, EmbeddingSettingsDto, SearchProvider, SearchSettingsDto,
    SearchSettingsPatch,
};
pub use settings_materials::{
    CharacterDto, CharacterInput, CharacterPatch, CharacterReorderInput, SettingDocDto,
    SettingDocInput,
};
pub use tracks::{TrackDto, TrackInput, TrackPatch};
