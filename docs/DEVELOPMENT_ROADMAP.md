# Sonne — Development Roadmap

> Status: proposed sequencing based on the audited MVP and the approved product direction.  
> A plan is complete only when its stated acceptance criteria are met, not merely when code exists.

## Delivery principles

- Do not start an implementation plan without a bounded objective, source inspection, and acceptance criteria.
- Do not install dependencies, change models, or alter security-sensitive configuration without an explicit decision for that plan.
- Preserve working chat behavior while replacing internals incrementally.
- Keep macOS and Windows implications visible from the first plan; Windows validation must occur before claiming cross-platform support.
- Treat research and decision records as work, especially for image generation, search, storage, and sync.

## Dependency overview

```text
Repository baseline and test strategy
        │
        ├── AI core and router stabilization ──┐
        │                                      ├── Chat persistence ──┬── Shared memory
        │                                      │                       ├── Backup/import-export
        │                                      │                       └── Projects / Diff / rollback
        │                                      │
        │                                      ├── Settings and permission foundation
        │                                      └── Image-engine research and integration
        │
        └── Search/tool policy ────────────────┴── Code Workspace tools

Optional internet sync comes after durable local data, backup/import-export,
and a defined conflict/security model.
```

## Plan 1 — Repository baseline and quality gates

**Purpose:** Establish a reliable change baseline before product work expands.

**Depends on:** Plan 0 documentation.

**Scope:** Decide repository initialization/version-control workflow if absent; replace template identity only if separately approved; add proportionate test/lint/build strategy and clear development commands.

**Done when:**

- Repository history and review workflow are available.
- A documented, repeatable validation baseline exists.
- Existing chat behavior remains verified.
- No template-removal work is bundled with unrelated feature changes.

**Parallel work:** UI inventory, prompt-evaluation examples, and Windows environment preparation can be researched without changing runtime architecture.

## Plan 2 — AI core, model configuration, and router stabilization

**Purpose:** Make model behavior deliberate before chat data becomes durable.

**Depends on:** Plan 1 quality gates for safe validation.

**Scope:** Finalize model instruction boundaries; separate model transport, prompts, router policy, and UI; define model aliases; add health, cancellation, errors, stream robustness, and routing evaluation cases.

**Must decide first:**

- Final Pro AI and Gemma instruction designs.
- Which instruction belongs in a Modelfile versus request-level context.
- Router signals, follow-up retention behavior, manual override semantics, and hand-off limits.
- Whether Ollama access remains frontend-direct temporarily or moves behind a Rust-mediated local service contract.

**Done when:**

- Manual selection takes precedence and is observable.
- Router decisions are explainable and tested against an agreed evaluation set.
- Generic programming tasks are not forced into an HTML/landing-page format.
- Stream termination, malformed chunks, cancellation, and unavailable Ollama states are handled predictably.
- Model/prompt changes are tested without silently rebuilding or overwriting user models.

## Plan 3 — Durable chat, local storage, and archive foundation

**Purpose:** Preserve conversations safely across restarts.

**Depends on:** Plan 2's stable conversation/message contract.

**Scope:** Choose local storage, schema, migrations, chat metadata, folders/projects/pins/archive, configurable 30-day auto-archive, text search, import/export, and backup foundations.

**Done when:**

- Restart restores conversations and metadata accurately.
- Archive, restore, pin, and folder/project organization are explicit and testable.
- Archived chats remain searchable and available for controlled context use.
- Import/export and local backup behavior have documented data-loss and compatibility boundaries.
- Storage does not require sync to function.

## Plan 4 — Settings, permissions, and safety foundation

**Purpose:** Establish the native trust boundary before local project tools gain power.

**Depends on:** Plan 3 storage for durable preferences and audit-relevant records.

**Scope:** Basic/Advanced settings, model/resource settings, permission records, approved directory scopes, secret-file policy, and Rust-enforced consent boundaries.

**Done when:**

- Settings persist locally and are recoverable.
- Permission scopes are enforced by native code, not by AI instructions.
- The UI clearly distinguishes read, staged-change, write, command, download, and data-transmission actions.
- Access to `.env`, credentials, tokens, and other sensitive paths follows an explicit policy.

## Plan 5 — Shared memory and context management

**Purpose:** Give both models useful, controlled continuity without dumping entire chat histories into prompts.

**Depends on:** Plans 2–4.

**Scope:** Memory schema, extraction criteria, confidence/confirmation rules, retrieval, editing/deletion UX, deletion tombstones or equivalent protection, and context budget rules.

**Done when:**

- Both models receive only relevant approved memory facts.
- Ambiguous important facts require confirmation.
- Manual deletion prevents automatic resurrection from chat history.
- Memory retrieval is inspectable and can be disabled or edited by the user.

## Plan 6 — Code Workspace, controlled edits, and Diff

**Purpose:** Turn Sonne into a trustworthy local development workspace.

**Depends on:** Plans 3–5, especially storage and native permissions.

**Scope:** Multiple approved projects, relevant-file reading, structure/dependency analysis, external-change detection, change plans, staged patches, Diff review, snapshots, rollback, and command execution policy.

**Done when:**

- No project directory is accessed without approval.
- The app can explain the proposed files and line-level changes before application.
- Users can accept/reject a staged change and restore a defined snapshot.
- Build/lint/test execution follows command-risk rules and reports actual results.
- Unrelated code is not rewritten as a side effect of a task.

## Plan 7 — Internet search and tool policy

**Purpose:** Add useful web-assisted work without weakening privacy or instruction safety.

**Depends on:** Plan 4; Code Workspace tool integration additionally depends on Plan 6.

**Scope:** Search triggers, source selection and reliability cues, web-content isolation, prompt-injection handling, download approvals, and external-data minimization.

**Done when:**

- The system can state why it searched and which sources informed a result.
- Web content cannot authorize tool use, data sharing, or permissions.
- Private files, full chat history, secrets, and tokens are excluded from external requests by design.
- Downloading remains approval-gated.

## Plan 8 — Image engine research and local creative workflow

**Purpose:** Select and integrate a maintainable local image workflow for both hardware profiles.

**Depends on:** Plan 4 for settings/resource controls; architecture research can begin earlier.

**Scope:** Compare local image engines/models, define Mac and RTX 4060 workflows, job lifecycle, progress/cancellation, asset storage/library, Basic/Advanced controls, editing, variations, background removal, and upscaling.

**Done when:**

- The chosen engine is supported by evidence from both target platforms or the documented platform boundary is explicit.
- Generated assets are stored locally and represented in the UI with job states.
- Cancellation and errors are recoverable.
- No cloud engine is used implicitly.

## Plan 9 — Cross-platform validation and optimization

**Purpose:** Validate the accumulated application on macOS and Windows rather than treating Tauri compilation as platform support.

**Depends on:** Each implemented module.

**Scope:** Native packaging, permissions, Ollama connectivity, model availability, image workflow behavior, storage locations, window/UI behavior, and performance on the known machines.

**Done when:**

- A documented validation matrix covers macOS and Windows.
- Platform-specific constraints and fallback behavior are explicit.
- A feature is not called cross-platform until it has been verified on both targets.

## Plan 10 — Local backup, portability, and optional sync research

**Purpose:** Make data movable and recoverable before any internet-based synchronization.

**Depends on:** Plans 3, 4, and 5.

**Scope:** Automatic local backup, integrity checks, portable import/export, conflict scenarios, device identity, encryption, and optional free/reliable sync research.

**Done when:**

- Local backup and Mac/Windows import/export work independently of a remote service.
- Data loss, conflict, and restore semantics are documented and tested.
- Any sync proposal has an explicit security, cost, conflict, and offline model.

## Work that can proceed in parallel

| Parallel track | Constraint |
|---|---|
| Model/prompt evaluation corpus | Does not rebuild or overwrite installed models without approval. |
| Image-engine research | Does not select or install an engine until decision criteria are reviewed. |
| UX flows and information architecture | Does not claim persistence/workspace actions that do not exist. |
| Windows environment preparation | Does not claim verified Windows support before tests run there. |
| Documentation and task templates | Must be kept consistent with code and approved decisions. |

## Deferred until prerequisites exist

- Cross-device internet sync is not a prerequisite for durable local chat.
- Semantic memory/search is not a substitute for a local data model.
- Autonomous file edits are not a substitute for scoped permissions and Diff review.
- Cloud services remain optional alternatives, not hidden dependencies.
