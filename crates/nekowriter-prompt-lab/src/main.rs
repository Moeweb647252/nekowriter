use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand, ValueEnum};
use nekowriter_agent::{PromptKey, validate_builtin_prompt_pair};
use reqwest::{Client, Method, StatusCode};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MANIFEST_FILE: &str = "manifest.json";
const CASE_FILE: &str = "case.json";
const MODEL_FILE: &str = "model.json";
const PROMPTS_DIR: &str = "prompts";
const EVIDENCE_DIR: &str = "evidence";
const LATEST_REPORT_FILE: &str = "evidence/latest.json";
const DATA_DIR: &str = "data";
const SERVICE_LOG_FILE: &str = "service.log";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Parser)]
#[command(
    name = "nekowriter-prompt-lab",
    version,
    about = "以真实 NekoWriter 产出验证提示词改动"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Debug, Subcommand)]
enum CommandLine {
    /// 校验一套内建提示词覆盖是否仍满足模板合同。
    Validate {
        #[arg(long)]
        prompts: PathBuf,
    },
    /// 创建一个隔离实验，冻结案例、模型配置和提示词副本。
    Init {
        #[arg(long)]
        case: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        prompts: PathBuf,
        experiment: PathBuf,
    },
    /// 启动真实服务并运行到指定阶段或下一个人工边界。
    Run {
        experiment: PathBuf,
        #[arg(long)]
        app: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = StopAt::Human)]
        until: StopAt,
        #[arg(long, default_value_t = 900)]
        timeout_seconds: u64,
    },
    /// 在隔离实验中提交外部 Agent 作出的决定，然后继续运行。
    Act {
        experiment: PathBuf,
        #[arg(long)]
        action: PathBuf,
        #[arg(long)]
        app: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = StopAt::Human)]
        until: StopAt,
        #[arg(long, default_value_t = 900)]
        timeout_seconds: u64,
    },
    /// 输出上一次边界快照及该实验收集到的原始模型请求与消息账本。
    Inspect {
        experiment: PathBuf,
        #[arg(long)]
        run: Option<Uuid>,
    },
    /// 并列输出两个已结束实验的可比事实，不自动替代编辑判断。
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum StopAt {
    /// 停在等待审核、等待回答、失败、停止、完成或没有活跃运行的边界。
    Human,
    Preparation,
    Outline,
    ChapterFinal,
    Complete,
}

#[derive(Debug, Error)]
enum PromptLabError {
    #[error("文件系统错误：{0}")]
    Io(#[from] io::Error),
    #[error("JSON 错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP 错误：{0}")]
    Http(#[from] reqwest::Error),
    #[error("数据库错误：{0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("{0}")]
    Message(String),
}

type Result<T> = std::result::Result<T, PromptLabError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExperimentCase {
    name: String,
    novel_title: String,
    novel_brief: String,
    #[serde(default)]
    chapter_guidance: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelConfig {
    api_base_url: String,
    api_key_env: String,
    model_id: String,
    capabilities: ModelCapabilities,
    max_context_tokens: i64,
    reserved_output_tokens: i64,
}

/// 与 `NekoWriter` 写作模型设置一一对应，不能把独立能力压缩为推导规则。
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelCapabilities {
    streaming: bool,
    tools: bool,
    stream_usage: bool,
    #[serde(default)]
    thinking: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExperimentManifest {
    format_version: u8,
    case_sha256: String,
    model_sha256: String,
    prompt_sha256: String,
    project_id: Option<Uuid>,
    version_id: Option<Uuid>,
    #[serde(default)]
    run_ids: BTreeSet<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Review {
        review_id: Uuid,
        expected_state_revision: String,
        passed: bool,
        #[serde(default)]
        issues: String,
        #[serde(default)]
        citations: Vec<Value>,
    },
    Answer {
        question_id: Uuid,
        expected_state_revision: String,
        answers: Vec<Value>,
    },
    StartNextChapter {
        expected_state_revision: String,
        guidance: String,
    },
    StartOutline {
        chapter_id: Uuid,
        expected_state_revision: String,
    },
    ContinueRun {
        run_id: Uuid,
        expected_state_revision: String,
    },
    ConfirmCompletion {
        expected_state_revision: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    manifest: ExperimentManifest,
    state: Value,
    runs: Vec<RunTrace>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunTrace {
    run_id: Uuid,
    run: Value,
    model_requests: Vec<Value>,
    agent_messages: Vec<Value>,
}

struct LocalService {
    child: Child,
    client: Client,
    api_base: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        CommandLine::Validate { prompts } => {
            let digest = validate_prompt_directory(&prompts)?;
            print_json(&json!({ "valid": true, "promptSha256": digest }));
        }
        CommandLine::Init {
            case,
            model,
            prompts,
            experiment,
        } => init_experiment(&case, &model, &prompts, &experiment)?,
        CommandLine::Run {
            experiment,
            app,
            until,
            timeout_seconds,
        } => run_experiment(&experiment, app.as_deref(), until, timeout_seconds).await?,
        CommandLine::Act {
            experiment,
            action,
            app,
            until,
            timeout_seconds,
        } => {
            act_on_experiment(&experiment, &action, app.as_deref(), until, timeout_seconds).await?;
        }
        CommandLine::Inspect { experiment, run } => inspect_experiment(&experiment, run)?,
        CommandLine::Compare {
            baseline,
            candidate,
        } => compare_experiments(&baseline, &candidate)?,
    }
    Ok(())
}

fn init_experiment(case: &Path, model: &Path, prompts: &Path, experiment: &Path) -> Result<()> {
    ensure_empty_or_missing(experiment)?;
    let experiment_case = read_case(case)?;
    let model_config = read_model(model)?;
    let prompt_sha256 = validate_prompt_directory(prompts)?;

    fs::create_dir_all(experiment.join(EVIDENCE_DIR))?;
    copy_prompt_directory(prompts, &experiment.join(PROMPTS_DIR))?;
    write_json_file(&experiment.join(CASE_FILE), &experiment_case)?;
    write_json_file(&experiment.join(MODEL_FILE), &model_config)?;
    let manifest = ExperimentManifest {
        format_version: 1,
        case_sha256: sha256_file(&experiment.join(CASE_FILE))?,
        model_sha256: sha256_file(&experiment.join(MODEL_FILE))?,
        prompt_sha256,
        project_id: None,
        version_id: None,
        run_ids: BTreeSet::new(),
    };
    save_manifest(experiment, &manifest)?;
    print_json(&json!({
        "experiment": experiment,
        "next": "run",
        "apiKey": format!("从 {} 读取；不会写入案例、模型配置或证据报告", model_config.api_key_env),
    }));
    Ok(())
}

async fn run_experiment(
    experiment: &Path,
    app: Option<&Path>,
    until: StopAt,
    timeout_seconds: u64,
) -> Result<()> {
    let mut manifest = load_manifest(experiment)?;
    verify_frozen_inputs(experiment, &mut manifest)?;
    let model = read_model(&experiment.join(MODEL_FILE))?;
    let app = resolve_app(app)?;
    let mut service = LocalService::launch(&app, experiment).await?;
    let outcome: Result<Value> = async {
        service.configure_model(&model).await?;
        initialize_if_needed(&service, experiment, &mut manifest).await?;
        let state = wait_for_boundary(
            &service,
            manifest
                .version_id
                .ok_or_else(|| PromptLabError::Message("实验缺少 versionId".to_owned()))?,
            until,
            Duration::from_secs(timeout_seconds),
            &mut manifest,
        )
        .await?;
        save_manifest(experiment, &manifest)?;
        Ok(state)
    }
    .await;
    finish_service(&mut service, &model).await?;
    let state = outcome?;
    let report = save_report(experiment, &manifest, state).await?;
    print_json(&boundary_output(experiment, &report));
    Ok(())
}

async fn act_on_experiment(
    experiment: &Path,
    action_path: &Path,
    app: Option<&Path>,
    until: StopAt,
    timeout_seconds: u64,
) -> Result<()> {
    let mut manifest = load_manifest(experiment)?;
    verify_frozen_inputs(experiment, &mut manifest)?;
    let version_id = manifest
        .version_id
        .ok_or_else(|| PromptLabError::Message("实验尚未初始化；请先运行 run".to_owned()))?;
    let model = read_model(&experiment.join(MODEL_FILE))?;
    let action: Action = read_json(action_path)?;
    let app = resolve_app(app)?;
    let mut service = LocalService::launch(&app, experiment).await?;
    let outcome: Result<Value> = async {
        service.configure_model(&model).await?;
        apply_action(&service, version_id, action).await?;
        let state = wait_for_boundary(
            &service,
            version_id,
            until,
            Duration::from_secs(timeout_seconds),
            &mut manifest,
        )
        .await?;
        save_manifest(experiment, &manifest)?;
        Ok(state)
    }
    .await;
    finish_service(&mut service, &model).await?;
    let state = outcome?;
    let report = save_report(experiment, &manifest, state).await?;
    print_json(&boundary_output(experiment, &report));
    Ok(())
}

fn inspect_experiment(experiment: &Path, run: Option<Uuid>) -> Result<()> {
    let report: Report = read_json(&experiment.join(LATEST_REPORT_FILE))?;
    if let Some(run_id) = run {
        let trace = report
            .runs
            .iter()
            .find(|trace| trace.run_id == run_id)
            .ok_or_else(|| PromptLabError::Message(format!("报告中没有 Run {run_id}")))?;
        print_json(trace);
    } else {
        print_json(&report);
    }
    Ok(())
}

fn compare_experiments(baseline: &Path, candidate: &Path) -> Result<()> {
    let baseline_report: Report = read_json(&baseline.join(LATEST_REPORT_FILE))?;
    let candidate_report: Report = read_json(&candidate.join(LATEST_REPORT_FILE))?;
    if baseline_report.manifest.case_sha256 != candidate_report.manifest.case_sha256 {
        return Err(PromptLabError::Message(
            "两个实验没有使用同一冻结案例，不能比较".to_owned(),
        ));
    }
    if baseline_report.manifest.model_sha256 != candidate_report.manifest.model_sha256 {
        return Err(PromptLabError::Message(
            "两个实验没有使用同一冻结模型配置，不能比较".to_owned(),
        ));
    }
    print_json(&json!({
        "caseSha256": baseline_report.manifest.case_sha256,
        "modelSha256": baseline_report.manifest.model_sha256,
        "baseline": comparison_side(baseline, &baseline_report),
        "candidate": comparison_side(candidate, &candidate_report),
        "decision": "请比较 evidence/latest.json 中的实际产出与回归样本；此命令不把运行状态或审核通过率误当成写作质量。",
    }));
    Ok(())
}

fn comparison_side(experiment: &Path, report: &Report) -> Value {
    let mut statuses = BTreeMap::<String, usize>::new();
    let mut request_count = 0_usize;
    for trace in &report.runs {
        for request in &trace.model_requests {
            request_count += 1;
            if let Some(status) = request.get("status").and_then(Value::as_str) {
                *statuses.entry(status.to_owned()).or_default() += 1;
            }
        }
    }
    json!({
        "experiment": experiment,
        "promptSha256": report.manifest.prompt_sha256,
        "stage": value_at(&report.state, "/preparation/workflow/stage"),
        "execution": value_at(&report.state, "/preparation/workflow/execution"),
        "modelRequestCount": request_count,
        "modelRequestStatuses": statuses,
        "report": experiment.join(LATEST_REPORT_FILE),
    })
}

async fn initialize_if_needed(
    service: &LocalService,
    experiment: &Path,
    manifest: &mut ExperimentManifest,
) -> Result<()> {
    if manifest.project_id.is_some() && manifest.version_id.is_some() {
        return Ok(());
    }
    if manifest.project_id.is_some() || manifest.version_id.is_some() {
        return Err(PromptLabError::Message(
            "实验清单中的 projectId 与 versionId 必须同时存在或同时为空".to_owned(),
        ));
    }
    let case = read_case(&experiment.join(CASE_FILE))?;
    let bundle = service
        .post(
            "/projects",
            json!({
                "commandId": Uuid::new_v4(),
                "name": case.name,
                "novelTitle": case.novel_title,
                "versionLabel": "prompt-lab",
            }),
        )
        .await?;
    let project_id = uuid_at(&bundle, "/project/id")?;
    let version_id = uuid_at(&bundle, "/version/id")?;
    upload_prompt_overrides(service, project_id, &experiment.join(PROMPTS_DIR)).await?;
    service
        .put(
            &format!("/versions/{version_id}/brief"),
            json!({ "commandId": Uuid::new_v4(), "content": case.novel_brief }),
        )
        .await?;
    let preparation = service
        .get(&format!("/versions/{version_id}/preparation"))
        .await?;
    let state_revision = string_at(&preparation, "/workflow/stateRevision")?;
    service
        .post(
            &format!("/versions/{version_id}/preparation/start"),
            json!({
                "commandId": Uuid::new_v4(),
                "expectedStateRevision": state_revision,
            }),
        )
        .await?;
    manifest.project_id = Some(project_id);
    manifest.version_id = Some(version_id);
    Ok(())
}

async fn upload_prompt_overrides(
    service: &LocalService,
    project_id: Uuid,
    prompts: &Path,
) -> Result<()> {
    validate_prompt_directory(prompts)?;
    for key in PromptKey::ALL {
        let stable_key = key.as_str();
        let system = fs::read_to_string(prompts.join(format!("{stable_key}.system.md")))?;
        let user = fs::read_to_string(prompts.join(format!("{stable_key}.user.md")))?;
        service
            .put(
                &format!("/projects/{project_id}/prompts/{stable_key}"),
                json!({
                    "commandId": Uuid::new_v4(),
                    "expectedRevision": "0",
                    "system": system,
                    "user": user,
                }),
            )
            .await?;
    }
    Ok(())
}

async fn apply_action(service: &LocalService, version_id: Uuid, action: Action) -> Result<()> {
    match action {
        Action::Review {
            review_id,
            expected_state_revision,
            passed,
            issues,
            citations,
        } => {
            service
                .post(
                    &format!("/reviews/{review_id}/decision"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                        "passed": passed,
                        "issues": issues,
                        "citations": citations,
                    }),
                )
                .await?;
        }
        Action::Answer {
            question_id,
            expected_state_revision,
            answers,
        } => {
            service
                .post(
                    &format!("/questions/{question_id}/answer"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                        "answers": answers,
                    }),
                )
                .await?;
        }
        Action::StartNextChapter {
            expected_state_revision,
            guidance,
        } => {
            let next = service
                .post(
                    &format!("/versions/{version_id}/chapters/next"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                    }),
                )
                .await?;
            let chapter_id = uuid_at(&next, "/workflow/activeChapterId")?;
            let state_revision = string_at(&next, "/workflow/stateRevision")?;
            service
                .put(
                    &format!("/chapters/{chapter_id}/guidance"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": state_revision,
                        "content": guidance,
                    }),
                )
                .await?;
            let outline = service
                .get(&format!("/chapters/{chapter_id}/outline"))
                .await?;
            let state_revision = string_at(&outline, "/workflow/stateRevision")?;
            service
                .post(
                    &format!("/chapters/{chapter_id}/outline/start"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": state_revision,
                    }),
                )
                .await?;
        }
        Action::StartOutline {
            chapter_id,
            expected_state_revision,
        } => {
            service
                .post(
                    &format!("/chapters/{chapter_id}/outline/start"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                    }),
                )
                .await?;
        }
        Action::ContinueRun {
            run_id,
            expected_state_revision,
        } => {
            service
                .post(
                    &format!("/runs/{run_id}/continue"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                    }),
                )
                .await?;
        }
        Action::ConfirmCompletion {
            expected_state_revision,
        } => {
            service
                .post(
                    &format!("/versions/{version_id}/completion"),
                    json!({
                        "commandId": Uuid::new_v4(),
                        "expectedStateRevision": expected_state_revision,
                    }),
                )
                .await?;
        }
    }
    Ok(())
}

async fn wait_for_boundary(
    service: &LocalService,
    version_id: Uuid,
    until: StopAt,
    timeout: Duration,
    manifest: &mut ExperimentManifest,
) -> Result<Value> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let state = service.snapshot(version_id).await?;
        record_run_ids(&state, &mut manifest.run_ids);
        if should_stop(&state, until) {
            return Ok(state);
        }
        if std::time::Instant::now() >= deadline {
            return Err(PromptLabError::Message(format!(
                "等待 {until:?} 超时；最后状态是 stage={} execution={}",
                string_at(&state, "/preparation/workflow/stage")
                    .unwrap_or_else(|_| "unknown".to_owned()),
                string_at(&state, "/preparation/workflow/execution")
                    .unwrap_or_else(|_| "unknown".to_owned()),
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn should_stop(state: &Value, until: StopAt) -> bool {
    let workflow_stage = value_at(state, "/preparation/workflow/stage").and_then(Value::as_str);
    let execution = value_at(state, "/preparation/workflow/execution").and_then(Value::as_str);
    match until {
        StopAt::Preparation => workflow_stage == Some("preparing_reviewing"),
        StopAt::Outline => workflow_stage == Some("writing_outline_reviewing"),
        StopAt::ChapterFinal => workflow_stage == Some("writing_chapter_reviewing"),
        StopAt::Complete => workflow_stage == Some("completed"),
        StopAt::Human => {
            matches!(
                execution,
                Some("waiting_answer" | "waiting_review" | "failed" | "stopped" | "interrupted")
            ) || workflow_stage == Some("completed")
                || (execution == Some("idle") && workflow_stage == Some("writing_chapter_done"))
        }
    }
}

impl LocalService {
    async fn launch(app: &Path, experiment: &Path) -> Result<Self> {
        let port = free_port()?;
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(experiment.join(SERVICE_LOG_FILE))?;
        let child = Command::new(app)
            .arg("--data-dir")
            .arg(experiment.join(DATA_DIR))
            .arg("--port")
            .arg(port.to_string())
            .arg("--no-browser")
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file))
            .spawn()?;
        let service = Self {
            child,
            client: Client::new(),
            api_base: format!("http://127.0.0.1:{port}/api"),
        };
        service.wait_until_healthy().await?;
        Ok(service)
    }

    async fn wait_until_healthy(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + HEALTH_TIMEOUT;
        loop {
            if let Ok(response) = self
                .client
                .get(format!("{}/health", self.api_base))
                .send()
                .await
                && response.status() == StatusCode::OK
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(PromptLabError::Message(format!(
                    "NekoWriter 服务未在 {} 内启动；查看 {}",
                    format_duration(HEALTH_TIMEOUT),
                    SERVICE_LOG_FILE
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn configure_model(&self, model: &ModelConfig) -> Result<()> {
        let api_key = std::env::var(&model.api_key_env).map_err(|_| {
            PromptLabError::Message(format!("环境变量 {} 未设置或为空", model.api_key_env))
        })?;
        if api_key.trim().is_empty() {
            return Err(PromptLabError::Message(format!(
                "环境变量 {} 不能为空",
                model.api_key_env
            )));
        }
        let settings = self.get("/settings").await?;
        let revision = value_at(&settings, "/writingModel/revision")
            .and_then(Value::as_str)
            .unwrap_or("0");
        self.put(
            "/settings",
            json!({
                "commandId": Uuid::new_v4(),
                "expectedRevision": revision,
                "apiBaseUrl": model.api_base_url,
                "apiKey": api_key,
                "modelId": model.model_id,
                "capabilities": model.capabilities,
                "maxContextTokens": model.max_context_tokens.to_string(),
                "reservedOutputTokens": model.reserved_output_tokens.to_string(),
            }),
        )
        .await?;
        Ok(())
    }

    async fn clear_model_key(&self, model: &ModelConfig) -> Result<()> {
        let settings = self.get("/settings").await?;
        let Some(writing_model) = settings.get("writingModel") else {
            return Ok(());
        };
        let revision = string_at(writing_model, "/revision")?;
        self.put(
            "/settings",
            json!({
                "commandId": Uuid::new_v4(),
                "expectedRevision": revision,
                "apiBaseUrl": model.api_base_url,
                "apiKey": Value::Null,
                "modelId": model.model_id,
                "capabilities": model.capabilities,
                "maxContextTokens": model.max_context_tokens.to_string(),
                "reservedOutputTokens": model.reserved_output_tokens.to_string(),
            }),
        )
        .await?;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        let shutdown_result = self.post("/system/shutdown", json!({})).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if self.child.try_wait()?.is_some() {
                return shutdown_result.map(|_| ());
            }
            if std::time::Instant::now() >= deadline {
                self.child.kill()?;
                self.child.wait()?;
                return Err(PromptLabError::Message(
                    "NekoWriter 未在正常退出期限内停止，已终止其子进程".to_owned(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn snapshot(&self, version_id: Uuid) -> Result<Value> {
        let preparation = self
            .get(&format!("/versions/{version_id}/preparation"))
            .await?;
        let chapters = self
            .get(&format!("/versions/{version_id}/chapters"))
            .await?;
        let mut chapter_states = Vec::new();
        let chapter_list = chapters
            .as_array()
            .ok_or_else(|| PromptLabError::Message("服务返回的章节目录不是数组".to_owned()))?;
        for chapter in chapter_list {
            let chapter_id = uuid_at(chapter, "/id")?;
            chapter_states.push(json!({
                "summary": chapter,
                "outline": self.get(&format!("/chapters/{chapter_id}/outline")).await?,
                "body": self.get(&format!("/chapters/{chapter_id}/body")).await?,
            }));
        }
        Ok(json!({ "preparation": preparation, "chapters": chapter_states }))
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::PUT, path, Some(body)).await
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{}", self.api_base, path);
        let request = self.client.request(method, url);
        let response = match body {
            Some(body) => request.json(&body).send().await?,
            None => request.send().await?,
        };
        let status = response.status();
        let response_body = response.text().await?;
        if !status.is_success() {
            return Err(PromptLabError::Message(format!(
                "服务返回 HTTP {status}: {response_body}"
            )));
        }
        serde_json::from_str(&response_body).map_err(PromptLabError::from)
    }
}

async fn finish_service(service: &mut LocalService, model: &ModelConfig) -> Result<()> {
    let clear_result = service.clear_model_key(model).await;
    let shutdown_result = service.shutdown().await;
    clear_result?;
    shutdown_result
}

async fn save_report(
    experiment: &Path,
    manifest: &ExperimentManifest,
    state: Value,
) -> Result<Report> {
    let report = Report {
        manifest: manifest.clone(),
        runs: read_run_traces(&experiment.join(DATA_DIR), &manifest.run_ids).await?,
        state,
    };
    let timestamp = unix_timestamp_ms()?;
    write_json_file(
        &experiment
            .join(EVIDENCE_DIR)
            .join(format!("boundary-{timestamp}.json")),
        &report,
    )?;
    write_json_file(&experiment.join(LATEST_REPORT_FILE), &report)?;
    Ok(report)
}

async fn read_run_traces(data_dir: &Path, run_ids: &BTreeSet<Uuid>) -> Result<Vec<RunTrace>> {
    let database_path = data_dir.join("nekowriter.sqlite3");
    if !database_path.exists() {
        return Ok(Vec::new());
    }
    let database = Database::connect(read_only_sqlite_url(&database_path)?).await?;
    let mut traces = Vec::new();
    for run_id in run_ids {
        let run = single_row_json(
            &database,
            "SELECT id, kind, stage, status, CAST(resume_count AS TEXT) AS resume_count, CAST(request_count AS TEXT) AS request_count, CAST(request_allowance_end AS TEXT) AS request_allowance_end, checkpoint_json, CAST(started_at AS TEXT) AS started_at, CAST(ended_at AS TEXT) AS ended_at FROM runs WHERE id = ?",
            run_id,
        )
        .await?;
        let model_requests = query_json(
            &database,
            "SELECT id, CAST(attempt_no AS TEXT) AS attempt_no, status, settings_snapshot_json, prompt_snapshot_json, context_snapshot_json, provider_request_id, usage_json, error_json, CAST(created_at AS TEXT) AS created_at, CAST(updated_at AS TEXT) AS updated_at, CAST(dispatched_at AS TEXT) AS dispatched_at, CAST(completed_at AS TEXT) AS completed_at FROM model_requests WHERE run_id = ? ORDER BY attempt_no",
            run_id,
        )
        .await?;
        let agent_messages = query_json(
            &database,
            "SELECT id, CAST(sequence AS TEXT) AS sequence, role, content_json, request_id, tool_call_id, CAST(created_at AS TEXT) AS created_at FROM agent_messages WHERE run_id = ? ORDER BY sequence",
            run_id,
        )
        .await?;
        traces.push(RunTrace {
            run_id: *run_id,
            run: run.unwrap_or_else(|| json!({ "missing": true })),
            model_requests,
            agent_messages,
        });
    }
    Ok(traces)
}

async fn single_row_json(
    database: &sea_orm::DatabaseConnection,
    sql: &str,
    run_id: &Uuid,
) -> Result<Option<Value>> {
    let rows = database.query_all_raw(statement(sql, run_id)).await?;
    Ok(rows.first().map(row_to_json))
}

async fn query_json(
    database: &sea_orm::DatabaseConnection,
    sql: &str,
    run_id: &Uuid,
) -> Result<Vec<Value>> {
    Ok(database
        .query_all_raw(statement(sql, run_id))
        .await?
        .iter()
        .map(row_to_json)
        .collect())
}

fn statement(sql: &str, run_id: &Uuid) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, [run_id.to_string().into()])
}

fn row_to_json(row: &sea_orm::QueryResult) -> Value {
    let mut object = serde_json::Map::new();
    for column in row.column_names() {
        let value = row.try_get::<Option<String>>("", &column).unwrap_or(None);
        let json_value = match value {
            Some(value) if column.ends_with("_json") => {
                serde_json::from_str(&value).unwrap_or(Value::String(value))
            }
            Some(value) => Value::String(value),
            None => Value::Null,
        };
        object.insert(to_camel_case(&column), json_value);
    }
    Value::Object(object)
}

fn read_only_sqlite_url(path: &Path) -> Result<String> {
    let parent = path
        .parent()
        .ok_or_else(|| PromptLabError::Message("实验数据库路径缺少父目录".to_owned()))?;
    let absolute_path = parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| PromptLabError::Message("实验数据库路径缺少文件名".to_owned()))?,
    );
    let file_url = Url::from_file_path(absolute_path)
        .map_err(|()| PromptLabError::Message("无法构造实验数据库 URL".to_owned()))?;
    let encoded_path = file_url
        .as_str()
        .strip_prefix("file://")
        .ok_or_else(|| PromptLabError::Message("无法解析实验数据库 URL".to_owned()))?;
    #[cfg(windows)]
    let encoded_path = encoded_path.strip_prefix('/').unwrap_or(encoded_path);
    Ok(format!("sqlite://{encoded_path}?mode=ro"))
}

fn record_run_ids(value: &Value, run_ids: &mut BTreeSet<Uuid>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "activeRunId"
                    && let Some(value) = value.as_str()
                    && let Ok(id) = Uuid::parse_str(value)
                {
                    run_ids.insert(id);
                }
                record_run_ids(value, run_ids);
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| record_run_ids(value, run_ids)),
        _ => {}
    }
}

fn boundary_output(experiment: &Path, report: &Report) -> Value {
    json!({
        "experiment": experiment,
        "stage": value_at(&report.state, "/preparation/workflow/stage"),
        "execution": value_at(&report.state, "/preparation/workflow/execution"),
        "report": experiment.join(LATEST_REPORT_FILE),
        "knownRuns": report.runs.iter().map(|trace| trace.run_id).collect::<Vec<_>>(),
    })
}

fn validate_prompt_directory(prompts: &Path) -> Result<String> {
    if !prompts.is_dir() {
        return Err(PromptLabError::Message(format!(
            "提示词目录不存在：{}",
            prompts.display()
        )));
    }
    let mut hasher = Sha256::new();
    for key in PromptKey::ALL {
        let stable_key = key.as_str();
        let system_path = prompts.join(format!("{stable_key}.system.md"));
        let user_path = prompts.join(format!("{stable_key}.user.md"));
        let system = fs::read_to_string(&system_path)?;
        let user = fs::read_to_string(&user_path)?;
        validate_builtin_prompt_pair(key, &system, &user).map_err(|error| {
            PromptLabError::Message(format!("{stable_key} 未通过模板合同：{error}"))
        })?;
        hasher.update(stable_key.as_bytes());
        hasher.update([0]);
        hasher.update(system.as_bytes());
        hasher.update([0]);
        hasher.update(user.as_bytes());
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn copy_prompt_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for key in PromptKey::ALL {
        let stable_key = key.as_str();
        for suffix in ["system", "user"] {
            let file_name = format!("{stable_key}.{suffix}.md");
            fs::copy(source.join(&file_name), destination.join(file_name))?;
        }
    }
    Ok(())
}

fn ensure_empty_or_missing(path: &Path) -> Result<()> {
    if path.exists() && fs::read_dir(path)?.next().is_some() {
        return Err(PromptLabError::Message(format!(
            "实验目录必须不存在或为空：{}",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_app(argument: Option<&Path>) -> Result<PathBuf> {
    let path = match argument {
        Some(path) => path.to_path_buf(),
        None => std::env::current_exe()?
            .parent()
            .map(|parent| parent.join("nekowriter"))
            .ok_or_else(|| {
                PromptLabError::Message("无法定位 prompt-lab 可执行文件目录".to_owned())
            })?,
    };
    if !path.is_file() {
        return Err(PromptLabError::Message(format!(
            "找不到 NekoWriter 可执行文件：{}；开发环境请传 --app target/debug/nekowriter",
            path.display()
        )));
    }
    Ok(path)
}

fn read_case(path: &Path) -> Result<ExperimentCase> {
    let experiment_case: ExperimentCase = read_json(path)?;
    if experiment_case.name.trim().is_empty()
        || experiment_case.novel_title.trim().is_empty()
        || experiment_case.novel_brief.trim().is_empty()
    {
        return Err(PromptLabError::Message(
            "case.json 的 name、novelTitle 和 novelBrief 都不能为空".to_owned(),
        ));
    }
    Ok(experiment_case)
}

fn read_model(path: &Path) -> Result<ModelConfig> {
    let model: ModelConfig = read_json(path)?;
    if model.api_base_url.trim().is_empty()
        || model.api_key_env.trim().is_empty()
        || model.model_id.trim().is_empty()
        || model.max_context_tokens <= 0
        || model.reserved_output_tokens <= 0
        || model.reserved_output_tokens >= model.max_context_tokens
        || !model.capabilities.tools
    {
        return Err(PromptLabError::Message(
            "model.json 缺少有效模型设置；tools 必须为 true，且 token 预算必须有效".to_owned(),
        ));
    }
    Ok(model)
}

fn load_manifest(experiment: &Path) -> Result<ExperimentManifest> {
    read_json(&experiment.join(MANIFEST_FILE))
}

fn verify_frozen_inputs(experiment: &Path, manifest: &mut ExperimentManifest) -> Result<()> {
    let case_sha256 = sha256_file(&experiment.join(CASE_FILE))?;
    if manifest.case_sha256 != case_sha256 {
        return Err(PromptLabError::Message(
            "案例副本已变化；请新建实验，而不是改写冻结案例".to_owned(),
        ));
    }
    let model_sha256 = sha256_file(&experiment.join(MODEL_FILE))?;
    if manifest.model_sha256 != model_sha256 {
        return Err(PromptLabError::Message(
            "模型配置副本已变化；请新建实验，而不是改写冻结模型配置".to_owned(),
        ));
    }
    let prompt_sha256 = validate_prompt_directory(&experiment.join(PROMPTS_DIR))?;
    if manifest.project_id.is_none() {
        manifest.prompt_sha256 = prompt_sha256;
    } else if manifest.prompt_sha256 != prompt_sha256 {
        return Err(PromptLabError::Message(
            "实验已运行，提示词副本不能再改写；请新建候选实验".to_owned(),
        ));
    }
    Ok(())
}

fn save_manifest(experiment: &Path, manifest: &ExperimentManifest) -> Result<()> {
    write_json_file(&experiment.join(MANIFEST_FILE), manifest)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_json_file(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn uuid_at(value: &Value, pointer: &str) -> Result<Uuid> {
    let value = string_at(value, pointer)?;
    Uuid::parse_str(&value)
        .map_err(|error| PromptLabError::Message(format!("{pointer} 不是 UUID：{error}")))
}

fn string_at(value: &Value, pointer: &str) -> Result<String> {
    value_at(value, pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PromptLabError::Message(format!("服务响应缺少字符串字段 {pointer}")))
}

fn value_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    value.pointer(pointer)
}

fn unix_timestamp_ms() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PromptLabError::Message(format!("系统时间早于 UNIX epoch：{error}")))?
        .as_millis())
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn to_camel_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut upper_next = false;
    for character in value.chars() {
        if character == '_' {
            upper_next = true;
        } else if upper_next {
            output.extend(character.to_uppercase());
            upper_next = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn format_duration(duration: Duration) -> String {
    format!("{} 秒", duration.as_secs())
}

fn print_json(value: &impl Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_active_run_ids_without_scanning_unrelated_strings() {
        let value = json!({
            "activeRunId": "550e8400-e29b-41d4-a716-446655440000",
            "note": "550e8400-e29b-41d4-a716-446655440001",
            "items": [{ "activeRunId": null }],
        });
        let mut run_ids = BTreeSet::new();
        record_run_ids(&value, &mut run_ids);
        assert_eq!(run_ids.len(), 1);
    }

    #[test]
    fn human_boundary_does_not_treat_a_queued_run_as_finished() {
        let queued = json!({
            "preparation": { "workflow": { "stage": "preparing_generating", "execution": "queued" } }
        });
        assert!(!should_stop(&queued, StopAt::Human));
        let waiting = json!({
            "preparation": { "workflow": { "stage": "preparing_reviewing", "execution": "waiting_review" } }
        });
        assert!(should_stop(&waiting, StopAt::Human));
    }
}
