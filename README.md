# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Local development and validation

Sonne uses local Ollama for AI inference and Tauri for the desktop runtime.

### Requirements

- Node.js: `24.12.0` or newer (the built-in TypeScript type stripping used by the test runner is stable from Node 24.12.0).
- npm (ships with Node.js).
- Rust stable toolchain and the platform prerequisites required by Tauri 2.
- Ollama and the local models configured for this installation.

### Commands

- `npm run dev` — start the Vite development server.
- `npm run test` — run the pure context, model-routing, router, Ollama-lifecycle, and prompt-metric tests with Node's built-in test runner.
- `npm run build` — run TypeScript checks and create the frontend production build.
- `npm run check:rust:fmt` — verify Rust formatting without changing files.
- `npm run check:rust` — run Rust tests and compile checks.
- `npm run check` — run the complete validation sequence above.

The test runner does not replace TypeScript type-checking; `npm run build` performs that check. The Rust checks require the native development toolchain for the current platform.
