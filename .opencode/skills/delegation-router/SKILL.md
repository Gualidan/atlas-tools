---
name: delegation-router
description: >
  Use when the user asks whether a task should be delegated to the agent (or
  to a subagent) or done by the user themselves — for example "should I
  delegate this?", "should the AI write this, or me?", "agent or manual?".
  Scores the task on how precisely it can be specified and how cheaply the
  result can be verified, recommends one of four lanes, and sets the
  verification gate before any work starts.
---

# Delegation Router

Use this skill to decide, before work starts, who should hold the pen on a task: the user, the agent, or a hybrid. Produce a short, explicit recommendation with a verification gate — never silently start implementing.

## The decision rule

Delegation leverage on a task = **how precisely it can be specified × how cheaply the output can be verified**. Tasks high on both are safe to delegate; tasks low on either stay with the user (with the agent, at most, as a sounding board).

Simple rule that overrides everything else: **if the user cannot meaningfully judge the diff, do not ship it.**

## Step 1 — Classify the task

**Green — delegate implementation freely:**

- Boilerplate and plumbing: CRUD, DTOs, serializers, migrations, config, Dockerfiles, CI YAML.
- Mechanical refactors with an existing green test suite as the safety net: renames, extractions, library upgrades.
- Adapters and converters between two known interfaces; transformations with example input → output pairs.
- Well-defined algorithms: parsers, validators, formatters, small state machines.
- Tests written from an agreed spec (hand to the `tdd-test-writer` skill).
- Edge-case enumeration, codebase archaeology ("where is this called?"), explanations of unfamiliar code.
- Documentation written from existing code: docstrings, README, ADR drafts.
- One-off scripts, with review before running anything destructive.

**Yellow — delegate, but the user steers hard:**

- New feature implementation — only when decomposed into small slices from an agreed plan; messy as one big request.
- UI work — fast for scaffolding, but design judgment, accessibility, and visual detail need review of rendered output, not just code.
- Concurrency and async code — plausible-looking with subtle bugs; needs stress tests and careful diff reading.
- Niche or very new library APIs — signatures may be hallucinated; verify against real docs.
- Performance work — suggestions are cheap, but everything must be measured; profiling judgment stays with the user.

**Red — do not delegate implementation:**

- Architecture and data-model decisions — the agent is a sounding board ("critique this design"), never the decider.
- Security-critical logic: crypto, auth, secrets handling — verify against known-good implementations.
- Core domain logic where understanding is the deliverable — the user implements; pair with the `tdd-test-writer` skill so tests are authored independently first.
- Anything the user could not meaningfully review, including code in a domain they do not yet understand.

## Step 2 — Pick the lane

- **Lane A — User implements, agent supports.** For red-list tasks and any task where the user wants to learn or stay sharp. The agent may explain, critique, and enumerate options, and may author tests via `tdd-test-writer`; it writes no production code.
- **Lane B — Agent as sounding board only.** "Critique this design", "what am I missing", "compare these two approaches". No code changes; output is analysis.
- **Lane C — Agent implements with a verification gate.** For green-list tasks, and yellow-list tasks with extra checks. The gate is defined before work starts (Step 3).
- **Lane D — Hybrid (default for features).** Agree the spec → agent writes tests via `tdd-test-writer` → user implements the core domain logic → agent implements the boring edge of the same feature (adapters, boilerplate) under the same test suite.

## Step 3 — Set the verification gate before any work

No delegation without a gate. Write down, concretely:

1. The exact command that decides "done" (test, typecheck, lint, benchmark, or for UI: what gets reviewed rendered).
2. What the user will personally review (every diff for yellow/red; sampled diffs for mechanical green-list work).
3. The fallback if the gate fails twice (stop and reconsider the lane, not more retries).

## Step 4 — Route within OpenCode

- Spec and plan first: use OpenCode's Plan mode to agree the plan before any edits; switch to Build to execute. Never delegate what has not been specified.
- Read-only research (locating code, comparing options, summarizing many files) → a `mode: subagent` agent, so the primary session's context stays clean.
- Tests-only authoring → the `tdd-test-writer` subagent.
- Multi-file implementation → the primary (Build) session, in small slices.
- Delegate to a subagent by asking for it by name: "Use the tdd-test-writer subagent to ...".

## Tie-breakers

- Unsure → delegate one small slice first with a gate; escalate only if reviewing it was cheap.
- If reviewing the output would take longer than doing the task → Lane A.
- The user's goal is learning or staying sharp → Lane A with `tdd-test-writer` tests.
- The task cannot be specified yet → write the spec together first; never skip specification to save time.
- The user disagrees with the recommendation → their call; record the decision and proceed.

## Output format

```text
DELEGATION DECISION

Task: [one line]
Lane: A (you implement) | B (sounding board) | C (agent implements) | D (hybrid)
Why: [1-3 lines tied to specifiability x verifiability]
Verification gate: [exact command + what gets personally reviewed]
First step: [one concrete action]
```

Keep the recommendation short and act on it once the user confirms. Do not re-litigate a lane the user has already chosen.
