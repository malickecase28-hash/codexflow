**Actual usecase harness and features to use in codeflow harness**

The way I would approach it if the objective were to make local Qwen/DeepSeek/GLM-class models behave like much stronger agents. So this researach i did for this project and this is where we start and what we map. for everything you choose to do give me on paragrpagh telling me what your choosing and taking and why. You may proceed reading below....

I would **not** build a monolithic harness from scratch. I would build a relatively thin orchestration spine and aggressively steal proven subsystems from existing open-source projects.

After digging through the current 2026 ecosystem, there is enough open source now that you can cover nearly every item from the previous harness list.

One thing also changed from what you suspected: **Google Antigravity is no longer purely a closed-source inspiration target.** Google now has a public Antigravity Python SDK under Apache 2.0 and a public CLI repository. The SDK exposes hooks, policies, tools, triggers and agent configuration directly. ([GitHub][1])

# My short version

If I were constructing your system today, I would study or borrow most heavily from:

| Problem                                  | Project I would mine first               |
| ---------------------------------------- | ---------------------------------------- |
| Overall replaceable harness architecture | **DeepSeek Harness**                     |
| Weak-model coding interface              | **Oh My Pi**                             |
| Policy/hooks architecture                | **Gemini CLI + DeepSeek Harness**        |
| Semantic code understanding              | **Serena**                               |
| Automatic context reduction              | **Headroom**                             |
| Repo structural compression              | **Aider + Repomix**                      |
| AST search/editing                       | **ast-grep + Tree-sitter**               |
| LSP intelligence                         | **Serena / multilspy**                   |
| Tool progressive disclosure              | **OMP + PydanticAI + ToolHive**          |
| Structured generation                    | **XGrammar**                             |
| Local inference serving                  | **SGLang / vLLM**                        |
| Model-role routing                       | **OMP + RouteLLM**                       |
| Long-term agent memory                   | **Letta / Letta Code**                   |
| Fact/interaction memory                  | **Mem0**                                 |
| Temporal knowledge memory                | **Graphiti**                             |
| Durable workflows                        | **LangGraph or Temporal**                |
| Subagents                                | **Qwen Code / Gemini CLI / Deep Agents** |
| Sandboxing                               | **Microsandbox**                         |
| Stronger Linux sandbox                   | **gVisor**                               |
| Maximum isolation                        | **Firecracker**                          |
| Managed sandbox abstraction              | **E2B**                                  |
| Browser verification                     | **Playwright MCP/CLI**                   |
| Browser autonomy                         | **Browser Harness / Browser Use**        |
| MCP governance                           | **ToolHive**                             |
| Deterministic policies                   | **OPA**                                  |
| Tracing                                  | **Phoenix or Langfuse**                  |
| Agent evaluations                        | **Inspect AI**                           |
| Tool-interface research                  | **SWE-agent**                            |
| Coding harness regression evals          | **SWE-agent + Inspect**                  |
| Prompt/harness regression testing        | **Promptfoo**                            |
| Checkpointed state graph                 | **LangGraph**                            |
| Persistent agent identity                | **Letta**                                |
| Context compression research             | **LLMLingua**                            |
| Model-adaptive toolsets                  | **PydanticAI**                           |
| Parallel coding agents                   | **Qwen Code / Cline / Roo**              |
| Generator → editor separation            | **Aider Architect mode**                 |
| Permission reviewer pattern              | **Codex + Gemini CLI**                   |

But the interesting part is how these fit together.

---

# 1. DeepSeek Harness

## `deepseek-ai/deepseek-harness`

This might be the **most interesting architectural repository for your particular project**.

It is MIT licensed and explicitly designed around:

> Everything is a plugin.

Not just MCP-style optional extras. DeepSeek makes the model adapter, session log, tool registry and **agent loop itself replaceable components**. ([GitHub][2])

Its fundamental architecture is roughly:

```text
                    Cordis Context
                         │
     ┌───────────────────┼───────────────────┐
     │                   │                   │
    LLM               Session             Tools
   plugin              plugin             plugin
     │                   │                   │
     └────────────── Agent Loop ─────────────┘
                         │
         ┌───────────────┼─────────────────┐
         │               │                 │
     Compaction       Policies          Subagents
         │               │                 │
       Skills          Sandbox          Workflow
```

### Things I would directly steal

**Event-sourced sessions.**

A DeepSeek session is an append-only typed event log. The LLM conversation is *derived* from that log instead of being the canonical state. ([GitHub][3])

That is an extremely good design.

Instead of:

```text
messages = canonical truth
```

use:

```text
events = canonical truth

events
  ↓ projection
model-visible context
```

Now you can independently:

* compact
* replay
* hide
* replace
* summarize
* inspect
* audit
* reconstruct
* fork

without corrupting the original trajectory.

### Compaction is a capability seam

DeepSeek does not hard-wire its summarizer into the loop. The compactor is replaceable.

Its current design records:

* selected ranges
* token counts
* summaries
* shadowed events
* model calls
* compaction lifecycle

as durable events. ([GitHub][4])

That is *very* compatible with what you are already doing with Headroom.

### Even oversized tool results have a separate pruner

DeepSeek exposes a `toolResultPruner`, token meter and other explicit capability seams rather than treating all context reduction as one giant summarization problem. ([GitHub][5])

### Policies live around lifecycle events

Its core loop is intentionally minimal:

```text
call model
run tools
repeat
```

Things such as:

* permission policy
* compaction
* retries
* sandboxing
* plan mode
* persistence
* subagents

live outside it and intercept lifecycle events. ([GitHub][6])

This is exactly how I would architect your policy-heavy system.

### Model-written workflows

DeepSeek also has an interesting workflow seam where an agent can produce an orchestration script that launches subagents. ([GitHub][7])

That gives you:

```text
LLM decides workflow
        ↓
deterministic runtime executes workflow
        ↓
workers return bounded results
```

rather than forcing the parent model to babysit every call.

### Warning

It is explicitly labelled **developer preview** and says breaking changes should be expected. ([GitHub][2])

### My assessment

**Study deeply: 10/10**

**Use unchanged as production core today: 7/10**

**Mine architecture from it: 10/10**

For what you're building, I would absolutely read the DeepSeek Harness source.

---

# 2. Oh My Pi

## `can1357/oh-my-pi`

OMP is probably the repository I would study most carefully for:

> **How do I squeeze dramatically more coding ability out of the same weights?**

It is MIT licensed and has deliberately optimized its agent-computer interface. ([GitHub][8])

The project claims dramatic harness-only differences for some models, including one edit benchmark increasing from 6.7% to 68.3% after changing the edit interface. Those numbers are project claims rather than independent measurements, but the engineering principle is highly relevant. ([GitHub][8])

### Mine its `read` primitive

OMP defaults to structural summarization rather than blindly dumping large files. It supports bounded reads and structural summaries. ([GitHub][9])

That should be policy in a weak-model harness.

```text
BAD

read_file("5000-line-file.ts")
→ 5000 lines


BETTER

inspect_file("5000-line-file.ts")
→ imports
→ classes
→ signatures
→ relevant regions
```

### Mine Hashline

OMP's default editing format uses content/hash anchored edits rather than fragile line numbers or enormous file regeneration. It includes stale-anchor recovery and supports all-or-nothing multi-file patch application. ([GitHub][9])

This is extremely relevant to small models.

### Mine its AST tooling

OMP exposes:

* `ast_grep`
* `ast_edit`
* structural matching
* dry-run rewrites

through ast-grep/tree-sitter. ([GitHub][10])

### Mine its LSP integration

The LSP layer allows the agent to ask:

```text
definition
references
diagnostics
symbols
implementations
types
```

rather than infer them from raw text.

### Mine persistent execution kernels

OMP has persistent Python and JavaScript environments which can **call agent tools themselves**. ([GitHub][8])

That means:

```text
LLM
 ↓
Python
 ├─ tool.read()
 ├─ tool.search()
 ├─ compute()
 └─ tool.write()
```

The model doesn't have to orchestrate every deterministic step.

This is huge.

### Mine role-based model routing

OMP has model roles including:

* `default`
* `smol`
* `slow`
* `vision`
* `plan`
* `designer`
* `commit`
* `task`

and configuration can map each role to different providers/models and reasoning levels. ([GitHub][11])

For local models this is exactly what you want.

For example:

```text
default
    Qwen3-Coder-30B

smol
    Qwen3-8B

plan
    Qwen3-Next-80B

slow
    DeepSeek-V3.2

vision
    Qwen-VL

commit
    7B model
```

And potentially:

```text
advisor
    strongest model available
```

### Dynamic tools

OMP can keep rarely used tools outside the hot tool namespace rather than exposing every schema all the time. It supports an extended dynamic tool surface and BM25-style discovery mechanisms. ([GitHub][12])

### My assessment

If your goal is specifically:

> **make weaker coding models punch way above their raw benchmark capability**

OMP may currently be the single most useful repository to dissect.

---

# 3. Qwen Code

## `QwenLM/qwen-code`

This one should be especially interesting to you.

It is Apache-2.0, supports Qwen, OpenAI, Anthropic, Gemini APIs and local Ollama/vLLM models. ([GitHub][13])

It now includes:

* Auto-Memory
* Auto-Skills
* subagents
* agent teams
* dynamic workflows
* hooks
* MCP
* plan mode
* LSP
* sandbox
* git worktrees
* computer use
* headless operation
* session management
* daemon mode
* SDK

([GitHub][13])

And unlike a generic harness, it is being developed alongside Qwen models.

That makes it particularly important for understanding **model/harness co-design**.

### Subagents

Qwen's Agent tool can:

* create specialized workers
* fork parent context
* restrict worker tools
* run workers independently
* preserve worker state for continuation

([GitHub][14])

That's excellent source material for your supervisor-worker system.

### My assessment

For somebody specifically experimenting with Qwen locally:

**mandatory reading.**

Even if you do not use Qwen Code itself, compare its prompts/tool interface against your own harness.

---

# 4. Gemini CLI

## `google-gemini/gemini-cli`

Gemini CLI is Apache-2.0 open source. ([GitHub][15])

And from an architectural standpoint it has become very interesting.

## Context policy

Current configuration includes:

* history token budget
* retained token budget
* context compression
* message budgets

([GitHub][16])

## Hooks

This is one of the strongest ideas to copy.

Gemini exposes lifecycle hooks including:

```text
BeforeModel
AfterModel
BeforeToolSelection
```

`BeforeModel` can change:

* context
* prompt
* model parameters

while `BeforeToolSelection` can alter which tools are exposed. ([GitHub][16])

That is exactly where I would insert your Headroom policy.

For example:

```text
BeforeModel
    │
    ├─ context budget check
    ├─ Headroom compression
    ├─ deduplicate tool results
    ├─ retrieve required memories
    ├─ inject current task state
    └─ preserve pinned invariants
```

And:

```text
BeforeToolSelection
    │
    ├─ infer current workflow state
    ├─ expose only relevant tools
    ├─ hide expensive tools
    └─ enforce read/write permissions
```

That is considerably better than asking the model to voluntarily manage itself.

## Policy engine

Gemini CLI has a TOML policy engine controlling whether tool calls are:

* allowed
* denied
* user-confirmed

([GitHub][17])

This deserves serious study given your goal.

## Tool-level sandboxing

It supports sandboxing individual tool invocations rather than necessarily sandboxing the entire UI/process. ([GitHub][18])

## Loop detection

Gemini CLI has explicit automatic infinite-loop detection. ([GitHub][19])

## Isolated subagents

Subagents receive:

* independent context
* custom system prompt
* specific model
* restricted tools
* isolated MCP servers
* per-subagent policy

and are prevented from recursively spawning more subagents. ([GitHub][20])

This is exceptionally good architecture for weaker models.

---

# 5. Google Antigravity SDK

This one surprised me.

## `google-antigravity/antigravity-sdk-python`

Google now exposes an Apache-licensed SDK for the Antigravity harness. ([GitHub][1])

The source shows explicit components for:

```text
Agent
├─ hook runner
├─ policy
├─ tool context
├─ tool runner
└─ trigger runner
```

([GitHub][21])

So I would move Antigravity from your:

> closed system to imitate

bucket into:

> **partially open system whose public SDK should be dissected**

bucket.

The desktop system itself exposes:

* parallel agents
* browser agent
* artifacts
* plans
* code diffs
* architecture diagrams
* browser recordings

([Google Antigravity][22])

The browser agent runs through a separate browser profile and includes URL allow/deny controls. ([Google Antigravity][23])

That's excellent reference material for **proof-producing agents**.

Instead of agent completion being:

```text
"I fixed it."
```

Antigravity's approach encourages:

```text
"I fixed it."

Evidence:
- diff
- screenshot
- browser recording
- test output
- implementation plan
```

Copy that philosophy.

---

# 6. OpenAI Codex CLI

## `openai/codex`

The CLI itself is Apache-2.0 open source. ([GitHub][24])

There is a lot worth mining from modern Codex.

### Sandboxing + approval policy

Codex separates:

```text
technical capability boundary
```

from:

```text
approval policy
```

This is an important distinction.

A command can be mechanically impossible inside the sandbox even if the model wants to do it.

OpenAI's current internal deployment also uses an **automatic reviewer agent** for some permission escalations instead of interrupting the human every time. ([OpenAI][25])

That suggests:

```text
weak worker
     ↓
requests dangerous action
     ↓
cheap policy checks
     ↓
review agent
     ↓
allow / deny / human
```

Excellent pattern.

### Hooks

Current open-source Codex has lifecycle hook machinery covering events such as:

* PreToolUse
* PermissionRequest
* PostToolUse
* PreCompact
* PostCompact
* SessionStart
* SessionEnd
* UserPromptSubmit

([GitHub][26])

Again, this is exactly the direction I would take your harness.

### Skills

Codex's application server has explicit skills discovery/injection rather than simply putting all skills into every system prompt. ([GitHub][27])

### MCP both directions

Codex can act as:

```text
MCP client
```

and experimentally:

```text
MCP server
```

allowing another supervisor agent to invoke Codex itself as a worker. ([GitHub][28])

That's powerful composition.

---

# 7. Serena

## `oraios/serena`

You already know this one, and your instinct is correct.

Serena is not merely "another MCP server."

It is essentially:

> **an IDE intelligence layer for an agent.**

Its project architecture centres around:

* LSP lifecycle
* symbols
* semantic tools
* symbolic editing
* code-editor abstractions
* memories
* modes/context

([GitHub][29])

### Why Serena matters for weak models

Instead of:

```text
grep "authenticateUser"
```

it can expose conceptually:

```text
find symbol authenticateUser

find references authenticateUser

find implementations AuthenticationProvider

rename symbol

safe delete

diagnostics
```

This dramatically reduces reasoning burden.

### Serena memory

Serena also supports:

```text
.serena/memories/
```

plus global human-readable markdown memories. ([GitHub][30])

### Important caution

Serena has had recent security advisories around project activation/dashboard functionality. If you're baking it deeply into an autonomous harness, keep it updated and treat repository configuration as untrusted input rather than blindly trusting arbitrary projects.

---

# 8. Headroom

## `headroomlabs-ai/headroom`

You have already made what I think is the right architectural leap:

> **compression should be policy, not a voluntary skill.**

Headroom is Apache 2.0 and provides:

* library
* proxy
* MCP
* reversible compression
* content-specific compressors
* shared context
* cross-agent memory
* failed-session learning

([GitHub][31])

It handles:

* JSON
* logs
* code
* diffs
* RAG
* files
* conversation context

and stores originals in its CCR system so compressed content can later be recovered. ([GitHub][32])

That reversibility is especially important.

I would make:

```text
COMPRESS
```

different from:

```text
DELETE
```

in your harness.

Something like:

```text
HOT
exact model context

WARM
compressed context + retrieval key

COLD
original external artifact

ARCHIVE
full historical event log
```

Headroom naturally fits the WARM layer.

---

# 9. LLMLingua

## `microsoft/LLMLingua`

MIT.

A research-oriented prompt compression system from Microsoft. ([GitHub][33])

Unlike structural Headroom-style transformations, LLMLingua uses a smaller model to identify lower-information prompt tokens.

I would not blindly insert it everywhere.

But it is worth testing for:

* retrieved documents
* long natural-language evidence
* few-shot examples
* verbose instructions

rather than code.

Use Headroom as your **general deterministic/context-aware policy**, and experiment with LLMLingua for specific natural-language context classes.

---

# 10. Repomix

## `yamadashy/repomix`

Excellent secondary context tool.

Repomix provides:

* repository packing
* token counting
* directory maps
* include/exclude control
* secret detection
* Tree-sitter structural compression
* MCP support

([GitHub][34])

Its structural mode can retain things like:

```text
imports
function signatures
interfaces
class structures
```

while removing implementation bodies.

That's useful for creating your **cold-start repository map**.

I would combine:

```text
Aider map
+
Repomix structural map
+
Serena semantic lookup
```

rather than ever dumping the entire repository.

---

# 11. Aider

Aider remains extremely useful as a source repository.

I'd specifically steal two ideas.

## Repository map

Aider's repo map ranks important symbols and signatures across the repository rather than throwing everything into context.

## Architect/editor split

Aider Architect mode separates:

```text
model 1
reason about change

        ↓

model 2
translate proposal into edits
```

And importantly, even using the **same model twice** can help because the roles and contexts are separated.

That maps very nicely onto weak local models:

```text
Qwen 32B planner

     ↓ structured proposal

Qwen 32B editor
```

instead of expecting one trajectory to do everything.

---

# 12. Tree-sitter

## `tree-sitter/tree-sitter`

MIT. ([GitHub][35])

I would regard Tree-sitter as basic harness infrastructure.

Use it to generate:

* symbol maps
* imports
* classes
* signatures
* dependency approximations
* chunk boundaries
* structural summaries

This lets you move repository understanding out of the LLM.

---

# 13. ast-grep

## `ast-grep/ast-grep`

MIT.

Structural:

* search
* lint
* rewrite
* codemods

on top of Tree-sitter. ([GitHub][36])

Very high value.

I would expose:

```text
text_search
symbol_search
ast_search
references
```

as different concepts.

Do not make the model use regex for everything.

---

# 14. multilspy

## `microsoft/multilspy`

MIT Python library wrapping language servers. ([GitHub][37])

If you don't want Serena as a hard dependency, multilspy is useful for building your own thin semantic code layer.

So:

```text
Serena
```

is the ready-made solution.

```text
multilspy
```

is a useful lower-level building block.

---

# 15. PydanticAI

This has become much more interesting than many people realize.

## Dynamic toolsets

PydanticAI toolsets can be:

* wrapped
* filtered
* changed dynamically
* approval-protected
* deferred
* composed

([GitHub][38])

## Tool search

It now has explicit deferred tool discovery.

Its own docs note that tool selection tends to degrade when agents see roughly 30–50 tools simultaneously, and provides tool search to hide the long tail. ([GitHub][39])

It supports:

```text
keyword
BM25
regex
custom search
```

and provider-specific native tool search when supported. ([GitHub][40])

### This matters enormously for local models

Your local Qwen probably should see something like:

```text
read
search
edit
bash
todo
discover_tool
```

not 128 MCP schemas.

## On-demand capabilities

PydanticAI can lazily expose whole bundles containing:

* instructions
* tools
* hooks
* model settings

([GitHub][41])

This is basically **skills done as runtime components rather than markdown prompt injection**.

Very relevant to your architecture.

---

# 16. ToolHive

## `stacklok/toolhive`

Apache 2.0.

If you're going deep into MCP, look at this carefully.

ToolHive provides:

* isolated MCP-server containers
* registry
* gateway
* identity
* access policy
* audit
* network policy
* tool filtering
* semantic tool search
* deterministic workflows

([GitHub][42])

It also has explicit filesystem/network/privilege permission profiles. ([GitHub][43])

This solves an important architectural problem:

```text
LLM
 ↓
YOUR HARNESS
 ↓
ToolHive
 ↓
MCP servers
```

instead of:

```text
LLM
 ↓
70 random MCP processes
with arbitrary host permissions
```

For your seriousness level, I would strongly consider this.

---

# 17. XGrammar

## `mlc-ai/xgrammar`

Apache-2.0.

For weaker local models this is one of my highest-priority recommendations.

XGrammar guarantees structured generation using constrained decoding and supports:

* JSON Schema
* EBNF
* Lark
* structural tags
* tool calling
* Qwen
* DeepSeek
* Kimi
* Llama
* other common model styles

([GitHub][44])

Instead of:

```text
Please output valid JSON.
```

make invalid JSON **impossible to generate**.

Instead of:

```text
Please call one of these actions correctly.
```

constrain decoding to:

```text
ACTION := READ | SEARCH | EDIT | TEST
```

This is exactly how you help weaker models.

### Also inspect

* Outlines
* lm-format-enforcer
* Instructor

But for local inference, I would start with **XGrammar**.

---

# 18. vLLM / SGLang

For a local harness, the inference server itself should participate in harness quality.

Both can support constrained/structured generation.

SGLang is particularly interesting for agentic local serving because current versions integrate structured-generation engines and reasoning/tool parsers.

I would test:

```text
SGLang
+
XGrammar
```

and:

```text
vLLM
+
XGrammar
```

against your existing inference stack.

The point is that your harness can make malformed tool calls mechanically impossible **before tokens even leave the sampler**.

---

# 19. RouteLLM

## `lm-sys/RouteLLM`

Apache-2.0.

This is specifically for routing between a:

```text
weak model
```

and:

```text
strong model
```

depending on query difficulty. ([GitHub][45])

It explicitly supports examples involving a local Ollama model as the weak endpoint and a stronger model as escalation. ([GitHub][46])

You could extend the concept into:

```text
Qwen 8B
    ↓ uncertain
Qwen 32B
    ↓ validator failure
Qwen 72B
    ↓ repeated failure
frontier API
```

This is more useful than permanently assigning every task to the strongest model.

---

# 20. LiteLLM

## `BerriAI/litellm`

Worth using as a provider abstraction/gateway.

It gives you one normalized interface across many providers. ([GitHub][47])

I would put model **connection/failover/rate-limit plumbing** here and keep intelligent task-routing logic in your harness.

Conceptually:

```text
Harness intelligence
      ↓
OMP/RouteLLM-style router
      ↓
LiteLLM
      ↓
providers
```

---

# 21. Letta

## `letta-ai/letta`

Apache-2.0.

Letta is probably the deepest open-source project to study for **memory as a first-class property of an agent**, rather than RAG bolted onto a chat history. ([GitHub][48])

Even more interesting:

# Letta Code

## `letta-ai/letta-code`

Letta Code explicitly calls itself a **memory-first agent harness**. ([GitHub][49])

Its agents can modify their own persistent context and skills.

The current memory system uses **MemFS**, a git-backed memory filesystem. ([GitHub][50])

That's very interesting.

It supports concepts like:

```text
memory/
├── user/
├── project/
├── architecture/
├── workflows/
├── learned_failures/
└── preferences/
```

and the agent can reorganize that information.

It even has a "dreaming" process where background agents inspect recent sessions and consolidate durable lessons. ([GitHub][50])

That is worth stealing.

---

# 22. Mem0

## `mem0ai/mem0`

Apache-2.0.

Mem0 provides an easier-to-integrate memory subsystem supporting multiple scopes including:

* User
* Session
* Agent

([GitHub][51])

I'd choose:

**Letta** if memory is central to the entire agent identity.

**Mem0** if you want a modular memory service plugged into your own harness.

---

# 23. Graphiti

## `getzep/graphiti`

Worth considering when your long-term memory needs relationships and **time** rather than merely semantic similarity.

Think:

```text
User preferred X
          ↓
later changed to Y
          ↓
because project moved to Z
```

instead of flattening all three into vector entries.

I'd use Graphiti for:

* project history
* architecture evolution
* entity relationships
* changing facts

rather than basic task scratch memory.

---

# 24. LangGraph

## `langchain-ai/langgraph`

This is not my first choice for the model-facing coding interface.

It **is** one of my first choices for the deterministic outer runtime.

LangGraph provides:

* durable execution
* stateful graph workflows
* checkpointing
* human intervention
* memory
* fault recovery

([GitHub][52])

The checkpointer stores state after each graph superstep, enabling:

* time travel
* interruption
* replay
* fault tolerance

([GitHub][53])

So you could have:

```text
START
  ↓
ORIENT
  ↓
PLAN
  ↓
RETRIEVE
  ↓
IMPLEMENT
  ↓
VERIFY
  ├── FAIL → REPAIR
  └── PASS → REVIEW
                 ↓
               DONE
```

and let **LangGraph**, not the weak LLM, own the state transitions.

That's powerful.

---

# 25. Deep Agents

## `langchain-ai/deepagents`

This is worth distinguishing from LangGraph itself.

Deep Agents calls itself a batteries-included harness and bundles:

* filesystem
* planning
* context management
* subagents
* skills

while remaining model-agnostic, including local/self-hosted models. ([GitHub][54])

Its subagents have isolated contexts and narrower toolsets. ([GitHub][55])

This is probably one of the easiest Python projects from which to borrow a **Claude-Code-ish agent architecture**.

---

# 26. Temporal

## `temporalio`

For truly long-running systems, consider putting the most important orchestration **outside the agent framework entirely**.

Temporal is built around deterministic, resumable workflows with explicit retry policies. ([GitHub][56])

Example:

```text
agent task
   ↓
machine crashes
   ↓
restart
   ↓
resume exact workflow state
```

That's much stronger than hoping a JSON session file survived correctly.

I would not necessarily use it for interactive coding turns.

I would use it for:

* jobs lasting hours/days
* queued agents
* CI agents
* scheduled work
* large batch work
* external approvals
* resilient multi-agent jobs

---

# 27. Open Policy Agent

## `open-policy-agent/opa`

Apache-2.0, CNCF project. ([GitHub][57])

Given what you said about turning Headroom into **policy**, you should seriously consider separating policy from agent implementation entirely.

Example:

```text
agent requests:
  tool = bash
  command = rm ...
  cwd = project
  role = worker
```

OPA decides:

```json
{
  "allow": false
}
```

not the LLM.

You can encode:

```text
worker cannot git push

research agent cannot edit

planner cannot shell-write

test agent cannot change source

model under 14B must read target before edit

editing generated files prohibited

network requires explicit domain policy

tests must pass before completion

premium model only allowed for escalation
```

as deterministic rules.

**This matches your philosophy extremely well.**

---

# 28. SWE-agent

SWE-agent should be treated as **research material for agent-computer interface design**.

Some particularly useful findings from its ACI:

* editors immediately run linters
* invalid syntax can block the edit
* file reads are bounded
* search results are compressed
* successful commands explicitly say they succeeded even if stdout is empty

These kinds of tiny interface decisions changed model performance substantially.

That is exactly the level of harness engineering you want for weak models.

I would mine SWE-agent as an empirical answer to:

> **What tool interface does an LLM actually understand well?**

---

# 29. OpenHands

OpenHands is one of the strongest projects to study for the **workspace/runtime layer**.

Its architecture includes concepts such as:

* agent
* conversation state
* tool system
* workspaces
* events
* skills
* condenser
* security

It also supports Dockerized execution.

I'd mine it for:

* local/remote workspace abstraction
* event model
* sandbox lifecycle
* conversation state
* security boundaries

rather than necessarily copy its entire agent UX.

---

# 30. Cline

Cline remains useful source material for:

* Plan/Act separation
* checkpoints
* diff/revert
* terminal monitoring
* MCP
* lifecycle hooks
* agent teams
* Git worktrees

That makes it a good repository to inspect for **safe long-running editing sessions**.

---

# 31. Roo Code

Roo's strongest idea is its mode architecture.

Typical modes include:

```text
Architect
Code
Ask
Debug
Orchestrator
```

Different modes can have different:

* prompts
* tools
* write access
* models

Its Orchestrator delegates tasks rather than directly performing ordinary code edits.

This is an excellent design for weak models.

Don't tell the same model:

> You can do absolutely anything.

Tell it:

```text
CURRENT ROLE: DEBUGGER

Allowed:
  search
  read
  run tests
  diagnostics

Forbidden:
  broad refactoring
  architecture changes
```

You have reduced the effective branching factor.

---

# 32. Goose

## `block/goose` → AAIF ecosystem

Goose is particularly useful as reference for:

* MCP-centric architecture
* local-model support
* portable providers
* ACP
* custom distributions

I'd look at it if you want your harness to become an **agent platform** rather than only a coding application.

---

# 33. Continue

Continue has an idea I particularly like for your project:

## AI checks as source-controlled configuration

Instead of:

> "Remember to review security."

Have a repository-defined agent check that runs in CI.

This lets you turn more model behaviors into:

```text
required pipeline stage
```

rather than:

```text
prompt suggestion
```

Exactly the same general philosophy you've used with Headroom.

---

# 34. Playwright MCP / Playwright CLI

## `microsoft/playwright-mcp`

Apache-2.0.

For testing applications I strongly recommend giving the agent browser state through accessibility snapshots rather than relying solely on screenshots. ([GitHub][58])

Microsoft itself now notes that coding agents may benefit from **CLI + skills instead of MCP** because the CLI representation can be significantly more token-efficient than exposing many MCP schemas and accessibility trees. ([GitHub][58])

That's an important general lesson:

> MCP is not automatically the optimal model interface.

Sometimes:

```text
small CLI
+
good skill
```

beats:

```text
25 giant tool schemas
```

For weaker models, I would test both.

---

# 35. Browser Harness

## `browser-use/browser-harness`

This one is fascinating.

It is a tiny, editable CDP harness where the agent can effectively add missing browser helper functions itself. ([GitHub][59])

It represents another architecture:

```text
small powerful primitive
       +
model writes deterministic automation
```

rather than:

```text
100 browser tools
```

Very relevant to your project.

---

# 36. Browser Use

## `browser-use/browser-use`

Provides a complete open-source browser agent with:

* persistent browser
* tools
* recovery loops
* local model support
* custom actions

([GitHub][60])

Use this when you want **autonomous browser work**.

Use Playwright when you primarily want **deterministic testing/verification**.

Use Browser Harness when you want **minimal primitives and model-generated browser code**.

Different purposes.

---

# 37. Microsandbox

## `superradcompany/microsandbox`

This is currently one of the projects I would inspect first for a serious local agent.

Apache 2.0.

It runs untrusted code inside local microVMs and supports:

* Linux
* macOS
* Windows
* OCI images
* embedded runtime
* no permanent daemon
* hardware isolation

([GitHub][61])

This fits local agents much better than a cloud-only sandbox.

I would consider:

```text
Host
  │
Harness
  │
Microsandbox
  │
Agent workspace
```

with carefully mapped mounts.

---

# 38. gVisor

## `google/gvisor`

Apache-2.0.

For Linux systems where normal containers are insufficient, gVisor inserts a userspace application kernel between the workload and host kernel. ([GitHub][62])

Good middle ground:

```text
Docker
  ↓ weaker isolation

gVisor
  ↓ stronger isolation

microVM
  ↓ strongest boundary
```

---

# 39. Firecracker

## `firecracker-microvm/firecracker`

Apache-2.0.

If you eventually run untrusted autonomous agents on infrastructure, this is the heavyweight option.

Firecracker microVMs use hardware virtualization and deliberately minimize the virtualized device surface. ([GitHub][63])

I wouldn't start here for a personal workstation harness.

I absolutely would consider it for a multi-user agent service.

---

# 40. E2B

## `e2b-dev/E2B`

Apache-2.0, self-hostable.

Gives agent-friendly sandbox lifecycle and code interpreter abstractions rather than forcing you to write VM/container management yourself. ([GitHub][64])

Use if:

> I need agent sandboxes.

Use Firecracker/gVisor/Microsandbox if:

> I am building the sandbox infrastructure.

---

# 41. Langfuse

## `langfuse/langfuse`

Mostly MIT OSS, with enterprise sections separately licensed. ([GitHub][65])

Tracks:

* LLM calls
* tools
* retrieval
* latency
* tokens
* cost
* evaluation
* datasets
* prompts

([GitHub][66])

For your project, tracing is not optional.

You need to know:

```text
Why did Qwen fail?
```

Was it:

```text
wrong context?
wrong search result?
bad edit representation?
bad tool choice?
missing tool?
compaction loss?
model reasoning?
verification failure?
premature stopping?
```

Without traces, all of those look like:

> model dumb

when often they are harness problems.

---

# 42. Phoenix

## `Arize-ai/phoenix`

MIT.

Alternative to Langfuse with strong OpenTelemetry orientation and:

* tracing
* datasets
* experiments
* evaluations
* replay
* prompt iteration

([GitHub][67])

For your use case, I may slightly prefer **Phoenix** because you're building an experimental harness rather than merely monitoring a production SaaS agent.

Either is viable.

---

# 43. Inspect AI

## `UKGovernmentBEIS/inspect_ai`

MIT, from the UK AI Security Institute.

Excellent harness/evaluation framework with:

* tool-using agents
* multi-turn evaluations
* model grading
* extensibility
* hundreds of existing evaluations

([GitHub][68])

I would use this to create your own:

# HarnessBench

For example:

```text
MODEL:
Qwen-X

H1:
raw baseline

H2:
+ Serena

H3:
+ Serena
+ Headroom

H4:
+ Serena
+ Headroom
+ hashline

H5:
+ all above
+ verification loop

H6:
+ all above
+ subagents
```

Run exactly the same 100 tasks.

Now you know **which harness feature actually buys intelligence**.

That would be extremely valuable.

---

# 44. Promptfoo

Useful for:

* regression evals
* red teaming
* output tests
* provider comparisons
* local Ollama/OpenAI-compatible models

Its red-team/eval infrastructure can operate against local endpoints. ([GitHub][69])

I'd use:

**Inspect** for deep agent trajectory benchmarking.

**Promptfoo** for rapid CI regression checks.

---

# 45. Route tools through a real policy layer

This is where I'd go beyond most existing harnesses.

You have already done this with Headroom.

I'd extend the philosophy.

Instead of:

```text
MODEL:
"remember to search before reading"
```

make:

```text
POLICY:
large file read rejected unless:
- relevant region known
- structural summary tried
```

Instead of:

```text
MODEL:
"remember to run tests"
```

make:

```text
completion transition blocked
unless verification state == PASS
```

Instead of:

```text
MODEL:
"please don't keep calling the same command"
```

make:

```text
loop detector:
same normalized tool call ×3
→ block
→ force replan
```

Instead of:

```text
MODEL:
"don't overwrite generated code"
```

make:

```text
edit policy:
generated-file classifier
→ deny
```

That is where **OPA + lifecycle hooks + event-sourced state** become very powerful.

---

# My recommended architecture for your project

Given that Headroom is already an automatic policy, I would build something approximately like this:

```text
                         USER
                           │
                           ▼
               ┌─────────────────────┐
               │   ORCHESTRATOR      │
               │ event/state machine │
               └──────────┬──────────┘
                          │
           ┌──────────────┼────────────────┐
           │              │                │
           ▼              ▼                ▼
      POLICY BUS      CONTEXT BUS      MODEL ROUTER
           │              │                │
           │              │                ├─ tiny
           │              │                ├─ default
           │              │                ├─ slow
           │              │                ├─ planner
           │              │                ├─ vision
           │              │                └─ reviewer
           │              │
           │              ├─ Headroom
           │              ├─ Serena
           │              ├─ repo map
           │              ├─ task state
           │              ├─ memories
           │              └─ retrieval
           │
           ├─ OPA
           ├─ permissions
           ├─ loop detection
           ├─ read-before-edit
           ├─ verify-before-done
           ├─ token budgets
           └─ escalation rules
                          │
                          ▼
                    AGENT LOOP
                          │
              ┌───────────┴───────────┐
              │                       │
              ▼                       ▼
          TOOL ROUTER              SUBAGENTS
              │                       │
        progressive                  isolated
        disclosure                   contexts
              │
   ┌──────────┼───────────────────────────────┐
   │          │          │         │          │
   ▼          ▼          ▼         ▼          ▼
 Serena    Hashline   ast-grep    LSP      Playwright
   │
   ├─────────────────────────┐
   │                         │
   ▼                         ▼
 filesystem               execution
                          sandbox
                             │
                      Microsandbox
                             │
                             ▼
                           tests
                             │
                     compiler/linter
                             │
                             ▼
                       VERIFICATION
                             │
                  ┌──────────┴─────────┐
                  │                    │
                PASS                  FAIL
                  │                    │
                  ▼                    ▼
                DONE              REPAIR LOOP


   ALL EVENTS
       │
       ├── Phoenix/Langfuse
       ├── event log
       ├── replay
       └── HarnessBench
```

That is roughly what I would consider a **next-generation local model harness**.

---

# More specifically: what I would steal from each project

Here's my actual shopping list.

| Source               | Steal this                             |
| -------------------- | -------------------------------------- |
| **DeepSeek Harness** | plugin/event architecture              |
| **DeepSeek Harness** | event-sourced canonical session        |
| **DeepSeek Harness** | replaceable compaction seam            |
| **DeepSeek Harness** | workflow/subagent engine               |
| **OMP**              | Hashline                               |
| **OMP**              | summarized reads                       |
| **OMP**              | model-specific prompting               |
| **OMP**              | persistent execution kernel            |
| **OMP**              | role-based model routing               |
| **OMP**              | dynamic tools                          |
| **Qwen Code**        | local-model co-design                  |
| **Qwen Code**        | teams/subagents                        |
| **Gemini CLI**       | BeforeModel hook                       |
| **Gemini CLI**       | BeforeToolSelection hook               |
| **Gemini CLI**       | explicit policy engine                 |
| **Gemini CLI**       | subagent tool isolation                |
| **Codex**            | permission reviewer agent              |
| **Codex**            | sandbox vs approval separation         |
| **Codex**            | lifecycle hooks                        |
| **Antigravity**      | artifacts/proof of work                |
| **Antigravity**      | browser subagent                       |
| **Headroom**         | always-on reversible context reduction |
| **Serena**           | symbolic code intelligence             |
| **Aider**            | repo map                               |
| **Aider**            | architect/editor split                 |
| **Repomix**          | static project map                     |
| **Tree-sitter**      | syntax infrastructure                  |
| **ast-grep**         | deterministic structural editing       |
| **multilspy**        | LSP abstraction                        |
| **PydanticAI**       | deferred toolsets                      |
| **PydanticAI**       | on-demand capability bundles           |
| **ToolHive**         | MCP registry/isolation                 |
| **XGrammar**         | mechanical output correctness          |
| **RouteLLM**         | capability escalation                  |
| **Letta**            | persistent agent identity              |
| **Mem0**             | modular memory                         |
| **Graphiti**         | temporal entity memory                 |
| **LangGraph**        | deterministic workflow state           |
| **Temporal**         | crash-proof long-running work          |
| **OPA**              | hard policy                            |
| **Microsandbox**     | local execution isolation              |
| **Playwright**       | deterministic UI verification          |
| **Browser Harness**  | minimal browser primitive              |
| **Phoenix**          | trajectory tracing                     |
| **Inspect AI**       | harness evaluation                     |
| **SWE-agent**        | empirically optimized ACI              |

---

# Things I would NOT duplicate

I would specifically **avoid writing your own**:

* AST parser
* LSP protocol stack
* MCP registry
* browser driver
* microVM system
* vector database
* general workflow persistence
* structured-generation decoder
* telemetry database
* provider compatibility gateway

Those are solved problems.

Your intellectual work should be concentrated on:

1. **policy**
2. **context-selection strategy**
3. **tool surface**
4. **workflow state machine**
5. **failure recovery**
6. **model-specific adaptation**
7. **verification**
8. **routing**
9. **evals**

That is where your harness becomes genuinely differentiated.

---

# What I would actually fork

This is a different question from "what should I study."

If you wanted me to choose a chassis:

### Option A — DeepSeek Harness

Use if your priority is:

> **maximum architectural control**

Best fit for the kind of policy-centric system you're describing.

I would probably choose this for research.

### Option B — Oh My Pi

Use if your priority is:

> **get spectacular coding-agent performance now**

Then graft:

```text
OMP
+ Headroom
+ Serena
+ XGrammar
+ Microsandbox
+ Phoenix
```

This is probably the quickest path to your "Qwen feels like Fable/Sol" objective.

### Option C — Qwen Code

Use if:

> Qwen is going to remain your primary model family.

Then you are benefiting from direct model/harness co-development.

### Option D — Gemini CLI

Use if:

> lifecycle policy and hooks matter more than exotic coding tools.

Its current hook/policy/subagent design is excellent.

### Option E — Deep Agents + LangGraph

Use if:

> I want a Python-native agent runtime that I can completely rewrite around.

Probably the cleanest traditional framework route.

---

# What I would personally do in your case

Because you've already reached the point of making **Headroom a harness policy**, I don't think you should switch entirely into someone else's opinionated harness.

I'd use a **Franken-harness deliberately**.

### Core

Either:

```text
DeepSeek Harness event architecture
```

or your existing runtime if changing core would be expensive.

### Context

```text
Headroom
    +
Serena
    +
Aider-style repo map
    +
Repomix cold map
    +
memory tiers
```

### Editing

```text
OMP Hashline
    +
ast-grep
    +
Serena/LSP
```

### Tool surface

```text
hot tools:
read
search
edit
run
test
todo
discover

cold tools:
ToolHive / dynamic discovery
```

### Model output

```text
XGrammar
```

wherever possible.

### Roles

```text
tiny
default
planner
slow
critic
test
vision
```

OMP-style.

### Outer workflow

```text
deterministic FSM
```

with LangGraph if convenient.

### Persistent state

```text
event log
+
git
+
task state
+
checkpoints
```

### Long-term memory

I'd experiment with:

```text
Letta-style MemFS
```

before blindly adding an enormous vector memory system.

Human-readable, git-backed memory has a lot going for it.

### Sandbox

For local:

```text
Microsandbox
```

would be high on my testing list. ([GitHub][61])

### Policy

```text
hooks
+
OPA-like deterministic rules
```

### Verification

```text
compiler
lint
unit tests
integration tests
Playwright
static checks
```

### Observability

```text
Phoenix
```

### Evaluation

```text
Inspect AI
+
your own coding task suite
```

---

# And then do something extremely important

Build an **ablation benchmark**.

Don't benchmark:

> Qwen in my harness.

Benchmark:

```text
Qwen raw
Qwen + tools
Qwen + tools + Serena
Qwen + tools + Serena + Headroom
Qwen + those + Hashline
Qwen + those + FSM
Qwen + those + verification loop
Qwen + those + critic
Qwen + those + subagents
```

Measure:

```text
task success
edit success
tests passed
tokens
tool calls
failed calls
repair iterations
context size
time
model escalations
premature completions
loops
```

That will tell you which pieces create **actual effective intelligence**.

Inspect AI is well suited to building this kind of repeatable evaluation harness. ([GitHub][68])

And the SWE-agent work gives you precedent for treating the agent-computer interface itself as an empirical variable rather than assuming "tool calling is tool calling."

---

# The deeper architecture I think you're converging on

What you've done by turning Headroom from a skill into policy is the key move.

I would keep moving responsibilities downward:

```text
                MODEL
                  │
           judgment/reasoning
                  │
        ─────────────────────
                  │
               HARNESS
                  │
     planning constraints
     context allocation
     memory retrieval
     model routing
     tool selection constraints
     completion criteria
     failure classification
                  │
        ─────────────────────
                  │
          DETERMINISTIC LAYER
                  │
     JSON grammar
     AST parser
     LSP
     calculator
     search
     compiler
     tests
     policies
     permissions
     sandbox
     checkpoints
     event log
```

Every time you can move something from:

> **"the model should remember to do X"**

to:

> **"the system guarantees X"**

you effectively make the model stronger.

That's particularly true below frontier scale.

And having surveyed this ecosystem, I don't think you need to invent very much infrastructure at all. The missing piece is mostly **assembling these ideas into one coherent policy-driven system and then empirically tuning that system per model**.

The five repositories I would open first, before touching another major piece of your harness, are **DeepSeek Harness**, **Oh My Pi**, **Gemini CLI**, **Qwen Code**, and **PydanticAI**. They represent five different but unusually sophisticated answers to the harness problem. Then I would treat **Headroom + Serena + XGrammar + ToolHive + Microsandbox + Phoenix + Inspect AI** as your initial subsystem toolbox. ([GitHub][2])

If you keep developing this, monitoring those repositories and adjacent agent-harness projects for new architectural ideas would actually be useful.
