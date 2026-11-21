# Sonne — Master Specification

> Status: planning baseline, prepared from a repository audit on 2026-11-22.  
> This document separates product intent from what the repository implements today. It is not a claim that planned capabilities already exist.

## 1. Product definition

Sonne is a personal, local-first AI desktop workspace for macOS and Windows. It is intended to combine local language models, chat, software-project work, creative tools, controlled file changes, and eventually optional web-assisted workflows in one privacy-conscious desktop application.

Sonne is not being designed as a multi-tenant SaaS product. It must remain useful offline for its core workflows and must not require a paid cloud AI API, a subscription, or a permanent third-party server.

### Product principles

- **Local first.** Core AI inference, data storage, and creative workflows run locally where technically feasible.
- **User control.** Model choice, data, file access, proposed changes, and external communication remain visible and controllable.
- **Privacy by architecture.** Private content is not sent to external services merely because a tool is available.
- **Practical intelligence.** Choose the lightest capable model and avoid expensive analysis for simple work.
- **Trustworthy changes.** File modification must be reviewable through a clear Diff and explicit approval where risk warrants it.
- **Cross-platform intent.** macOS and Windows are first-class target platforms, while hardware-aware behavior may differ.
- **Incremental delivery.** Build stable foundations before high-risk capabilities such as broad filesystem access, web tools, image engines, or synchronization.

## 2. Target operating context

| Context | Known target |
|---|---|
| Primary development machine | MacBook Pro M5 Pro, 24 GB unified memory, macOS |
| Future Windows machine | Ryzen 5 5600, 32 GB RAM, RTX 4060 with 8 GB VRAM |
| Application shell | Tauri v2 desktop application |
| Core local model service | Ollama |
| Main models | Pro AI based on `qwen3:30b-a3b`; Gemma based on `gemma3:12b` |

The two machines need not provide identical generation speed, model capacity, or image workflows. Functional parity is the goal; identical local performance is not an assumption.

## 3. Sonne Voice, model roles, and target routing behavior

### Sonne Voice (agreed behavior)

Pro AI and Gemma share one conversational identity. This describes the intended user experience; it does not replace each model's separate working instructions.

- Be friendly, intelligent, creative, and professional.
- Communicate naturally, without rigid response templates.
- Respond in the user's language.
- Adapt response length: concise for simple requests and appropriately detailed for complex work.
- Offer honest opinions, including reasoned disagreement when useful.
- Avoid unnecessary introductions, repetition, excessive praise, sycophancy, and theatrical apologies.
- When an error is identified, acknowledge it briefly, investigate the actual cause, and correct it.

### Pro AI

Pro AI is the strong local model based on `qwen3:30b-a3b`. Its agreed responsibilities are:

- Complex programming and engineering.
- Multi-step tasks and architectural analysis.
- Advanced UI/UX and design work.
- Complex debugging and code modifications.
- Deep research and tasks requiring broader reasoning.

### Gemma

Gemma is the fast local model based on `gemma3:12b`. Its agreed responsibilities are:

- Everyday conversations and translations.
- Short explanations and idea generation.
- Simple calculations.
- Small HTML/CSS components and JavaScript functions.
- Simple SQL queries and code explanations.
- Minor code modifications and information lookup.

Gemma should delegate a task to Pro AI when the work exceeds Gemma's practical capabilities.

### Model Router (agreed target behavior)

- Automatic routing is the default; a manual model choice overrides it.
- Routing considers the current conversation, project context, task complexity, and previous decisions rather than keywords alone.
- Short follow-up messages in a complex conversation retain the context of the ongoing task and must not trigger an unjustified switch.
- Both models can transfer tasks to each other when appropriate.
- The receiving model can reject or return an unsuitable task.
- A transfer preserves relevant conversation history, project decisions, previous results, and constraints.
- Transfer limits and loop detection prevent endless delegation cycles.
- The interface clearly indicates a switch, for example: `Switched to Pro AI`.
- Routing remains lightweight and fast.
- Shared long-term memory supplies relevant approved facts rather than an indiscriminate copy of every chat.

The current application does not yet implement these router requirements in full. Its present router is keyword- and model-name-based; see the current-status table and `ARCHITECTURE.md` for the implemented behavior.

## 4. Functional modules and current status

| Module | Target outcome | Current repository status |
|---|---|---|
| AI Core | Unified local model client, streaming, context, errors, interruptions | **Partial.** Browser-side Ollama calls and streaming exist; no dedicated service layer, cancellation, timeout policy, or context management. |
| Sonne Voice & Router | Stable voice, complexity-aware routing, controlled hand-offs | **Partial.** Manual override and keyword router exist; no complexity/history/project-aware decisions or hand-off logic. |
| Chat & History | Persistent chats, folders, projects, pins, archive, search, import/export | **Missing.** Messages are React in-memory state only. |
| Shared Memory | User-controlled, relevant, durable memory shared by models | **Missing.** |
| Internet Search & Tools | Deliberate web research with source assessment and prompt-injection defenses | **Missing.** |
| Code Workspace | Approved project directories, analysis, staged edits, Diff, rollback, snapshots | **Missing.** No filesystem integration is present. |
| Image & Creative Tools | Local image generation/editing, library, Basic/Advanced workflows | **Placeholder only.** Image intent is recognized but no engine runs. |
| Settings, Permissions & Security | Basic/Advanced settings and Rust-enforced access policy | **Partial.** Tauri capabilities exist, but no product settings or scoped file permission system. |
| Backup & Sync | Local backups, portable import/export; optional future sync | **Missing.** |
| Cross-platform validation | Supported macOS and Windows desktop behavior | **Partial.** The project is Tauri-based; Windows has not been verified. |

## 5. Actual current baseline

The audited implementation is a small Tauri + React chat MVP:

- React 19 and TypeScript form the visible application.
- Prism provides syntax highlighting for detected fenced code blocks.
- React calls Ollama directly at `http://127.0.0.1:11434` for model discovery and chat completion.
- Responses are received as a newline-delimited stream and rendered in the chat.
- The UI has Auto/Chat/Code/Image modes, a manual model selector, a thinking toggle, copy controls, and an HTML/CSS/SVG preview iframe.
- The Rust layer currently initializes Tauri and the opener plugin; it does not implement model, storage, filesystem, or permission business logic.
- The custom Pro AI Modelfile exists and derives from `qwen3:30b-a3b`.

The repository does not use TailwindCSS despite earlier project-context wording. Styling is custom CSS in `src/App.css`.

### Verification notes

- The previous audit on 2026-11-21 recorded successful `npm run build` and `cargo check`; `cargo test` completed with zero tests.
- This documentation-only audit re-inspected the present source, dependencies, scripts, configuration, and repository structure. It intentionally did not rerun build/test commands because this plan permits only documentation edits.
- No source-level difference from the previous audited baseline was observed. There is no Git repository in the working directory, so historical change verification is unavailable.

## 6. Product requirements by module

### A. AI Core

- Support local Ollama model discovery and generation.
- Stream responses with clear generation, failure, cancellation, and retry states.
- Construct bounded context deliberately; do not send full history by default.
- Keep model prompts, routing policy, and transport logic independently testable.

### B. Chat and history

- Persist conversations and metadata locally.
- Support folders, projects, pins, archive, restoration, import, export, text search, and later semantic retrieval.
- Default inactive-chat archival after 30 days must be configurable.
- Archived chats remain searchable and recoverable.

### C. Shared memory

- Maintain one long-term memory source used by both models.
- Save useful durable facts automatically only when confidence is sufficient; request confirmation for important ambiguous changes.
- Allow manual creation, editing, and deletion through Settings.
- A deleted memory must not silently return from historical chat extraction.

### D. Internet search and tools

- Search only when it materially helps.
- Prefer multiple sources for consequential facts and expose concise source-quality signals.
- Treat web content as untrusted instruction input.
- Never send secrets, private files, full chat history, or access tokens to external services without explicit user intent.
- Downloads require approval.

### E. Code Workspace

- Keep Projects separate from chat navigation.
- Restrict access to directories the user approved.
- Read project structure, README, AGENTS.md, and relevant configuration before proposing changes.
- Produce minimal scoped edits; show a Diff before applying consequential changes.
- Provide rejection, rollback, local snapshots, external-change detection, and validation through build/lint/test when available.
- Define safe versus approval-required commands in the native backend, not only in a prompt.

### F. Image and creative tools

- Use a separately bounded local image engine.
- Support text-to-image, editing, variations, background removal, upscaling, and a local library.
- Offer Basic and Advanced modes.
- Research Mac and Windows workflows before choosing models or engines; quality is more important than raw speed.
- A cloud image option, if ever added, remains optional and requires explicit confirmation.

### G. Settings, permissions, backup, and sync

- Provide Basic and Advanced settings for models, resources, chats, memory, image generation, and permissions.
- Enforce access policy in Rust.
- Keep local storage primary; provide local backups and cross-platform import/export.
- Treat internet synchronization as optional and non-blocking. It must not require a paid service by default.

## 7. Boundaries and non-goals

- No mandatory cloud AI, paid API, shared drive, or always-on remote server.
- No broad filesystem scanning without a user-approved directory and a task need.
- No automatic destructive file operation, dangerous command, or private-data transmission.
- No claim that image generation, search, shared memory, code editing, or sync exists before implementation and verification.
- No full rewrite without a justified, reviewed decision.

## 8. Open decisions

The following are deliberately unresolved and require research or an explicit decision before implementation:

1. Local storage technology, schema, migrations, encryption-at-rest policy, and backup format.
2. Final shared-memory extraction, ranking, confirmation, deletion, and retrieval rules.
3. Model prompt boundaries, final Modelfiles, model aliases, context-window policy, and routing evaluation set.
4. The local image engine(s), model(s), and hardware-specific Mac/Windows workflows.
5. Search/tool provider boundaries, consent UX, source-quality policy, and prompt-injection containment.
6. Code Workspace snapshot/rollback format and command-risk classification.
7. Optional sync architecture, conflict policy, device authentication, and end-to-end encryption model.

## 9. Source of truth hierarchy

When the documents, a task description, and code disagree, use this order:

1. Explicit current user decision.
2. Current repository code and configuration for present implementation facts.
3. This specification for accepted product direction.
4. Roadmap and task-specific acceptance criteria.

Any conflict or ambiguity must be reported instead of silently resolved through assumptions.
