# SONNE — MODEL CONFIGURATION ARCHITECTURE v1

## 1. Purpose

This document defines how Sonne configures, manages, and initializes its local AI models.

The architecture must support multiple models with different capabilities while maintaining one consistent Sonne identity and instruction system.

The configuration system must be modular, maintainable, predictable, and extensible.

## 2. Supported Models

Sonne initially supports two primary local models.

### 2.1 Pro AI

- Base model: `qwen3:30b-a3b`
- Model alias: `pro-ai`
- Role: Primary high-capability intelligence.
- Primary workloads: Complex reasoning, software engineering, analysis, research, planning, and demanding multi-step tasks.

### 2.2 Gemma

- Base model: `gemma3:12b`
- Model alias: `gemma`
- Role: Fast everyday intelligence.
- Primary workloads: General assistance, translations, explanations, writing, lightweight programming, and practical problem solving.

The aliases are logical application-level identifiers. They must not be assumed to be the actual Ollama model names.

## 3. Configuration Separation

Sonne must separate model configuration into distinct layers.

### 3.1 Base Model

The original Ollama model provides the underlying model architecture and pretrained capabilities.

Sonne must not modify the original base model.

### 3.2 Modelfile Configuration

Each Sonne model must have its own Modelfile.

The Modelfile is responsible for:

- Selecting the base model.
- Defining model-specific technical parameters.
- Applying stable model-level configuration.
- Providing a predictable Ollama model definition.

It must not contain the full contents of Sonne's instruction documents.

### 3.3 Runtime Instructions

Runtime Instructions are assembled by Sonne when preparing an AI request.

They contain the applicable instruction layers:

- Core System Rules.
- Sonne Voice.
- Model Intelligence Profile.
- Task-specific instructions.
- Relevant runtime constraints.

The Instruction Hierarchy determines how these layers are interpreted and prioritized.

Runtime instructions must be generated from the authoritative Markdown documents rather than maintained as manually duplicated copies.

### 3.4 Request Context

Request-specific information is provided separately from the stable model configuration.

It may include:

- Current user request.
- Relevant conversation history.
- Relevant authorized context.
- Current task information.
- Applicable tool or capability information.

Only context relevant to the current request should be included.

## 4. Configuration Architecture

The logical configuration flow is:

1. Sonne identifies the selected model.
2. Sonne loads that model's configuration.
3. Sonne resolves the applicable instruction documents.
4. Sonne assembles Runtime Instructions.
5. Sonne adds relevant conversation and request context.
6. Sonne prepares the Ollama request.
7. Ollama processes the request using the selected model.
8. Sonne handles the response.

The implementation must preserve the separation between model configuration, instruction assembly, request context, and response processing.

## 5. Model-Specific Configuration

Each model must have an independent technical configuration.

A model configuration should support:

- Logical model alias.
- Ollama model identifier.
- Model-specific temperature.
- Top-p sampling.
- Repeat penalty.
- Context window configuration.
- Maximum generation length.
- Optional model-specific parameters supported by the underlying runtime.

Not every parameter must be identical across models.

Values must be selected according to model behavior, available hardware, response quality, and reliability.

Do not assume that one configuration is optimal for both models.

## 6. Runtime Instruction Management

Runtime Instructions must be assembled dynamically.

The system must:

- Load the authoritative instruction documents.
- Apply the Instruction Hierarchy.
- Select the correct model-specific profile.
- Include relevant task instructions.
- Avoid unnecessary duplication.
- Preserve the user's current request and relevant context.

Instruction assembly must be deterministic and testable.

If a required instruction document is unavailable or cannot be loaded, the system must not silently pretend that the complete instruction set was applied.

## 7. Modelfile and Runtime Responsibilities

The Modelfile and Runtime Instructions serve different purposes.

### Modelfile

Responsible for stable model initialization and technical configuration.

### Runtime

Responsible for Sonne's active behavior, instruction hierarchy, task requirements, and request context.

The implementation must explicitly establish how Ollama combines or prioritizes Modelfile-defined system instructions and request-level messages.

Do not assume that a runtime system message automatically replaces or merges with the Modelfile SYSTEM instruction.

This behavior must be verified against the actual Ollama implementation and documentation before finalizing the request pipeline.

## 8. Model Aliases and Identification

Sonne must use stable logical aliases internally:

- `pro-ai`
- `gemma`

These aliases must be mapped to actual installed Ollama model identifiers.

The application must not rely on a hardcoded model identifier when a configurable mapping is more appropriate.

Model discovery must remain compatible with the installed models returned by Ollama.

If a configured model is unavailable, Sonne must handle that condition explicitly.

It must not silently select an unrelated model without communicating the change.

## 9. Model Availability and Initialization

The configuration system must account for:

- Ollama availability.
- Model installation status.
- Model identifier validity.
- Model initialization errors.
- Model loading delays.
- Model switching.

A model must be considered available only when the application has sufficient evidence that the requested model can be used.

Unavailable models must produce clear, actionable errors.

## 10. Configuration Validation

Before a model configuration is used, Sonne should validate:

- Required model identifier.
- Parameter types and supported ranges.
- Required instruction documents.
- Alias mapping.
- Runtime compatibility.

Invalid configuration must not silently produce an unrelated or misleading result.

Validation errors should identify the relevant configuration problem.

## 11. Model Updates and Reproducibility

Model configuration changes must be controlled.

Sonne should preserve the ability to identify:

- Which base model is used.
- Which Modelfile configuration is active.
- Which instruction document versions were loaded.
- Which runtime parameters were applied.

Updating instructions must not silently overwrite an installed model or change its configuration.

Model rebuilding must be an explicit operation.

## 12. Extensibility

The architecture should allow additional models in the future without redesigning the entire configuration system.

New models should be configurable through the same general structure while retaining their own:

- Model identifiers.
- Intelligence profiles.
- Technical parameters.
- Runtime requirements.
- Availability status.

The architecture must not depend on exactly two models.

## 13. Performance and Resource Awareness

Model configuration should account for the actual hardware environment.

Sonne must avoid assuming that all devices can run the same model configurations.

The system should consider:

- Available system memory.
- GPU capabilities.
- Model size.
- Context window requirements.
- Concurrent workload.
- Model loading and inference performance.

Performance optimizations must not silently compromise response correctness or instruction integrity.

## 14. Security and Local-First Principles

Model configuration must respect Sonne's offline-first and privacy-first architecture.

Local model execution must not require an external AI API.

Configuration documents and runtime context should remain local unless a separately authorized feature explicitly requires external communication.

The system must not expose sensitive local configuration data in ordinary model responses.

## 15. Architectural Boundaries

This document defines configuration architecture, not the complete implementation.

The following responsibilities remain separate:

- Instruction content: Final Instructions and base instruction documents.
- Instruction priority: Instruction Hierarchy.
- Model selection: AI Router.
- Configuration management: Model Configuration layer.
- Request execution: AI Request Pipeline.
- Response reliability: AI Request Pipeline and Evaluation.

Do not merge these responsibilities into one large application component.

## 16. Implementation Acceptance Criteria

This architecture is ready for implementation when:

- Pro AI and Gemma have independent configurations.
- Logical aliases map correctly to installed Ollama models.
- Modelfiles contain stable model-level configuration rather than duplicated instruction documents.
- Runtime Instructions can be assembled from authoritative documents.
- Model-specific parameters can be configured independently.
- Missing models and invalid configurations are handled explicitly.
- Model rebuilding is controlled and never happens silently.
- Ollama's system-message behavior is verified before request-pipeline integration.
- The architecture can support additional models later.

## 17. Final Principle

Sonne must manage its models as configurable components of one unified AI system.

Each model has its own technical configuration and intelligence profile, while sharing Sonne's identity, core rules, and instruction architecture.

Configuration must remain explicit, modular, reproducible, and independent from the user interface.