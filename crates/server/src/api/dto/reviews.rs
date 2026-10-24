//! Review DTOs (spec/08 §1.1–§1.2, §3, spec/10 §2.2).
//!
//! A report records the version it reviewed (`content_version`), so a report for an earlier
//! version is shown as such by the UI (spec/08 §1.2). `issues` is structured data: ticked issues
//! travel to the writer as `chapter_feedback.issues`, never as pasted text (spec/08 §3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{FeedbackMode, ReviewIssue, ReviewVerdict};

/// One review report (`GET /chapters/:id/reviews`, spec/10 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewReportDto {
    pub id: i32,
    pub chapter_id: i32,
    /// The chapter version this report reviewed (spec/08 §1.2).
    pub content_version: i32,
    pub session_id: Option<i32>,
    /// 1, 2, … per chapter.
    pub attempt: i32,
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub issues: Vec<ReviewIssue>,
    pub feedback: String,
    /// The author's comment that triggered this report (re-review), empty otherwise.
    pub user_comment: String,
    pub created_at: DateTime<Utc>,
}

/// One send-back (`chapter_feedback`), spec/08 §1.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChapterFeedbackDto {
    pub id: i32,
    pub chapter_id: i32,
    pub content_version: i32,
    pub review_report_id: Option<i32>,
    pub text: String,
    /// The ticked issues, copied from the report at send-back time (spec/08 §3).
    pub issues: Vec<ReviewIssue>,
    pub mode: FeedbackMode,
    /// The revision session this send-back started, when there was one.
    pub session_id: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// `GET /chapters/:id/reviews` (spec/10 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewsResponse {
    /// Newest first: report #2 before report #1 (spec/08 §3).
    pub reports: Vec<ReviewReportDto>,
    /// Newest first.
    pub feedback: Vec<ChapterFeedbackDto>,
}

/// `{ session_id }` responses of the review/maintenance endpoints (spec/10 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReviewSessionResponse {
    pub session_id: i32,
}

/// `POST /review-reports/:id/rereview` body (spec/03 §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RereviewInput {
    pub comment: String,
}

/// `POST /chapters/:id/send-back` body (spec/08 §1.1, spec/10 §2.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(optional_fields = nullable)]
#[ts(export)]
pub struct SendBackInput {
    pub feedback: String,
    pub mode: FeedbackMode,
    /// The report the feedback came from; its ticked issues are copied (spec/08 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_report_id: Option<i32>,
    /// Zero-based indexes into that report's `issues`.
    #[serde(default)]
    pub issue_indexes: Vec<usize>,
}

impl From<crate::db::entities::review_reports::Model> for ReviewReportDto {
    fn from(row: crate::db::entities::review_reports::Model) -> Self {
        Self {
            id: row.id,
            chapter_id: row.chapter_id,
            content_version: row.content_version,
            session_id: row.session_id,
            attempt: row.attempt,
            verdict: ReviewVerdict::from_str_opt(&row.verdict).unwrap_or(ReviewVerdict::Revise),
            summary: row.summary,
            issues: crate::db::json::from_json_or_default(&row.issues),
            feedback: row.feedback,
            user_comment: row.user_comment,
            created_at: row.created_at,
        }
    }
}

impl From<crate::db::entities::chapter_feedback::Model> for ChapterFeedbackDto {
    fn from(row: crate::db::entities::chapter_feedback::Model) -> Self {
        Self {
            id: row.id,
            chapter_id: row.chapter_id,
            content_version: row.content_version,
            review_report_id: row.review_report_id,
            text: row.text,
            issues: crate::db::json::from_json_or_default(&row.issues),
            mode: FeedbackMode::from_str_opt(&row.mode).unwrap_or(FeedbackMode::Revise),
            session_id: row.session_id,
            created_at: row.created_at,
        }
    }
}
