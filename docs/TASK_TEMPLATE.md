# Sonne — Task Template

Use this template for a future development, audit, research, or design task. Remove sections that do not apply, but preserve the distinction between current facts, requested work, and unverified assumptions.

```md
# [PLAN / TASK ID] — [Short title]

## 1. Context

- Product area: [AI Core / Chat / Memory / Projects / Search / Images / Settings / Backup / Platform]
- Current repository facts: [Only facts verified from current code/configuration]
- Related documents: [Links or paths]
- Target platforms: [macOS / Windows / both]
- Relevant hardware constraints: [Only when needed]

## 2. Goal

[One concrete user-visible or technical outcome.]

## 3. In scope

- [Required behavior or deliverable]
- [Required behavior or deliverable]

## 4. Out of scope

- [Explicitly excluded work]
- [Feature intentionally deferred]

## 5. Requirements

### Functional

- [Observable requirement]
- [Observable requirement]

### Data and persistence

- [What is stored, where, retention, migration/compatibility expectations]
- [What must not be stored or transmitted]

### Security and permissions

- [Required approval boundary or native enforcement]
- [Secrets/private data rules]

### UX

- [Loading, empty, error, cancellation, confirmation, accessibility states]

### Cross-platform

- [macOS/Windows behavior or explicit non-support]

## 6. Constraints

- Do not [specific prohibited action].
- Preserve [existing behavior/files].
- Do not add dependencies without explicit approval.
- Use [approved local service/interface] only.

## 7. Decisions already made

- [Decision, source, and date]

## 8. Open questions requiring a decision

- [Question and why it blocks or changes scope]

## 9. Expected implementation approach

1. Inspect [relevant files/services].
2. Propose or validate [architecture/data/permission boundary].
3. Implement [bounded slice].
4. Add/update [tests, documentation, migrations, UI states].
5. Verify [specific commands/manual scenarios].

## 10. Files and interfaces expected to change

| Path or interface | Expected reason | Confirmed after inspection? |
|---|---|---|
| `[path]` | [reason] | [yes/no] |

## 11. Acceptance criteria

- [ ] [User-observable completed behavior]
- [ ] [Failure/edge case behavior]
- [ ] [Permission/privacy behavior]
- [ ] [Persistence/restart behavior, if applicable]
- [ ] [Platform validation, if applicable]
- [ ] [Relevant test/build/manual verification passes]

## 12. Verification plan

| Check | Why | Expected evidence |
|---|---|---|
| [unit/static check] | [risk addressed] | [result] |
| [integration/manual test] | [risk addressed] | [result] |
| [platform check] | [risk addressed] | [result] |

## 13. Risks and rollback

- Risk: [risk]. Mitigation: [mitigation].
- Rollback/recovery: [how changes/data can be restored].

## 14. Final report required

- Outcome.
- Files changed and rationale.
- Verification results.
- Known limitations and skipped checks.
- Follow-up work or decisions still needed.
```

## Usage notes

- An audit task must explicitly say **no implementation** and list any allowed documentation outputs.
- A model task must separately identify Modelfile changes, request-level prompt changes, router changes, and model rebuild authorization.
- A persistence task must define migration, backup, restart, deletion, import/export, and recovery behavior.
- A file-workspace task must list permitted directories, approval boundaries, Diff/rollback requirements, and command-risk rules.
- A search or image task must specify which external/local engines are authorized and whether any network or download action needs confirmation.
