# OpenCode skills: TDD test-writer + delegation router

Two skills, one subagent, and three commands for OpenCode, implementing the workflow we agreed:

- **tdd-test-writer** (modified): spec → approved case list → small batch cycles. The agent authors 2–4 failing behavior tests per batch, verifies RED, hands off; you implement; the agent verifies GREEN, triages test-vs-spec disputes against the spec, updates the feature's Obsidian note, and continues. Your original rules are kept (never touch production code, verified RED for the right reason, no weakening tests, Obsidian MCP notes with no code in them). What changed from the old strict one-test-at-a-time: the **case list is written and approved first** (prevents drift and contamination), and tests are authored in **batches of 2–4** (each test still individually verified RED).
- **delegation-router** (new): before a task starts, scores it on specifiability × verifiability and recommends a lane — you implement (agent as sounding board), agent implements with a verification gate, or the hybrid (agent writes tests, you implement the core, agent does the boring edge).
- **tdd-test-writer subagent**: a `mode: subagent` OpenCode agent the primary session uses for RED-phase authoring, with a defined report format back.
- **Commands**: `/tdd`, `/tdd-green`, `/delegate`.

## File layout

```text
opencode-skills/
├── skills/
│   ├── tdd-test-writer/SKILL.md      # modified TDD workflow skill
│   └── delegation-router/SKILL.md    # new: delegate vs do-it-yourself decision skill
├── agents/
│   └── tdd-test-writer.md            # subagent for RED-phase test authoring
├── commands/
│   ├── tdd.md                        # /tdd <feature>      — start or resume the workflow
│   ├── tdd-green.md                  # /tdd-green          — implementation ready, verify GREEN
│   └── delegate.md                   # /delegate <task>    — get a delegation decision
└── opencode.example.json             # reference MCP config for the Obsidian MCP server
```

## Install

Copy the pieces into your project (or into `~/.config/opencode/` for global availability):

| What | Project (recommended) | Global |
| --- | --- | --- |
| Skills | `.opencode/skills/<skill-id>/SKILL.md` | `~/.config/opencode/skills/<skill-id>/SKILL.md` |
| Subagent | `.opencode/agents/tdd-test-writer.md` | `~/.config/opencode/agents/tdd-test-writer.md` |
| Commands | `.opencode/commands/<name>.md` | `~/.config/opencode/commands/<name>.md` |

For example, from this folder:

```bash
mkdir -p .opencode
cp -r skills/tdd-test-writer skills/delegation-router .opencode/skills/
cp -r agents .opencode/
cp -r commands .opencode/
```

OpenCode also discovers compatibility locations (`.claude/skills`, `.agents/skills`) and hot-reloads changes, so you can edit the skill mid-session. Full discovery table: https://opencode.ai/docs/skills

## Obsidian MCP

The skill expects an Obsidian MCP server already configured (as you have now) and calls it without an approval gate. Keep your existing entry; `opencode.example.json` is only a reference shape for `opencode.json`:

```json
{
  "mcp": {
    "servers": {
      "obsidian": {
        "type": "local",
        "command": ["npx", "-y", "mcp-obsidian"],
        "environment": { "OBSIDIAN_API_KEY": "{env:OBSIDIAN_API_KEY}" }
      }
    }
  }
}
```

(`mcp-obsidian` needs the Obsidian Local REST API plugin and an API key; adjust the entry to whichever Obsidian MCP server you use.) If the server is unavailable at runtime, the skill falls back to a code-free note draft in chat and says so — it never claims a note was saved.

## Day-to-day loop

1. **Spec first.** Use OpenCode's Plan mode to agree the plan/spec for the feature.
2. **`/tdd <feature>`** — the agent builds a one-line case list (simplest → hardest), invites your domain cases, waits for your approval, then authors the first batch of 2–4 tests, shows each failing RED for the right reason, and stops with an `TDD RED PHASE COMPLETE` handoff.
3. **You implement** that batch.
4. **`/tdd-green`** — the agent runs the handed-off tests, triages failures against the spec (quote spec line → "does the test contradict the spec?"), updates the Obsidian note, and continues with the next batch automatically.
5. **`/delegate <task>`** — for anything new, get an explicit keep/delegate/hybrid recommendation with a verification gate before starting.

## Customization knobs

- **Batch size**: 2–4 is the default; single pure functions may take the whole list in one batch (rule 5).
- **Case list location**: the approved checklist lives in the repo (checklist file or skipped/placeholder scaffold); the Obsidian note mirrors coverage status as metadata.
- **Subagent enforcement**: the tests-only boundary is enforced by instructions. If you want hard enforcement, add path-scoped `permissions` rules to the agent frontmatter (see https://opencode.ai/docs/agents) — verify the allow/deny precedence in your version before relying on it.
