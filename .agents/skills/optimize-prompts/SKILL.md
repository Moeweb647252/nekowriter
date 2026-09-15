---
name: optimize-prompts
description: Use real NekoWriter experiment output to evaluate and improve one built-in prompt at a time, with isolated baseline/candidate evidence and human-controlled workflow decisions.
---

# Optimize NekoWriter prompts from real output

Use this skill when the user asks to improve a built-in NekoWriter prompt based on actual writing results. The target prompt files are `crates/nekowriter-agent/prompts/*.system.md` and `*.user.md`; the experiment CLI is `nekowriter-prompt-lab`.

Do not judge a prompt from prose alone, request success, token usage, or an internal review's `passed` flag. The deciding evidence is the produced writing and review material in the experiment report.

## Before changing a prompt

1. Identify one prompt key and one observable quality question. State what text should improve and what must not regress.
2. Read the prompt's declared variables and surrounding stage contract. Preserve all required placeholders and do not alter stage-submission tool semantics.
3. Prepare one primary case and one regression case. Keep each case byte-identical between baseline and candidate.
4. Build the application and CLI once if needed:

   ```sh
   cargo build -p nekowriter-app -p nekowriter-prompt-lab
   ```

5. Create a baseline experiment with the repository prompt directory. Create the candidate from the same frozen inputs, then change only the target prompt pair in the candidate's `prompts/` directory. Validate it before consuming model tokens:

   ```sh
   target/debug/nekowriter-prompt-lab validate --prompts <candidate>/prompts
   ```

See `docs/07-prompt-lab.md` for the case/model schema and action shapes.

## Running real evidence

Run both experiments with the same model configuration and `--app target/debug/nekowriter`. The model key belongs only in the configured environment variable; never place it in case files, actions, reports, commits, or chat output.

```sh
target/debug/nekowriter-prompt-lab run <experiment> --app target/debug/nekowriter
target/debug/nekowriter-prompt-lab inspect <experiment>
```

At every `waiting_review` or `waiting_answer` boundary:

1. Read `evidence/latest.json`, including the generated text, review feedback, current `workflow.stateRevision`, and the specific review/question ID.
2. Make the editorial decision yourself. Do not automatically approve a review just to advance the run.
3. Submit exactly that decision with `act`, then inspect the next boundary. For a rejection, give precise, text-grounded issues. For a question, provide all required answers.
4. Continue until the predeclared endpoint. Start a new chapter only when the case requires it; use `start_next_chapter` with deliberate chapter guidance.

Use `inspect --run <id>` when the outcome needs explanation: it exposes the immutable prompt/context snapshots and full agent message ledger for that run.

## Comparing and deciding

After the primary and regression pairs reach comparable endpoints, run:

```sh
target/debug/nekowriter-prompt-lab compare <baseline> <candidate>
```

Treat that output as a factual index, not a quality score. Compare the actual prose against the stated quality question: causality, character motivation, scene progression, style, constraint adherence, and any target-specific criteria. Also check that the regression output did not materially worsen.

Accept the candidate only when both are true:

- the primary case has a clear, text-backed improvement; and
- the regression case has no material degradation.

If accepted, copy only the verified candidate prompt files back to `crates/nekowriter-agent/prompts/`, retain the required template variables, and rerun validation. Do not change unrelated prompts, model settings, workflow code, or production data to make an experiment look better.

If the evidence is ambiguous or the regression is worse, do not write the candidate back. Report the observed trade-off instead.

## End of a round

Finish after one candidate decision. Report to the user:

- target prompt and quality hypothesis;
- baseline/candidate report paths and exact text evidence;
- regression result;
- accepted or rejected decision and files changed, if any.

Do not start an additional optimization round until the user asks.
