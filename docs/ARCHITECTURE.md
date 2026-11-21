# Sonne — Architecture

> Status: audited current architecture plus a target-direction proposal.  
> Labels **Current** and **Target** are intentional; target sections are not implemented.

## 1. Current architecture at a glance

```text
Tauri desktop shell
├── Rust bootstrap (`src-tauri/src/main.rs`)
│   └── `sonne_lib::run()`
├── Rust application layer (`src-tauri/src/lib.rs`)
│   ├── initializes Tauri
│   ├── initializes `tauri-plugin-opener`
│   └── registers an unused template `greet` command
└── Webview frontend
    └── React application (`src/App.tsx`)
        ├── UI state and all chat logic
        ├── intent detection and model selection
        ├── direct HTTP requests to local Ollama
        ├── NDJSON response streaming
        ├── code parsing, Prism highlighting, and copy
        └── sandboxed iframe preview

Local service outside the application
└── Ollama at `http://127.0.0.1:11434`
    ├── `GET /api/tags`
    └── `POST /api/chat`
```

## 2. Repository map

| Path | Current responsibility |
|---|---|
| `src/App.tsx` | All visible app behavior: prompts, routing, model discovery, network transport, stream parsing, UI state, message rendering, image placeholder, preview state. |
| `src/App.css` | All custom visual styling. |
| `src/main.tsx` | React root bootstrap. |
| `src-tauri/src/main.rs` | Desktop executable entry point. |
| `src-tauri/src/lib.rs` | Tauri builder, opener plugin, unused `greet` command. |
| `src-tauri/tauri.conf.json` | Build, window, bundle, and security configuration. CSP is currently `null`. |
| `src-tauri/capabilities/default.json` | `core:default` and `opener:default` capabilities. |
| `Modelfile.pro-ai` | Custom Pro AI model definition based on `qwen3:30b-a3b`. |
| `package.json` | Vite/Tauri development, build, and preview scripts; no test or lint script. |

There is no `docs/` content before this Plan 0, no database/schema, no filesystem service, no test suite, and no Git metadata in the working directory.

## 3. Current AI request flow

```text
User enters prompt
       │
       ▼
`detectIntent(prompt, modeSetting)`
       │
       ├── image → append a local placeholder response; no model request
       └── chat or code
               │
               ▼
`pickModelForMode(resolvedMode, models, selectedModel)`
       │
       ▼
Build messages:
  current mode system prompt
  + every prior user/assistant message in RAM
  + current user message
       │
       ▼
Browser `fetch` → Ollama `/api/chat`
  `{ model, stream: true, think, messages }`
       │
       ▼
Read newline-delimited chunks → update final assistant placeholder
       │
       ▼
Parse fenced code → Prism code block or plain pre-wrapped text
```

### Routing behavior

- A manual available model has priority over automatic routing.
- In auto mode, prompts matching image keywords produce the image placeholder.
- In auto mode, code keywords route to a name containing `pro-ai`, then `coder` as fallback.
- Otherwise, chat routes to the first model name containing `gemma3`.
- If no expected model is available, selection falls back to the first loaded model or `pro-ai:latest`.

This is name- and keyword-based routing only. It does not inspect task complexity, the previous chosen model, project context, model health, latency, token budget, or hardware resources.

### Prompt layers

`pro-ai` has a model-level system instruction in `Modelfile.pro-ai`. For every non-image request, the frontend additionally sends either a chat-mode or code-mode system message. The application does not have a Gemma Modelfile or a model-specific Gemma system prompt.

The exact composition/precedence of Ollama's model-level system instructions and the request-level system message is not defined by this repository; it must not be assumed from frontend code alone.

## 4. Current presentation and rendering flow

- The frontend loads Prism language modules dynamically for markup, CSS, JavaScript, TypeScript, Java, C, and Python.
- A regular expression identifies fenced code blocks of the form ` ```language ... ``` `.
- Unfenced complete HTML documents are detected and presented as HTML code.
- Non-code text is rendered as plain text with whitespace preservation; there is no Markdown renderer.
- HTML, CSS, and SVG blocks can be previewed in an iframe using `srcDoc` and `sandbox="allow-scripts"`.
- The iframe sandbox separates the preview from the parent origin, but script execution remains enabled.

## 5. Current state ownership

| State | Owner | Durability |
|---|---|---|
| Messages, input, selected model, mode, thinking flag | React component state | Lost on reload/restart |
| Available model names | React state, loaded once from Ollama | Lost on reload; no manual refresh |
| Pro AI instructions and parameters | Local Modelfile / registered Ollama model | Outside application persistence |
| Application capabilities | Tauri config/capability JSON | Static configuration |

No application-owned data store currently exists.

## 6. Current security and platform boundaries

### Current facts

- Ollama is called from the frontend, not from Rust.
- The only declared Tauri plugin is opener; no filesystem or dialog plugin is configured.
- No local-project, file-read, file-write, command-execution, or diff capability exists.
- CSP is disabled with `"csp": null`.
- The preview executes model-supplied scripts in a sandboxed iframe.
- The product has not been tested on Windows.

### Implications to preserve during future design

- Browser/UI prompts must never be treated as the permission boundary for file, command, secret, or network access.
- The native backend must own future scopes and approval checks.
- Preview, search, downloaded content, and model output are untrusted input surfaces.
- Local-first does not itself prevent outbound network access; user-visible consent and enforceable boundaries remain necessary.

## 7. Current technical debt and limitations

1. `App.tsx` holds unrelated responsibilities in one component: transport, state, prompting, routing, parsing, and presentation.
2. There is no persistence, repository history, test suite, lint script, or CI evidence.
3. Stream parsing does not process a final buffered line without a trailing newline and does not isolate malformed JSON records.
4. A collected `thinking` value is rendered only while there is no normal content; it disappears once content is present.
5. The app stores `modelUsed` and `modeUsed` but does not show them in completed-message UI.
6. The code-mode application prompt mandates an HTML-oriented response format for all code-routed prompts, including generic Python, Java, SQL, API, bug, and error requests.
7. Markdown is not rendered despite model outputs potentially containing Markdown markers.
8. The template README, browser title, favicon, and Rust `greet` command remain.
9. CSS has no responsive or reduced-motion media rules despite the UI being a desktop product with a 800×600 configured initial window.

## 8. Target architecture direction

The following describes a staged direction, not approved implementation detail.

```text
Presentation layer (React)
├── Chat, Projects, Images, Settings views
├── Local UI state and rendering only
└── Explicit approval and review surfaces

Application services (typed frontend/Rust IPC contracts)
├── Conversation and context service
├── Router and model policy service
├── Memory retrieval and confirmation service
├── Project analysis / change-plan service
├── Search-tool policy service
└── Image-job service

Native trust boundary (Rust/Tauri)
├── Scoped file access and directory grants
├── Durable local storage, migrations, backups
├── Command classification and execution controls
├── Local-service health checks and IPC streaming
└── Audit-friendly snapshots, Diff, rollback, and permissions

External/local engines
├── Ollama
├── future local image engine(s)
└── optional web/search providers behind explicit policy
```

### Intended separation of responsibility

| Boundary | Intended responsibility |
|---|---|
| React | User interaction, view state, markdown/code rendering, review and consent UX. |
| Rust/Tauri | Durable data, security policy, filesystem and command scopes, local service management, platform-neutral IPC. |
| Ollama/model | Language generation only; no direct permission to touch user data or run tools. |
| Tool adapters | Narrow, typed capabilities with explicit inputs, outputs, scopes, and consent requirements. |

## 9. Architecture decisions that remain open

- Database/storage engine and migration approach.
- API contract between React and Rust for streaming and cancellation.
- Model context budgeting, token estimation, history summarization, and memory retrieval strategy.
- Image engine selection and job/storage protocol per platform.
- Search provider and web-content isolation model.
- Diff, snapshot, rollback, and external file-change detection format.
- Optional sync transport and conflict resolution.

No implementation should silently choose these on the basis of this document alone. Each should be made in a dedicated plan with alternatives, constraints, and acceptance criteria.
