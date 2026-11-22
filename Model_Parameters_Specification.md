# SONNE — MODEL PARAMETERS SPECIFICATION v1

## 1. Purpose

This document defines the initial technical parameters for Sonne's two local AI models on the current macOS development environment.

The target hardware is:

- Device: MacBook Pro
- Processor: Apple M5 Pro
- Unified Memory: 24 GB
- Runtime: Ollama
- Operating System: macOS

This specification applies exclusively to the current Mac environment.

Windows hardware and configuration will be specified separately during the cross-platform implementation phase.

## 2. Configuration Principles

Model parameters must balance:

1. Response quality.
2. Reasoning capability.
3. Generation speed.
4. Memory consumption.
5. Stability during extended conversations.

The two models have different responsibilities and must not automatically share identical parameters.

All values in this document are initial configuration targets. Their final suitability must be confirmed through practical testing.

## 3. Pro AI — Qwen3:30b-a3b

### 3.1 Model Identity

- Logical alias: `pro-ai`
- Ollama model: `qwen3:30b-a3b`
- Role: Primary high-capability model.
- Target hardware: MacBook M5 Pro, 24 GB unified memory.

### 3.2 Initial Parameters

| Parameter | Value |
|---|---:|
| Temperature | 0.65 |
| Top P | 0.90 |
| Repeat penalty | 1.08 |
| Context window (`num_ctx`) | 12288 |
| Maximum output (`num_predict`) | 8192 |

These values preserve the existing Pro AI configuration as the starting baseline.

### 3.3 Configuration Rationale

Pro AI is responsible for demanding tasks that benefit from deeper reasoning and more extensive analysis.

Its configuration should prioritize:

- Consistent reasoning quality.
- Reliable software engineering.
- Detailed technical explanations.
- Complex problem solving.
- Longer responses when necessary.

The context window should provide enough space for relevant conversation history, instructions, and complex requests without unnecessarily increasing memory consumption.

The maximum output limit allows extended responses while preventing uncontrolled generation.

### 3.4 Reasoning Behavior

Qwen3 supports thinking and non-thinking response modes.

Reasoning behavior should be controlled through the appropriate model and runtime mechanisms rather than relying exclusively on temperature or other sampling parameters.

The application must preserve the ability to configure thinking behavior independently of the model's general sampling configuration.

## 4. Gemma — Gemma3:12b

### 4.1 Model Identity

- Logical alias: `gemma`
- Ollama model: `gemma3:12b`
- Role: Fast everyday model.
- Target hardware: MacBook M5 Pro, 24 GB unified memory.

### 4.2 Initial Parameters

| Parameter | Proposed value |
|---|---:|
| Temperature | 0.70 |
| Top P | 0.90 |
| Repeat penalty | 1.05 |
| Context window (`num_ctx`) | 8192 |
| Maximum output (`num_predict`) | 4096 |

### 4.3 Configuration Rationale

Gemma is optimized for frequent everyday interactions and lightweight technical work.

Its configuration should prioritize:

- Responsive generation.
- Natural communication.
- Accurate short and medium-length answers.
- Translation and text editing.
- Explanations and summaries.
- Lightweight programming.
- Practical problem solving.

A moderate context window is sufficient as the initial target for its primary responsibilities.

The output limit should support useful responses without encouraging unnecessarily long generations.

### 4.4 Behavior Requirements

Gemma must not become excessively restricted by its performance-oriented configuration.

It should still make meaningful attempts at moderately complex tasks and provide useful partial solutions when appropriate.

Its role as the faster model must not compromise accuracy or the shared Sonne identity.

## 5. Shared Hardware Constraints

Both models operate on the same physical device and share unified memory with macOS and other applications.

The configuration must account for:

- Model weight memory.
- Context and KV cache memory.
- Runtime overhead.
- Operating system memory usage.
- Other active applications.
- Model loading and unloading behavior.

Sonne must not assume that both models can remain loaded simultaneously without sufficient available memory.

Model switching and loading behavior should be handled by the application and Ollama.

## 6. Context Window Management

The configured context window defines the total context capacity available to the model.

It must accommodate:

- System instructions.
- Runtime task instructions.
- Relevant conversation history.
- Current user request.
- Model response generation.

The application should avoid sending unnecessary historical content.

Context management must preserve important information rather than blindly including the entire conversation.

The actual context limit must be respected when assembling requests.

## 7. Generation Limits

`num_predict` defines the maximum number of tokens the model may generate for a response.

It must not be interpreted as a guaranteed response length.

The application should allow responses to finish naturally when the task is complete.

Generation limits may be adjusted independently for different workloads in the future, provided the behavior is predictable and tested.

## 8. Parameter Independence

Pro AI and Gemma must maintain separate configurations.

Changing a parameter for one model must not silently change the configuration of the other.

The configuration system should allow individual adjustment of:

- Temperature.
- Top P.
- Repeat penalty.
- Context window.
- Maximum output.
- Additional supported model parameters.

## 9. Configuration Ownership

The application must distinguish between:

- Default model parameters.
- Installed Ollama model configuration.
- Runtime request options.
- User-adjustable settings, if introduced later.

The effective parameters used for a request must be explicit and predictable.

Runtime overrides must not silently modify the persistent Modelfile configuration.

## 10. Validation Requirements

The initial values must be evaluated through practical tests on the target Mac.

Testing should include:

- Model loading and initialization.
- Memory consumption.
- Response latency.
- Short and long responses.
- Extended conversations.
- Complex Pro AI requests.
- Everyday Gemma requests.
- Model switching.
- Stability under normal multitasking.

If a configuration causes excessive memory pressure or instability, adjust it based on observed behavior.

Do not assume that a theoretically supported context window is necessarily practical on this hardware.

## 11. Future Optimization

Future optimization may include:

- Dynamic context allocation.
- Workload-specific generation limits.
- Improved model switching.
- Memory-aware model loading.
- Configurable sampling profiles.
- Performance monitoring.

These improvements should be introduced only when the core configuration is stable.

## 12. Implementation Acceptance Criteria

The configuration specification is ready for implementation when:

- Pro AI has an independent parameter profile.
- Gemma has an independent parameter profile.
- Both profiles target the current Mac hardware.
- The initial values are documented.
- Model aliases are mapped to their Ollama identifiers.
- Runtime parameters can be applied without silently rebuilding models.
- The configuration can be validated through practical testing.
- Windows-specific settings remain outside this specification.

## 13. Final Principle

Pro AI and Gemma must use configurations appropriate to their individual responsibilities.

Pro AI prioritizes deeper reasoning and demanding tasks.

Gemma prioritizes responsive everyday assistance without sacrificing usefulness or accuracy.

Both models must operate reliably within the memory and performance constraints of the current Mac environment.