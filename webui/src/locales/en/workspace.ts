import type { ArtifactKind } from "@/api/generated/ArtifactKind"
import type { ChapterKind } from "@/api/generated/ChapterKind"
import type { ChapterStatus } from "@/api/generated/ChapterStatus"
import type { ChapterStep } from "@/api/generated/ChapterStep"
import type { FeedbackDelivery } from "@/api/generated/FeedbackDelivery"
import type { FeedbackStatus } from "@/api/generated/FeedbackStatus"
import type { PreparationStep } from "@/api/generated/PreparationStep"
import type { ReviewStatus } from "@/api/generated/ReviewStatus"
import type { RunHalt } from "@/api/generated/RunHalt"
import type { RunStatus } from "@/api/generated/RunStatus"
import type { ThinkingLevel } from "@/api/generated/ThinkingLevel"

export default {
  backToWorks: "Works",
  draft: "Draft saved in this browser",
  dock: "Current step",
  header: {
    version: "Version {{number}}",
    versions: "Versions",
    versionItem: "Version {{number}}: {{title}}",
    newVersion: "New version…",
    settings: "Work settings",
  },
  stage: {
    preparation: {
      not_started: "Brief",
      generating: "Preparing",
      reviewing: "Reviewing the preparation",
      revising: "Revising the preparation",
    } satisfies Record<PreparationStep, string>,
    chapter: {
      ready: "Chapter ready",
      outline_generating: "Outlining a chapter",
      outline_reviewing: "Reviewing a chapter outline",
      drafting: "Writing a chapter",
      reviewing: "Reviewing a chapter",
      done: "Chapter done",
    } satisfies Record<ChapterStep, string>,
    completed: "Completed",
  },
  nav: {
    label: "Contents",
    open: "Show contents",
    preview: "Live preview",
    preparation: "Preparation",
    brief: "Novel brief",
    chapters: "Chapters",
    noChapters: "Chapters appear once the preparation is approved.",
  },
  chapterStatus: {
    not_started: "Not started",
    in_progress: "In progress",
    approved: "Approved",
  } satisfies Record<ChapterStatus, string>,
  artifact: {
    background: "Background",
    research: "Research",
    characters: "Characters",
    outline: "Outline",
  } satisfies Record<ArtifactKind, string>,
  reviewStatus: {
    open: "To review",
    delegated: "With the review agent",
    approved: "Approved",
    rejected: "Changes requested",
    stale: "Needs an update",
  } satisfies Record<ReviewStatus, string>,
  document: {
    revision: "Revision {{number}}",
    notWritten: "Not written yet.",
    chapterPlan: "Chapter plan",
    planItem: "{{number}}. {{title}}",
  },
  brief: {
    label: "Novel brief",
    description:
      "Describe the novel: genre, premise, characters, tone, and anything the agents should know. You can change it until preparation starts.",
    save: "Save brief",
    saved: "Saved",
    conflict: "The saved brief changed after this draft was started. Check it before saving.",
    empty: "No brief yet.",
  },
  preparation: {
    start: "Start preparation",
    startHint:
      "From the brief, the agent writes the background, research, characters and an outline with a chapter plan. You review each of them.",
    saveFirst: "Save the brief before starting.",
    generating: "The agent is writing the preparation documents.",
    revising: "The agent is revising: {{documents}}.",
    reviewTabs: "Preparation documents",
    approve: "Approve {{document}}",
    requestChanges: "Request changes",
    feedback: "What should change?",
    feedbackHint:
      "The agent revises with this feedback once every document of this round is decided.",
    decided: "Decided. The agent continues once every document of this round is decided.",
    stale: "A document this one depends on changed, so the agent will update it in the next round.",
    approvedNote: "Approved.",
    rejectedNote: "Changes requested: {{feedback}}",
    waiting: "Nothing to review in this document right now.",
    done: "The preparation is approved.",
  },
  chapterName: {
    main: "Chapter {{number}}: {{title}}",
    extra: "Extra {{number}}: {{title}}",
  } satisfies Record<ChapterKind, string>,
  writing: {
    pause: "Pause",
    pausing: "The agent pauses before its next change.",
    cancelPause: "Cancel pause",
    paused: "Paused before this change:",
    heldAppend: "Add text to the end of the chapter.",
    heldReplace: "Replace “{{old}}” with “{{new}}”.",
    heldDelete: "Delete “{{old}}”.",
    heldSubmit: "Submit the chapter for review.",
    continue: "Continue",
    feedback: "Feedback for the writer",
    feedbackHint:
      "It goes with the agent's next tool result, so it never interrupts a change in progress.",
    send: "Send feedback",
    sendAndContinue: "Send feedback and continue",
    queue: "Your feedback",
    reviewEachEdit: "Review each edit before it is applied",
    status: {
      queued: "Waiting for the next result",
      bound: "Goes with the next request",
      included: "Sent",
    } satisfies Record<FeedbackStatus, string>,
    delivery: {
      sent: "Sent; waiting for the answer",
      failed: "Not delivered: the request failed",
      unknown: "Delivery unknown: NekoWriter stopped during the request",
      cancelled: "Not delivered: the run was stopped",
    } satisfies Record<FeedbackDelivery, string>,
  },
  question: {
    title: "The agent asks",
    required: "Required",
    other: "Or answer in your own words",
    answer: "Your answer",
    submit: "Submit answer",
  },
  delegation: {
    delegate: "Let the review agent decide",
    delegateHint: "Your feedback and citations so far go to the review agent as notes.",
    delegated: "The review agent is reviewing this.",
    cancel: "Cancel and review myself",
    failed:
      "The review agent could not finish: {{reason}}. Review it yourself or delegate it again.",
    reason: {
      request_limit: "it ran out of requests",
      protocol_error_limit: "it did not follow the review protocol",
      model_not_ready: "the model cannot be used",
      context_limit_exceeded: "the text is too long for the model",
      provider: "the model service failed",
      process_stopped: "NekoWriter stopped while it worked",
      stopped_by_user: "it was cancelled",
      internal: "NekoWriter ran into an unexpected problem",
    } satisfies Record<RunHalt["kind"], string>,
    decided: "Decided by the review agent, {{confidence}} confident.",
  },
  proposal: {
    append: "The agent proposes adding text to the end of the chapter.",
    replace: "The agent proposes replacing “{{old}}” with “{{new}}”.",
    delete: "The agent proposes deleting “{{old}}”.",
    feedback: "Feedback (leave empty and cite nothing to accept the edit)",
    hint: "Accepting applies the edit. With feedback or a citation, the edit is rejected and the agent tries again.",
    accept: "Accept edit",
    reject: "Reject with feedback",
    label: "Proposed edit, not applied yet",
    added: "Added",
    removed: "Removed",
  },
  citations: {
    label: "Cited passages",
    cite: "Cite selected text",
    remove: "Remove citation “{{quote}}”",
    refused: {
      nothing: "Select a passage in the manuscript or a document first.",
      several_texts: "A citation stays within one text. Select a passage in a single chapter or document.",
    },
  },
  reader: {
    label: "Manuscript",
    empty: "Not written yet.",
    newContent: "New content below",
    writing: "Being written",
    editing: "The agent is changing the text…",
    held: "Held, not applied yet",
  },
  chapter: {
    goal: "Goal: {{goal}}",
    guidance: "Guidance for {{chapter}}",
    guidanceHint:
      "Optional. What should this chapter do or avoid? The agent outlines it from its goal and your guidance.",
    start: "Start outline",
    outlining: "The agent is outlining {{chapter}}.",
    outline: "Outline of {{chapter}}",
    approveOutline: "Approve outline",
    drafting: "The agent is writing {{chapter}}.",
    words_one: "{{count, number}} word",
    words_other: "{{count, number}} words",
    summary: "The agent's summary of {{chapter}}",
    approveChapter: "Approve chapter",
    reviewHint: "The agent revises with this feedback.",
    done: "{{chapter}} is approved.",
    nextChapter: "Next chapter",
    completionHint:
      "Every planned chapter is approved. Confirm the novel complete; you can still add extra chapters afterwards.",
    confirmCompletion: "Confirm the novel complete",
    finishExtra: "Finish extra chapter",
    completed: "The novel is complete.",
    extraTitle: "Title of an extra chapter",
    extraHint: "Extra chapters follow the same steps; the novel stays complete.",
    addExtra: "Add extra chapter",
  },
  run: {
    status: {
      queued: "Waiting for the model",
      running: "Writing",
      waiting_review: "Waiting for your review",
      paused: "Paused",
      waiting_answer: "Waiting for your answer",
      interrupted: "Interrupted",
      stopped: "Stopped",
      failed: "Failed",
      succeeded: "Done",
    } satisfies Record<RunStatus, string>,
    requests: "{{count}} of {{limit}} requests used",
    continue: "Continue",
    stop: "Stop",
    halt: {
      request_limit:
        "The run used all of its allowed requests. Continuing allows another 100 requests.",
      protocol_error_limit:
        "The model did not follow the writing protocol three times in a row. Continue to try again; another model may work better.",
      model_not_ready: "{{problem}} Then continue.",
      context_limit_exceeded:
        "The request needs about {{estimated}} tokens, but the model accepts {{limit}}. Choose a model with a larger context window, then continue.",
      provider: "The model service failed: {{failure}}. Check the service, then continue.",
      process_stopped:
        "NekoWriter stopped while this run was working. Continue when you are ready.",
      stopped_by_user:
        "You stopped the run. Its text and your feedback are kept; continue when you are ready.",
      internal:
        "NekoWriter ran into an unexpected problem; the details are in its log. Continue to try again.",
    } satisfies Record<RunHalt["kind"], string>,
  },
  preview: {
    label: "Live preview",
    note: "Still being written; nothing here is saved until the agent submits it.",
    notes: "The agent's notes",
  },
  newVersion: {
    title: "New version",
    description:
      "A new version is an empty manuscript with its own brief and preparation. The current version stays as it is.",
    novelTitle: "Novel title",
    submit: "Create version",
    done: "Created version {{number}}.",
  },
  settings: {
    title: "Work settings",
    writingLanguage: "Writing language",
    writingLanguageHint: "The agents write the novel in this language.",
    ownModel: "Use its own model",
    ownModelHint: "Otherwise the work uses the default model from Settings.",
    thinking: "Thinking",
    thinkingLevel: {
      off: "Off",
      low: "Low",
      medium: "Medium",
      high: "High",
    } satisfies Record<ThinkingLevel, string>,
    thinkingUnsupported: "The model this work uses does not support thinking.",
    submit: "Save",
    done: "Saved the work settings.",
  },
} as const
