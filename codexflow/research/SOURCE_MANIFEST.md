# CodexFlow Research Source Manifest

This directory tracks the two user-supplied research documents that define the Phase 3+ capability program.

## Source A

- Original filename: `Usecase for codeflow.md`
- SHA-256: `1e98b46fa5d97be9083e15b50f8d5e7d12c835d5a0a3f5be23ee301ca3f75632`
- Size observed in working environment: 54,943 bytes
- Rendered lines: 2,576
- Purpose: candidate open-source systems, concrete subsystems to mine, architectural recommendations, integration cautions, and a proposed next-generation local-model harness stack.
- Coverage artifact: `MASTER_60_CAPABILITY_MAP.md` uses these projects as candidate implementations/adapters.

Key project families covered include DeepSeek Harness, Oh My Pi, Qwen Code, Gemini CLI, Google Antigravity SDK, Codex, Serena, Headroom, LLMLingua, Repomix, Aider, Tree-sitter, ast-grep, multilspy, PydanticAI, ToolHive, XGrammar, vLLM/SGLang, RouteLLM, LiteLLM, Letta, Mem0, Graphiti, LangGraph, Deep Agents, Temporal, OPA, SWE-agent, OpenHands, Cline, Roo Code, Goose, Continue, Playwright, Browser Harness, Browser Use, Microsandbox, gVisor, Firecracker, E2B, Langfuse, Phoenix, Inspect AI and Promptfoo.

## Source B

- Original filename: `Codeflow Researach.md`
- SHA-256: `ee58e537e8aed21367b15cb8c04ba80ef2ed8050e970cac7db792f01d8f50f4c`
- Size observed in working environment: 44,475 bytes
- Rendered lines: 1,763
- Purpose: the master 60-category harness capability checklist, with 562+ concrete sub-capabilities describing how to move executive function, memory, verification, policy, retrieval, recovery and environment understanding out of the model and into the harness.
- Coverage artifact: `MASTER_60_CAPABILITY_MAP.md` maps all 60 top-level categories to CodexFlow primitives and candidate upstreams.

## Top-level Source B coverage

1. Context engineering
2. Give the model a map, not the entire world
3. Excellent tool design
4. Dynamic tool loading
5. Programmatic tool orchestration
6. Constrain the model's action space
7. Replace open-ended agency with bounded workflows where possible
8. Explicit decomposition
9. Explicit acceptance criteria
10. Independent verification
11. Automatic repair loops
12. Backtracking
13. Maintain a last-known-good state
14. Persistent task state
15. Long-running session handoff
16. Specialist skills
17. Few-shot procedural examples
18. Specialized agents
19. Supervisor-worker architecture
20. Parallelism
21. Self-consistency and ensembles
22. Search over reasoning trajectories
23. Separate generator and verifier
24. Confidence and escalation
25. Model routing
26. Use non-LLM components aggressively
27. Good editing primitives
28. Search before read
29. Read before edit
30. Plan before expensive action
31. Tool-result transformation
32. Environment introspection
33. Deterministic bootstrapping
34. Environment standardization
35. Make invariants mechanically enforceable
36. Guardrails
37. Sandboxing
38. State persistence and durable execution
39. Human-in-the-loop at the correct places
40. Observation and tracing
41. Transcript replay
42. Harness evaluations
43. Optimize the harness per model
44. Adaptive inference budgets
45. Failure-aware routing
46. Anti-loop mechanisms
47. Anti-premature-termination mechanisms
48. Grounding and retrieval
49. Source quality control
50. Prompt hierarchy and modular instructions
51. Explicit priorities
52. Strong defaults
53. Semantic affordances
54. Reduce choice wherever choice has no value
55. Increase choice where exploration has value
56. Keep the workspace as external cognition
57. Make the environment legible to the model
58. Externalize metacognition
59. Externalize executive function
60. Harnesses should turn intelligence problems into engineering problems

## Archival status

The source hashes above are the byte-identity anchors for the uploaded documents. The current ChatGPT GitHub connector accepts UTF-8 content writes but does not accept conversation-file references as upload parameters, so this branch contains the complete derived mapping and immutable source identity manifest rather than claiming a byte-for-byte connector upload that did not occur.

When a local CodexFlow/vanilla CLI session is available, the one-time archival action is simply to copy the two original files into `codexflow/research/sources/` and verify these SHA-256 values. Until that copy occurs, do not mark the `raw_source_archived` gate as passed.

## Definition of covered

A research category is not `covered` because it appears in a design file. It becomes covered only when its selected primitive or adapter has implementation evidence, relevant security/license review, telemetry, and benchmark/deterministic acceptance evidence. The long-term tracker should therefore move each category through:

```text
mapped -> designed -> implemented -> validated -> benchmarked -> retained/reworked/rejected
```
