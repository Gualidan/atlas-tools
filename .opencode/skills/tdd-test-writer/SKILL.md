---
name: tdd-test-writer
description: >
  Spec-driven, test-first workflow: turn the agreed spec into a reviewed case
  list, then run batch cycles — author a small batch of failing,
  behavior-focused tests, verify each fails for the right reason, and stop so
  the user can implement. After the user reports ready, verify GREEN, update
  the feature's Obsidian note, and continue with the next batch. Use for new
  features, bug fixes, regression tests, and explicit TDD or red-green
  workflows where the user wants to write production code.
---

# TDD Test Writer

Use this skill to run a test-first workflow in which the agent authors tests and the user implements production behavior. The agent never writes production code. Work has two phases: first convert the agreed spec into an approved case list (the contract), then run batch cycles of author-RED, hand-off, GREEN. Wait at every hand-off boundary; the user, not the agent, decides when to proceed.

The case-list-first design exists to prevent the two known failure modes of incremental test authoring: drift (each round re-guesses the spec, so style and coverage become inconsistent) and contamination (once tests are written against partially written implementation code, they confirm what was built instead of checking what was agreed). The approved case list, not the agent's memory of the conversation, is the contract for the whole feature.

## When to use

Use this skill when the user asks for:

- Test-first development or a red-green-refactor workflow, typically after a spec or plan has been agreed.
- Tests that define a feature before the user implements it.
- Regression tests for a reported bug.
- Incremental feature development where the agent authors the next batch of tests only after the user completes the current implementation step.

## Required rules

1. **Never modify production code.** This skill may create or edit test files only. Do not implement the behavior, even to make a test pass.
2. Write behavior-focused tests, not placeholders or tests that merely mirror an implementation detail. Test what the code does, not how it is structured internally. Do not mock the unit under test.
3. Every test must be run and shown to fail before handoff. The failure must demonstrate the missing or incorrect behavior, not a syntax, setup, or unrelated environment problem.
4. **Case list before test code.** No test is authored until a case list has been produced from the spec, reviewed, and approved by the user. During review, explicitly invite the user to add domain-specific cases you cannot know from the spec; never invent domain rules yourself.
5. **Batch discipline.** Author tests in batches of 2–4 related cases from the approved list. Small enough that a failure is triageable, large enough to avoid a round-trip per assertion. Each test within a batch is individually verified RED before handoff. Exception: if the whole feature is a single pure function with a bounded edge-case set, the entire approved list may be authored as one batch.
6. **Independence.** Author tests from the spec and the approved case list only. You may read code to learn conventions, fixtures, and existing public interfaces, but do not derive the assertions for new behavior from the user's implementation or from an implementation sketch. Never read the user's in-progress implementation of the current slice to decide what a test should assert.
7. Do not weaken, delete, or rewrite a handed-off test to accommodate an implementation. If a test fails, triage first (see the GREEN step). Change a test only after establishing, against the spec and the approved case list, that the test itself is wrong — explain the reason and get the user's approval first.
8. Prefer deterministic, targeted test commands over full-suite runs. Avoid network or time randomness; use existing mocks and fixtures where appropriate.
9. For bug fixes, include a regression test that captures the reported failure mode.
10. Use the `tdd-test-writer` subagent (`.opencode/agents/tdd-test-writer.md`) for RED-phase test authoring when that role is available. Require tests-only edits and evidence of an expected RED result. If unavailable, author the test directly and state `FALLBACK: tdd-test-writer unavailable`.
11. Never claim a test was run or passed without observing its result.

## Workflow

### 1. Confirm the behavior contract

- Inspect the relevant code, public behavior, test layout, and existing test conventions. Do not assume APIs or test helpers that are not present. This reading is for conventions and interfaces only — behavior for new features comes from the spec.
- Convert the request into explicit acceptance criteria and confirm the spec is agreed with the user before any test work. If the user has an implementation idea, do not let it become the source of truth for tests; the spec is.
- Consider happy-path, edge, and negative-path behavior as relevant. For bug fixes, capture the reported failure.
- If a behavior-critical detail is unclear, state an `ASSUMPTION:` or ask a focused question rather than inventing a requirement.

### 2. Discover test conventions

- Identify the existing test framework, runner, naming style, fixtures, and the narrowest command that can run the relevant test.
- Reuse existing helpers and patterns. Avoid introducing new dependencies or parallel test infrastructure without a clear need.
- Identify the exact test file and intended observable behavior before authoring.

### 3. Build and get approval for the case list

- From the agreed spec, produce a case list: one line per behavior, written as behavior-level test names, ordered simplest → hardest. Cover happy-path, edge, and negative-path cases; include the regression case for bug fixes.
- Present the list and prompt the user: "Which domain cases do you know that are not on this list?" Add their cases; prune low-value ones the user flags.
- **Wait for explicit approval of the list.** Do not author test code before approval.
- Record the approved list as a checklist in the repo (a checklist file, or the whole suite scaffolded as skipped/placeholder tests matching the list, if the runner supports it) and mirror it into the feature's Obsidian note as test metadata.
- Later changes to the list are spec changes: propose them, get user approval, then update the checklist and the Obsidian note.

Case list format (checklist, kept in the repo):

```text
## Case list — <feature>
- [ ] rejects an empty input with a validation error
- [ ] trims surrounding whitespace before validating
- [ ] returns a sorted result for unordered input
```

### 4. Author one batch of RED tests

- Take the next 2–4 related unchecked cases from the approved list. One test per case; do not write tests for cases outside the batch.
- Add exactly one test to the relevant file; do not edit production files.
- Include a one-line intent comment above each test stating which approved case it pins down — this makes review fast.
- Use a regression case for a bug fix. Keep assertions specific enough to distinguish correct behavior from false positives.
- If a test needs setup fixes, modify only the test and rerun it. Do not mask a missing behavior with mocks or weaken the assertion.

### 5. Verify RED

- Run the narrowest relevant test command for each new test.
- Confirm each test fails for the expected missing or incorrect behavior.
- If a test passes immediately, investigate whether the behavior already exists or the test is ineffective. Do not hand off a test that has not demonstrated the expected RED state.
- If a test fails for an unrelated syntax, setup, or environment reason, fix only the test/setup issue within the tests-only boundary and rerun. If the expected failure cannot be established, report the blocker and stop.

### 6. Hand off the batch and wait for the user

Return the required `TDD RED PHASE COMPLETE` format below, listing every test in the batch with its intent, verification command, and failure summary. Then stop. Do not implement production code, author the next batch, or continue autonomously.

### 7. Verify GREEN after the user implements

When the user says their implementation is ready:

- Review the change only as needed to understand the implementation boundary; do not edit production code.
- Run all handed-off tests of the current batch and report the observed results.
- **On failure, triage before touching anything.** The failure means either the implementation is wrong or the test is wrong. Check the failing test against the spec and the approved case list, and use the tie-breaker with the user: quote the spec line and the failing assertion and ask, "Does this test contradict the spec, and if so how would you fix it?" Only if the test is demonstrably wrong per spec, propose the corrected test, get approval, update the checklist and Obsidian note, and rerun. If the test is right, describe the failure and wait for the user to make changes.
- If all tests in the batch pass, update the feature's Obsidian note with the GREEN result, check off the covered cases, then select the next batch from the remaining list, author and verify it RED, update the note, and hand it off. Repeat until the approved list is fully covered.
- Do not report the feature complete until every approved case is checked off or explicitly descoped by the user, the broader safety check passes, and the user has asked to finish or no additional behavior slices remain.

### 8. Completion check

- Every case on the approved list is covered (checked off) or explicitly descoped with the user.
- The broader safety check (related tests or full suite) passes.
- Report the final GREEN evidence and the final state of the Obsidian note.

## Required RED-phase output

Use this format after each new batch has been verified failing:

```text
TDD RED PHASE COMPLETE

## Authoring Mode
- Subagent: tdd-test-writer
- Fallback: [only if the subagent was unavailable]

## Case List Coverage
- Approved cases: [total]
- Covered so far: [count]
- This batch: [case names]

## Test Files
- [path] — [one-line intent per test, tied to its approved case]

## Verification
- Command: [exact command]
- Result: FAIL (expected)
- Failure reason per test: [1-2 lines tied to missing behavior]

## Implementation Contract (for the user)
1. Do not modify these tests: [paths]
2. Implement production changes only in: [paths or modules]
3. Completion gate: [exact command] passes without weakening the tests.
4. Broader safety check: [secondary command, if appropriate]
5. Return evidence: changed files and a concise summary of command output.

## Assumptions
- ASSUMPTION: [only if needed]
```

On a later GREEN checkpoint, report the exact verification command and observed result in chat, update the Obsidian note, then proceed with only the next batch as described above.

## Obsidian feature notes

Maintain a code-free Obsidian record of the test-driven work:

1. Create **one new Obsidian note for each new feature or bug-fix effort**. Do not reuse a note for a different feature.
2. When the case list is approved, record it in the feature's note as test metadata (case names only). Keep the working checklist in the repo; the note mirrors its status.
3. After each batch is added, expand that feature's existing note with a dated entry covering the batch: the behaviors covered, test names and file locations, RED results, and the implementation handoff status. Do not create a separate note for every test or batch.
4. After the user implements a slice and its tests pass, update the same note with the GREEN result before authoring the next batch.
5. Include implementation recommendations in plain-language prose: the intended behavior, relevant module or boundary, constraints, and a suggested approach at a conceptual level. Recommendations must not include source code or dictate an unverified implementation detail.
6. **Never put code in an Obsidian note.** Do not include source snippets, patches, pseudocode, or code blocks. Keep the note readable as prose; test names and file locations may be recorded as test metadata, not as executable examples.
7. Use the Obsidian MCP server when available and save the note without an approval gate. If it is unavailable, say so plainly and provide a code-free note draft in chat; do not claim that a note was saved or guess a vault path.

Suggested note sections: feature summary and acceptance criteria; assumptions; the approved case list with coverage status; dated batch entries with RED/GREEN status and failure/pass summaries; and implementation recommendations in prose. Keep the note focused on the current feature and expand it as the batch sequence advances.

## OpenCode setup notes

- This skill lives at `.opencode/skills/tdd-test-writer/SKILL.md` (project) or `~/.config/opencode/skills/tdd-test-writer/SKILL.md` (global). OpenCode loads it via the `skill` tool when a task matches the description.
- RED-phase authoring runs in the `tdd-test-writer` subagent (`.opencode/agents/tdd-test-writer.md`, `mode: subagent`). Invoke it from the primary session with: "Use the tdd-test-writer subagent to author the next batch from the approved case list."
- The Obsidian MCP server is configured in `opencode.json` under `mcp.servers` (see `opencode.example.json` for a reference entry). The skill expects it enabled and callable without an approval gate.
- `/tdd` starts or resumes the workflow; `/tdd-green` reports the implementation ready and triggers GREEN verification.

## Quality bar

- A spec was agreed and an approved case list exists before any test code.
- Each RED failure is reproducible locally and attributable to the behavior gap.
- Tests specify observable behavior, have meaningful discriminating assertions, and carry an intent line tied to an approved case.
- Batches are 2–4 related cases (or the full list only for a single pure function).
- Regression coverage captures the reported bug.
- Failures are triaged against the spec before any test is changed.
- Only one batch is handed off at a time; the user remains responsible for all production implementation.
- Obsidian notes are created per feature, updated after every batch addition and verified GREEN step, contain implementation recommendations in prose, and never contain code.
