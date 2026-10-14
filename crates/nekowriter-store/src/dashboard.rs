//! The dashboard (`docs/product-spec.md` §15): token usage, running jobs and word-count
//! growth, computed from the records of live projects and versions on every read.

use std::collections::{HashMap, HashSet};

use nekowriter_core::{
    chapter::ChapterLabel,
    dashboard::TimeBuckets,
    document::DocumentRole,
    ids::{ConnectionTestId, ProjectId, RunId, VersionId},
    review::ReviewStatus,
    run::{RunHalt, RunKind, RunStatus},
    time::Timestamp,
    usage::{Answer, TokenUsage, UsageStatus, UsageTotals},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
};
use serde::Serialize;
use ts_rs::TS;

use crate::{
    Store, StoreError,
    entity::{
        chapter, connection_test, document,
        model_request::{self, ModelRequestStatus},
        project, review, revision, run, version, workflow,
    },
};

/// What the dashboard shows for a time axis and, optionally, one project.
#[derive(Clone, Debug)]
pub struct DashboardQuery {
    pub buckets: TimeBuckets,
    pub project: Option<ProjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DashboardView {
    pub usage: UsageView,
    pub words: WordsView,
    /// Runs that are working or waiting for the user.
    pub jobs: Vec<JobView>,
    /// Runs that stopped, failed or were interrupted and wait to be continued.
    pub attention: Vec<JobView>,
}

/// Token usage of the requests sent in the time range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsageView {
    /// Requests of writing runs: preparation, outlines and bodies.
    pub writing: UsageTotals,
    /// Requests of review agents.
    pub review: UsageTotals,
    /// Connection tests, which belong to no project.
    pub connection_tests: UsageTotals,
    /// Writing and review requests in each time bucket.
    pub buckets: Vec<UsageTotals>,
    /// Projects that sent requests in the time range.
    pub projects: Vec<ProjectUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectUsage {
    pub project_id: ProjectId,
    pub name: String,
    pub writing: UsageTotals,
    pub review: UsageTotals,
}

/// Word-count growth of chapter bodies in the time range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WordsView {
    /// Net growth in the whole range; negative when the bodies shrank.
    pub net: i64,
    /// Net growth in each time bucket.
    pub buckets: Vec<i64>,
    /// Projects whose bodies changed in the time range.
    pub projects: Vec<ProjectWords>,
    /// The words of all current bodies of live versions, now.
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectWords {
    pub project_id: ProjectId,
    pub name: String,
    pub net: i64,
}

/// A version's current run, with where it belongs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobView {
    pub run_id: RunId,
    pub kind: RunKind,
    pub status: RunStatus,
    pub halt: Option<RunHalt>,
    pub project_id: ProjectId,
    pub project_name: String,
    pub version_id: VersionId,
    pub version_number: i32,
    /// The chapter a chapter run writes.
    pub chapter: Option<ChapterLabel>,
    /// The status of the review agent deciding the run's submission, while its review is
    /// delegated.
    pub review_agent: Option<RunStatus>,
    pub updated_at: Timestamp,
}

impl Store {
    pub async fn dashboard(&self, query: &DashboardQuery) -> Result<DashboardView, StoreError> {
        let txn = self.db.begin().await?;
        let live = Live::load(&txn, query.project).await?;
        let (jobs, attention) = jobs(&txn, &live).await?;
        let view = DashboardView {
            usage: usage(&txn, &live, &query.buckets).await?,
            words: words(&txn, &live, &query.buckets).await?,
            jobs,
            attention,
        };
        txn.commit().await?;
        Ok(view)
    }

    /// Records a connection test, so its tokens count apart from runs (§15).
    pub async fn record_connection_test(
        &self,
        answered: bool,
        usage: Option<TokenUsage>,
    ) -> Result<(), StoreError> {
        let tx = self.begin_internal().await?;
        connection_test::ActiveModel {
            id: Set(ConnectionTestId::generate()),
            answered: Set(answered),
            usage: Set(usage),
            tested_at: Set(tx.now()),
        }
        .insert(tx.db())
        .await?;
        tx.commit().await
    }
}

/// The projects and versions the dashboard counts: live ones, of the selected project.
struct Live {
    projects: HashMap<ProjectId, String>,
    versions: HashMap<VersionId, version::Model>,
}

impl Live {
    async fn load<C: ConnectionTrait>(
        db: &C,
        selected: Option<ProjectId>,
    ) -> Result<Self, StoreError> {
        let mut projects = project::Entity::find().filter(project::Column::DeletedAt.is_null());
        if let Some(id) = selected {
            projects = projects.filter(project::Column::Id.eq(id));
        }
        let projects: HashMap<ProjectId, String> = projects
            .all(db)
            .await?
            .into_iter()
            .map(|project| (project.id, project.name.as_str().to_owned()))
            .collect();
        let versions = version::Entity::find()
            .filter(version::Column::DeletedAt.is_null())
            .filter(version::Column::ProjectId.is_in(projects.keys().copied()))
            .all(db)
            .await?
            .into_iter()
            .map(|version| (version.id, version))
            .collect();
        Ok(Self { projects, versions })
    }

    fn project_name(&self, id: ProjectId) -> String {
        self.projects.get(&id).cloned().unwrap_or_default()
    }
}

async fn usage<C: ConnectionTrait>(
    db: &C,
    live: &Live,
    buckets: &TimeBuckets,
) -> Result<UsageView, StoreError> {
    let requests = model_request::Entity::find()
        .filter(model_request::Column::DispatchedAt.gte(buckets.start()))
        .filter(model_request::Column::DispatchedAt.lt(buckets.end()))
        .all(db)
        .await?;
    let runs: HashMap<RunId, run::Model> = run::Entity::find()
        .filter(run::Column::Id.is_in(requests.iter().map(|request| request.run_id)))
        .all(db)
        .await?
        .into_iter()
        .map(|run| (run.id, run))
        .collect();

    let mut view = UsageView {
        writing: UsageTotals::default(),
        review: UsageTotals::default(),
        connection_tests: UsageTotals::default(),
        buckets: vec![UsageTotals::default(); buckets.count()],
        projects: Vec::new(),
    };
    let mut projects: HashMap<ProjectId, ProjectUsage> = HashMap::new();
    for request in requests {
        let Some(run) = runs
            .get(&request.run_id)
            .filter(|run| live.versions.contains_key(&run.version_id))
        else {
            continue;
        };
        let status = request_usage(&request);
        let project = projects
            .entry(run.project_id)
            .or_insert_with(|| ProjectUsage {
                project_id: run.project_id,
                name: live.project_name(run.project_id),
                writing: UsageTotals::default(),
                review: UsageTotals::default(),
            });
        if run.kind == RunKind::Review {
            view.review.add(status);
            project.review.add(status);
        } else {
            view.writing.add(status);
            project.writing.add(status);
        }
        if let Some(bucket) = buckets.index_of(request.dispatched_at) {
            view.buckets[bucket].add(status);
        }
    }
    view.projects = projects.into_values().collect();
    view.projects.sort_by(|a, b| a.name.cmp(&b.name));

    let tests = connection_test::Entity::find()
        .filter(connection_test::Column::TestedAt.gte(buckets.start()))
        .filter(connection_test::Column::TestedAt.lt(buckets.end()))
        .all(db)
        .await?;
    for test in tests {
        let answer = if test.answered {
            Answer::Received
        } else {
            Answer::Missing
        };
        view.connection_tests
            .add(UsageStatus::of(test.usage, answer));
    }
    Ok(view)
}

/// A request's usage: the counts are reported when the service sent a usage object, and
/// an answer was recorded when the request has counts at all.
fn request_usage(request: &model_request::Model) -> UsageStatus {
    let reported = request
        .raw_usage
        .as_ref()
        .map(|_| request.usage.unwrap_or_default());
    let answer = match (request.status, request.usage) {
        (_, Some(_)) => Answer::Received,
        (ModelRequestStatus::Dispatched, None) => Answer::Pending,
        (_, None) => Answer::Missing,
    };
    UsageStatus::of(reported, answer)
}

async fn words<C: ConnectionTrait>(
    db: &C,
    live: &Live,
    buckets: &TimeBuckets,
) -> Result<WordsView, StoreError> {
    let bodies = document::Entity::find()
        .filter(document::Column::Role.eq(DocumentRole::ChapterBody))
        .filter(document::Column::VersionId.is_in(live.versions.keys().copied()))
        .all(db)
        .await?;
    let project_of: HashMap<_, ProjectId> = bodies
        .iter()
        .filter_map(|body| {
            let version = live.versions.get(&body.version_id)?;
            Some((body.id, version.project_id))
        })
        .collect();

    let mut view = WordsView {
        net: 0,
        buckets: vec![0; buckets.count()],
        projects: Vec::new(),
        total: 0,
    };
    let changes = revision::Entity::find()
        .filter(revision::Column::DocumentId.is_in(project_of.keys().copied()))
        .filter(revision::Column::CreatedAt.gte(buckets.start()))
        .filter(revision::Column::CreatedAt.lt(buckets.end()))
        .filter(revision::Column::WordDelta.is_not_null())
        .all(db)
        .await?;
    let mut projects: HashMap<ProjectId, i64> = HashMap::new();
    for change in changes {
        let delta = i64::from(change.word_delta.unwrap_or_default());
        view.net += delta;
        if let Some(bucket) = buckets.index_of(change.created_at) {
            view.buckets[bucket] += delta;
        }
        if let Some(project) = project_of.get(&change.document_id) {
            *projects.entry(*project).or_default() += delta;
        }
    }
    view.projects = projects
        .into_iter()
        .map(|(project_id, net)| ProjectWords {
            project_id,
            name: live.project_name(project_id),
            net,
        })
        .collect();
    view.projects.sort_by(|a, b| a.name.cmp(&b.name));

    let current = revision::Entity::find()
        .filter(
            revision::Column::Id.is_in(bodies.iter().filter_map(|body| body.current_revision_id)),
        )
        .all(db)
        .await?;
    view.total = current
        .iter()
        .map(|revision| i64::from(revision.word_count.unwrap_or_default()))
        .sum();
    Ok(view)
}

/// The versions' current runs: those working or waiting for the user, and those waiting to
/// be continued.
async fn jobs<C: ConnectionTrait>(
    db: &C,
    live: &Live,
) -> Result<(Vec<JobView>, Vec<JobView>), StoreError> {
    let active: HashSet<RunId> = workflow::Entity::find()
        .filter(workflow::Column::VersionId.is_in(live.versions.keys().copied()))
        .all(db)
        .await?
        .into_iter()
        .filter_map(|workflow| workflow.active_run_id)
        .collect();
    let runs = run::Entity::find()
        .filter(run::Column::Id.is_in(active))
        .all(db)
        .await?;
    let chapters: HashMap<_, chapter::Model> = chapter::Entity::find()
        .filter(chapter::Column::Id.is_in(runs.iter().filter_map(|run| run.chapter_id)))
        .all(db)
        .await?
        .into_iter()
        .map(|chapter| (chapter.id, chapter))
        .collect();
    let delegated = review::Entity::find()
        .filter(review::Column::RunId.is_in(runs.iter().map(|run| run.id)))
        .filter(review::Column::Status.eq(ReviewStatus::Delegated))
        .all(db)
        .await?;
    let agents: HashMap<RunId, run::Model> = run::Entity::find()
        .filter(
            run::Column::Id.is_in(
                delegated
                    .iter()
                    .filter_map(|review| review.delegated_run_id),
            ),
        )
        .all(db)
        .await?
        .into_iter()
        .map(|agent| (agent.id, agent))
        .collect();

    let (mut jobs, mut attention) = (Vec::new(), Vec::new());
    for run in runs {
        let Some(version) = live.versions.get(&run.version_id) else {
            continue;
        };
        let review_agent = delegated
            .iter()
            .filter(|review| review.run_id == run.id)
            .find_map(|review| Some(agents.get(&review.delegated_run_id?)?.status));
        let job = JobView {
            run_id: run.id,
            kind: run.kind,
            status: run.status,
            halt: run.halt.clone(),
            project_id: run.project_id,
            project_name: live.project_name(run.project_id),
            version_id: run.version_id,
            version_number: version.number,
            chapter: run
                .chapter_id
                .and_then(|id| chapters.get(&id))
                .map(|chapter| ChapterLabel {
                    kind: chapter.kind,
                    number: chapter.number,
                    title: chapter.title.clone(),
                }),
            review_agent,
            updated_at: run.updated_at,
        };
        if run.status.can_continue() {
            attention.push(job);
        } else if run.status.is_active() {
            jobs.push(job);
        }
    }
    for list in [&mut jobs, &mut attention] {
        list.sort_by_key(|job| std::cmp::Reverse(job.updated_at));
    }
    Ok((jobs, attention))
}
