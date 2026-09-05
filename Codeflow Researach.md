**The start**

OpenAI has explicitly argued that the evaluation harness can change whether a capability appears at all, especially on long-horizon tasks. SWE-agent similarly found that redesigning the interface between the model and the computer substantially changed software-engineering performance. ([OpenAI][1])

So a useful approximation is:

> **Agent capability ≈ model capability × harness quality × environment quality × available compute**

It is not literally multiplication, but it captures why a modest local model inside an excellent agent loop can sometimes feel dramatically more capable than the same model in a chat box.

One qualification: the best harness is **model-adaptive**. More scaffolding is not universally better. Aider, for example, notes that some weaker models become confused by repository maps, and Anthropic has observed that overly large or overlapping toolsets hurt tool selection. ([Aider][2])

Below is the master list I would use.

---

# 1. Context engineering

This is probably the largest multiplier after tools.

Anthropic describes context engineering as optimizing the usefulness of the finite tokens available to the model rather than merely writing a clever system prompt. ([Anthropic][3])

A strong harness should have:

1. **Dynamic context selection** rather than dumping everything into the prompt.
2. **Relevance-ranked retrieval**.
3. **Token-budget-aware retrieval**.
4. **Different context budgets for different information classes**.
5. **Recent information prioritized over stale information**.
6. **Task-specific context assembly**.
7. **Progressive disclosure**, where detailed information is loaded only when needed.
8. **Context compaction** when conversations become long.
9. **Loss-aware compaction**, preserving decisions, constraints and unresolved issues.
10. **Hierarchical summaries**, such as project → module → file → function.
11. **Structured state outside the model context**.
12. **Scratch files** instead of expecting the model to remember everything.
13. **Persistent progress notes**.
14. **Separate working memory and long-term memory**.
15. **Episodic memory**, recording what happened previously.
16. **Semantic memory**, recording durable facts about the project.
17. **Procedural memory**, recording how particular tasks should be performed.
18. **Explicit unresolved-question storage**.
19. **Decision logs**.
20. **Assumption logs**.
21. **Artifact indexes**.
22. **Context deduplication**.
23. **Removal of stale intermediate outputs**.
24. **Removal of irrelevant tool results**.
25. **Context isolation between unrelated subtasks**.
26. **Context provenance**, so the agent knows where information came from.
27. **Conflict detection between retrieved pieces of context**.
28. **Priority rules for conflicting instructions**.
29. **Automatic retrieval of previous relevant work**.
30. **Selective rehydration after compaction**.

This is especially important for weak models because every irrelevant token creates another possible distraction.

---

# 2. Give the model a map, not the entire world

One of the most important harness concepts is **compressed environmental representation**.

OpenAI describes its own experience as essentially "give the agent a map, not a thousand-page manual." ([OpenAI][4])

For coding agents this includes:

31. Repository maps.
32. Directory summaries.
33. AST-derived symbol maps.
34. Class/function signatures.
35. Import/dependency graphs.
36. Call graphs.
37. Module ownership maps.
38. API indexes.
39. Database-schema summaries.
40. Architecture diagrams represented structurally.
41. Changed-file indexes.
42. Recent git-history summaries.
43. Search indexes.
44. LSP-derived symbol navigation.
45. Type information.
46. Test-to-source mappings.
47. Feature-to-code mappings.

Aider's repo map is an excellent example. It uses tree-sitter plus graph ranking to expose important symbols from across a repository without putting the entire repository in context. ([Aider][5])

The general idea extends beyond coding.

For research, make a **source map**.

For finance, make a **company/metric/entity map**.

For legal work, make a **case/statute/issue map**.

For a large organization, make an **people/project/document map**.

---

# 3. Excellent tool design

Giving a model tools is only the beginning.

**Tool ergonomics** matter enormously.

Anthropic found that tools should be designed specifically around how agents perceive and select actions, rather than simply exposing every underlying API endpoint. ([Anthropic][6])

A strong harness has:

48. Small numbers of high-value tools.
49. Clearly differentiated tools.
50. Minimal tool overlap.
51. Semantically obvious tool names.
52. Namespaced tools.
53. Clear parameter descriptions.
54. Meaningful parameter names.
55. Good default parameters.
56. Bounded outputs.
57. Pagination.
58. Filtering.
59. Range selection.
60. Search rather than massive list operations.
61. Concise/detailed response modes.
62. Natural-language identifiers alongside machine IDs.
63. Tool usage examples.
64. Examples of when **not** to use a tool.
65. Automatic parameter validation.
66. Automatic repair of trivial malformed arguments.
67. Helpful error messages.
68. Errors returned in model-readable language.
69. Retryable versus non-retryable error classification.
70. Tool capability metadata.
71. Tool permission metadata.
72. Tool cost/latency metadata.
73. Tool-side aggregation.
74. Tool-side joins.
75. Tool-side filtering.
76. Tool-side sorting.
77. Tool-side computation.
78. Tool-side deduplication.

A weak model should not have to fetch 5,000 records and reason token-by-token about which three matter.

The tool should return the three relevant records.

That principle alone can turn surprisingly small models into decent agents.

---

# 4. Dynamic tool loading

Another large improvement is not showing every possible tool to the model.

Modern systems increasingly support:

79. Tool search.
80. Dynamic tool discovery.
81. Task-specific tool subsets.
82. Lazy loading of tool schemas.
83. Skill-specific tool bundles.
84. Tool recommendations from the orchestrator.
85. Tool aliases appropriate to the current model.
86. Automatic hiding of irrelevant tools.

Anthropic introduced tool search specifically because putting hundreds or thousands of tool definitions directly into context wastes context and complicates selection. ([Anthropic][7])

This disproportionately helps weaker models.

---

# 5. Programmatic tool orchestration

One of the most powerful harness tricks is realizing:

**the LLM does not need to reason between every operation.**

Suppose you need to:

- fetch 100 records
- filter them
- calculate values
- deduplicate them
- rank them
- select five

A poor harness makes the model perform all of that.

A strong harness lets it write or invoke deterministic code.

Useful capabilities include:

87. Python execution.
88. Shell execution.
89. SQL execution.
90. JavaScript execution.
91. Sandboxed computation.
92. Programmatic API chaining.
93. Loops outside the LLM.
94. Conditional logic outside the LLM.
95. Deterministic filtering.
96. Deterministic aggregation.
97. Deterministic parsing.
98. Deterministic sorting.
99. Deterministic arithmetic.
100.  Deterministic schema conversion.

Anthropic and OpenAI both increasingly support this pattern because it prevents large intermediate results from filling model context unnecessarily. ([Anthropic][7])

This is one of the most important principles for local models:

> **Never use neural reasoning for something ordinary software can do perfectly.**

---

# 6. Constrain the model's action space

Weak models often perform much better when you reduce the number of possible wrong outputs.

Examples:

101. JSON Schema.
102. Grammar-constrained generation.
103. Enumerated choices.
104. Typed function arguments.
105. Strict tool schemas.
106. State-specific allowed actions.
107. Structured planning objects.
108. Structured task states.
109. Structured completion criteria.
110. Structured error reports.
111. Structured patch formats.
112. Structured citations.
113. Structured memory entries.

OpenAI's Structured Outputs work is a particularly clear demonstration: deterministic constrained decoding can eliminate classes of formatting/schema errors that prompting alone does not reliably solve. ([OpenAI][8])

This illustrates a broader rule:

> **If a behavior can be guaranteed mechanically, do not ask the model to remember to do it.**

---

# 7. Replace open-ended agency with bounded workflows where possible

A weak model often struggles with:

> "Figure everything out and accomplish the goal."

It can perform much better with:

> inspect → plan → retrieve → implement → test → repair → verify → report

Therefore a good harness supports:

114. Finite-state machines.
115. Explicit workflow graphs.
116. Deterministic routing.
117. Conditional transitions.
118. Required stages.
119. Optional stages.
120. Retry transitions.
121. Recovery transitions.
122. Escalation transitions.
123. Completion gates.

Anthropic distinguishes workflows, where code controls orchestration, from agents, where the model determines its trajectory. For well-defined tasks, workflows can offer much more predictable behavior. ([Anthropic][9])

For weaker models I would strongly favor **hybrid systems**:

**deterministic outer loop + agentic inner loop.**

That architecture is extremely powerful.

---

# 8. Explicit decomposition

Strong models can implicitly decompose a problem.

Weak models frequently need decomposition imposed externally.

A good harness supports:

124. Task decomposition.
125. Subtask generation.
126. Dependency ordering.
127. Critical-path identification.
128. Priority assignment.
129. One-subtask-at-a-time execution.
130. Subtask completion checks.
131. Replanning after failure.
132. Replanning after new information.
133. Preventing giant one-shot attempts.

Anthropic found this directly in long-running coding experiments: having the agent work incrementally on one feature at a time was important for preventing it from attempting the entire project at once. ([Anthropic][10])

---

# 9. Explicit acceptance criteria

Weak agents frequently suffer from **premature victory**.

They implement something and assume it works.

The harness should therefore maintain:

134. Feature checklist.
135. Definition of done.
136. Acceptance criteria.
137. Required test cases.
138. Required artifacts.
139. Required evidence.
140. Blocking conditions.
141. Unresolved failures.
142. Progress percentage.
143. Pass/fail status independent from model claims.

Anthropic's long-running harness used a structured feature list with features initially marked failing, and only changed their status after testing. ([Anthropic][10])

This is extremely important.

The model should not decide that the entire task is complete merely because it feels complete.

---

# 10. Independent verification

Probably the biggest multiplier after context and tools.

The model's answer should often be treated as a **candidate**, not truth.

A harness can provide:

144. Syntax validation.
145. Type checking.
146. Linting.
147. Unit tests.
148. Integration tests.
149. End-to-end tests.
150. Browser automation.
151. Screenshot comparison.
152. Compiler feedback.
153. Runtime tests.
154. Database constraints.
155. Schema validation.
156. Mathematical verification.
157. Citation validation.
158. Source existence checks.
159. Claim/source entailment checks.
160. Output validators.
161. Policy validators.
162. Regression testing.
163. Property-based testing.
164. Static analysis.
165. Security scanners.
166. Formatting checks.

This is another example of moving intelligence into the environment.

The model writes code.

**The compiler tells it whether the syntax is valid.**

The model writes a feature.

**The browser tells it whether the feature actually works.**

Anthropic reported significant benefits from explicitly requiring agents to perform end-to-end browser testing rather than merely inspecting code or running shallow tests. ([Anthropic][10])

---

# 11. Automatic repair loops

Verification becomes dramatically more useful when connected to repair.

For example:

```text
generate
↓
validate
↓
failure
↓
return exact failure
↓
repair
↓
validate again
```

A good harness includes:

167. Retry after tool failure.
168. Retry after parser failure.
169. Retry after compilation failure.
170. Retry after test failure.
171. Targeted error feedback.
172. Retry budgets.
173. Modified retry prompts.
174. Failure-specific recovery strategies.
175. Rollback after destructive failure.
176. Escalation after repeated failure.

This lets even mediocre models iteratively converge.

---

# 12. Backtracking

Normal chat interfaces encourage a disastrous behavior:

**every previous decision becomes effectively permanent.**

Good agent harnesses permit:

177. Git checkpoints.
178. State snapshots.
179. File checkpoints.
180. Database transactions.
181. Undo.
182. Rollback.
183. Forking.
184. Alternative trajectory exploration.
185. Restoring last-known-good states.

Claude Code added checkpoints, while durable runtimes such as LangGraph support checkpointing and state replay. ([Anthropic][11])

This matters enormously for weaker models because they make more bad intermediate decisions.

---

# 13. Maintain a last-known-good state

Closely related but worth separating.

Before beginning new work:

186. Run baseline tests.
187. Record passing tests.
188. Record current commit.
189. Record working configuration.
190. Detect pre-existing failures.
191. Avoid attributing old failures to new work.

After completing work:

192. Run tests again.
193. Compare against baseline.
194. Reject regressions.
195. Commit only clean state.

Anthropic's long-running experiment specifically had agents start by verifying basic functionality before beginning another feature. ([Anthropic][10])

---

# 14. Persistent task state

Do not make the model reconstruct the entire project from conversation history.

Keep state such as:

196. Current objective.
197. Current subtask.
198. Completed tasks.
199. Failed tasks.
200. Blocked tasks.
201. Next action.
202. Changed files.
203. Open questions.
204. Discovered constraints.
205. Important commands.
206. Test status.
207. Environment status.
208. Decisions made.
209. Remaining acceptance criteria.

Anthropic used progress files plus git history specifically to let fresh contexts get up to speed quickly. ([Anthropic][10])

---

# 15. Long-running session handoff

Context compaction by itself is insufficient.

A very good harness creates **handoff artifacts**.

At the end of an execution window:

210. Summarize accomplished work.
211. Record what remains.
212. Record current failures.
213. Record relevant files.
214. Record why decisions were made.
215. Record commands needed to restart.
216. Leave workspace clean.
217. Commit progress.
218. Write next recommended action.

At the beginning:

219. Read progress state.
220. Read recent commits.
221. Read acceptance criteria.
222. Verify environment.
223. Resume from explicit next step.

Anthropic's long-running-agent work is essentially built around this concept. ([Anthropic][10])

---

# 16. Specialist skills

Instead of stuffing every instruction into the system prompt, give the model reusable procedural modules.

Examples:

224. PDF skill.
225. Spreadsheet skill.
226. React skill.
227. Database migration skill.
228. Research skill.
229. Legal-analysis skill.
230. Code-review skill.
231. Debugging skill.
232. Security-review skill.

Each skill can include:

- instructions
- examples
- scripts
- templates
- reference files
- tools

Anthropic's Agent Skills architecture uses this progressive-disclosure approach: specialized instructions and resources are discovered and loaded only when relevant. ([Anthropic][12])

This is almost like giving the model dynamically loaded **software for thinking**.

---

# 17. Few-shot procedural examples

For weaker models especially, tool schemas are often insufficient.

Give examples showing:

233. Correct tool call.
234. Incorrect tool call.
235. Multi-step workflow.
236. Error recovery.
237. Expected output.
238. Boundary conditions.
239. When not to call the tool.

Anthropic specifically notes that JSON schemas do not fully communicate tool-use conventions, and examples can communicate those usage patterns. ([Anthropic][7])

---

# 18. Specialized agents

Instead of one model doing everything, separate responsibilities.

Possible agents:

240. Planner.
241. Researcher.
242. Implementer.
243. Reviewer.
244. Test agent.
245. Debugger.
246. Security reviewer.
247. Citation verifier.
248. Documentation agent.
249. Context summarizer.
250. Memory manager.
251. Tool router.
252. Supervisor.

Even if every role uses the **same small model**, specialization can help because each invocation gets a much simpler problem and narrower context.

---

# 19. Supervisor-worker architecture

A useful architecture is:

```text
             Supervisor
          /      |       \
   Research   Implement   Test
       \          |        /
            Reviewer
               |
             Final
```

The supervisor does not necessarily perform the work.

It:

253. decomposes tasks
254. assigns tasks
255. monitors progress
256. detects failure
257. synthesizes outputs
258. decides whether another pass is required

This reduces the planning burden on worker models.

---

# 20. Parallelism

Some problems benefit from several independent attempts.

Harnesses can:

259. Search multiple sources simultaneously.
260. Generate multiple solutions.
261. Run independent debugging attempts.
262. Ask several agents to inspect different modules.
263. Compare candidate approaches.
264. Merge independent findings.

Parallelism turns additional inference compute into higher effective capability.

---

# 21. Self-consistency and ensembles

Instead of:

> Ask model once.

Do:

```text
answer A
answer B
answer C
answer D
answer E

→ evaluate
→ select consensus/best candidate
```

Self-consistency has been shown to substantially improve reasoning on some benchmarks. ([arXiv][13])

Harness features include:

265. Multiple candidate generation.
266. Majority voting.
267. Weighted voting.
268. Consensus detection.
269. Candidate ranking.
270. Diversity prompting.
271. Independent sampling.

This can be particularly attractive with cheap local models.

Five runs of a cheap 7B/14B model may still be much cheaper than a frontier API call.

---

# 22. Search over reasoning trajectories

You can go further than simple ensembles.

A harness can support:

272. Branching candidate plans.
273. Plan scoring.
274. Pruning.
275. Backtracking.
276. Beam search.
277. Tree search.
278. Best-first search.
279. Monte Carlo-style exploration.

Tree of Thoughts demonstrated the basic principle that explicit exploration and backtracking can greatly outperform a single linear reasoning trajectory on some tasks. ([arXiv][14])

A model that is mediocre at producing the correct solution immediately may be quite good at **recognizing which of five proposed directions looks promising**.

---

# 23. Separate generator and verifier

This is one of my favorite architectures for weaker models.

```text
Model A:
produce candidate

Model B:
find problems

Model A:
repair

Model B:
recheck
```

Or even:

```text
weak model → generator
weak model → critic
deterministic system → validator
weak model → repair
```

Roles include:

280. Generator.
281. Critic.
282. Judge.
283. Verifier.
284. Repairer.

The same weights can perform every role, but context isolation prevents the model from simply reinforcing its original answer.

Reflexion demonstrated the usefulness of feeding explicit feedback from previous attempts into subsequent attempts without changing model weights. ([arXiv][15])

---

# 24. Confidence and escalation

A strong harness should know when **not** to trust its cheap model.

285. Confidence estimation.
286. Validator-based confidence.
287. Failure-count threshold.
288. Uncertainty detection.
289. Ambiguity detection.
290. Escalation to a stronger model.
291. Escalation to a human.
292. Additional retrieval when uncertain.
293. Additional verification when uncertain.

This permits extremely effective **model cascades**:

```text
tiny model
   ↓ uncertain
small model
   ↓ uncertain
large model
   ↓ high stakes
human
```

Language-model cascade research formalizes this general idea of composing repeated model interactions and verification structures into more capable systems. ([arXiv][16])

---

# 25. Model routing

Different models have different strengths.

The harness can dynamically choose:

294. Coding model.
295. Vision model.
296. Fast classifier.
297. Long-context model.
298. Reasoning model.
299. Embedding model.
300. Local model.
301. Frontier fallback.

You do not need Sol/Opus-class intelligence for:

- classifying a file
- extracting metadata
- sorting search results
- summarizing a tool response
- choosing between three simple routes

Save expensive intelligence for where it matters.

---

# 26. Use non-LLM components aggressively

This may be the single most important design attitude.

Use:

302. Regex.
303. Parsers.
304. ASTs.
305. Compilers.
306. Databases.
307. Search engines.
308. Embeddings.
309. Graph algorithms.
310. SAT/SMT solvers.
311. Calculators.
312. Symbolic algebra.
313. Optimization solvers.
314. Linters.
315. Type checkers.
316. Schema validators.

A 7B model + excellent deterministic infrastructure can outperform a much larger model that is forced to mentally simulate everything.

---

# 27. Good editing primitives

Coding models are substantially affected by how they are allowed to edit.

Better primitives include:

317. `apply_patch`.
318. Search-and-replace.
319. Line-range editing.
320. AST editing.
321. File creation separately from file modification.
322. Diff previews.
323. Patch validation.
324. Automatic indentation repair.
325. Conflict detection.

Poor editing interfaces force the model to regenerate giant files, increasing:

- token usage
- accidental deletions
- regressions
- hallucinated code

SWE-agent's research is especially relevant here because its core thesis is that the **agent-computer interface itself** materially affects LM performance. ([arXiv][17])

---

# 28. Search before read

Weak models benefit enormously from:

```text
search
↓
identify relevant region
↓
read 50 lines
```

rather than:

```text
read 15,000-line file
↓
hope model notices important part
```

Harness features:

326. Code search.
327. Semantic search.
328. Exact text search.
329. Symbol search.
330. Reference search.
331. Definition search.
332. Narrow-range file reads.
333. Context around matches.
334. Search-result ranking.

This sounds mundane but is an enormous intelligence multiplier.

---

# 29. Read before edit

Force useful behavioral invariants:

335. Search before modifying.
336. Read target file before modifying.
337. Inspect dependencies before changing API.
338. Inspect tests before implementing behavior.
339. Check existing abstractions before adding new ones.
340. Inspect git diff after modification.

Again, these can be **harness rules**, rather than hoping the model remembers.

---

# 30. Plan before expensive action

Some actions should require an explicit intermediate representation.

For example:

```json
{
  "goal": "...",
  "files_to_inspect": [],
  "expected_changes": [],
  "validation_plan": []
}
```

Benefits:

341. Catches misunderstanding early.
342. Creates persistent state.
343. Makes review possible.
344. Allows deterministic policy checks.
345. Reduces impulsive tool use.

But this should be adaptive. Requiring plans for trivial operations just creates token overhead.

---

# 31. Tool-result transformation

Raw API responses are often terrible model context.

The harness should transform:

```text
ugly internal API JSON
```

into:

```text
semantic agent-friendly result
```

Techniques:

346. Remove irrelevant metadata.
347. Resolve IDs to names.
348. Normalize dates.
349. Normalize units.
350. Sort by likely relevance.
351. Highlight abnormalities.
352. Group related results.
353. Include surrounding context.
354. Precompute useful statistics.
355. Mark missing values clearly.

Anthropic specifically reports that resolving opaque IDs into semantically meaningful identifiers improved retrieval precision. ([Anthropic][6])

---

# 32. Environment introspection

Before acting, the agent should be able to cheaply answer:

356. Where am I?
357. What project is this?
358. What files exist?
359. What runtime is available?
360. What dependencies are installed?
361. What tools can I use?
362. What branch am I on?
363. What changed recently?
364. What services are running?
365. What tests exist?
366. What permissions do I have?

Claude's long-running harness starts sessions with exactly this kind of orientation. ([Anthropic][10])

---

# 33. Deterministic bootstrapping

Have a predictable initialization stage.

It can:

367. Inspect environment.
368. Load project instructions.
369. Load progress state.
370. Verify dependencies.
371. Start required services.
372. Run baseline smoke tests.
373. Build repository map.
374. Retrieve active task.
375. Calculate context budget.

This prevents every agent session from reinventing startup behavior.

---

# 34. Environment standardization

Weak models struggle more with unexpected environments.

A strong harness provides:

376. Stable directory conventions.
377. Predictable naming.
378. Standard scripts.
379. Standard test commands.
380. Standard build commands.
381. Standard logging.
382. Standard config layout.
383. Reproducible environments.
384. Containers.
385. Dependency locking.

OpenAI's harness-engineering write-up emphasizes strict boundaries and predictable architecture as a way of making agent operation more reliable. ([OpenAI][4])

---

# 35. Make invariants mechanically enforceable

Instead of:

> "Please don't import layer X from layer Y."

Do:

> CI rejects imports from X to Y.

Examples:

386. Architecture tests.
387. Import restrictions.
388. Database constraints.
389. Required type checks.
390. Dependency rules.
391. Formatting rules.
392. API compatibility tests.
393. Security policies.

OpenAI describes this philosophy as enforcing architectural invariants rather than micromanaging every implementation decision. ([OpenAI][4])

This is exactly what you want for weaker models.

---

# 36. Guardrails

Good guardrails are not merely about safety.

They also reduce the search space.

394. Read-only by default.
395. Require approval for destructive actions.
396. Restrict writable directories.
397. Restrict external hosts.
398. Restrict available shell commands where appropriate.
399. Detect suspicious instructions.
400. Separate trusted instructions from untrusted retrieved text.
401. Protect secrets.
402. Validate high-impact actions.
403. Require confirmation for irreversible operations.

A model behaves more reliably when the environment makes catastrophic mistakes impossible.

---

# 37. Sandboxing

Allow the agent to experiment safely.

404. Disposable filesystem.
405. Containerized shell.
406. Restricted network.
407. Temporary database.
408. Test credentials.
409. Resource limits.
410. Process timeout.
411. Disk limits.
412. CPU limits.
413. Memory limits.

Now the agent can try things instead of merely reasoning about what might happen.

---

# 38. State persistence and durable execution

Long jobs should survive:

- context exhaustion
- model failure
- tool failure
- process crash
- deployment
- human interruption

Capabilities:

414. Checkpoints.
415. Durable state.
416. Resume tokens.
417. Idempotent operations.
418. Pending-write recovery.
419. Retry-safe workflow steps.
420. Long-running execution.

LangGraph, for example, treats checkpointing as the basis for memory, human interruption, fault tolerance and state replay. ([Docs by LangChain][18])

---

# 39. Human-in-the-loop at the correct places

Human involvement should be selective.

421. Ask humans about ambiguous requirements.
422. Ask humans before expensive irreversible actions.
423. Ask humans when confidence stays low.
424. Ask humans after repeated repair failure.
425. Let humans edit proposed tool arguments.
426. Let humans approve/reject actions.
427. Resume automatically afterward.

This is much better than either extreme:

**human approves everything** or **human approves nothing**.

---

# 40. Observation and tracing

If you cannot see why the model failed, you cannot improve the harness.

Track:

428. Prompts.
429. Context composition.
430. Tool calls.
431. Tool arguments.
432. Tool results.
433. Token counts.
434. Latency.
435. Errors.
436. Retry counts.
437. State transitions.
438. Model selections.
439. Validation outcomes.
440. Cost.
441. Final outcome.

OpenAI's Agents SDK and other runtimes make tracing a first-class part of agent orchestration for exactly this reason. ([OpenAI][19])

---

# 41. Transcript replay

Save complete trajectories.

Then you can:

442. Reproduce failures.
443. Compare harness versions.
444. Test different models on identical state.
445. Re-run from checkpoints.
446. Find tool-selection mistakes.
447. Find context pollution.
448. Find premature termination.
449. Find bad instructions.

This is essential harness engineering infrastructure.

---

# 42. Harness evaluations

Do not evaluate only the model.

Evaluate:

```text
model + prompt + tools + context system + runtime + workflow
```

Anthropic recommends building realistic evaluation suites for tools and examining complete agent trajectories, while OpenAI has explicitly emphasized that the harness itself can alter observed model capability. ([Anthropic][6])

You need:

450. Realistic tasks.
451. Held-out tasks.
452. Multi-step tasks.
453. Adversarial tasks.
454. Regression tests.
455. Exact verifiers where appropriate.
456. Semantic verifiers where necessary.
457. Tool-call metrics.
458. Token metrics.
459. Latency metrics.
460. Failure taxonomy.

---

# 43. Optimize the harness per model

This one is critical for what you're talking about with Qwen.

Do not assume:

```text
best harness for Opus
=
best harness for Qwen
```

Tune:

461. System prompt length.
462. Number of tools.
463. Tool naming.
464. Tool schema complexity.
465. Context size.
466. Repository map size.
467. Planning depth.
468. Retry count.
469. Temperature.
470. Number of candidates.
471. Output format.
472. Tool-result format.
473. Chunk size.
474. Summarization level.
475. Maximum trajectory length.

Anthropic has observed that even relatively small changes such as tool naming conventions and response formats can have model-dependent effects. ([Anthropic][6])

**Harness-model co-design** is probably where a lot of future performance will come from.

---

# 44. Adaptive inference budgets

Easy request:

```text
one model call
```

Hard request:

```text
plan
→ retrieve
→ generate 3 candidates
→ verify
→ repair
→ verify
```

Capabilities:

476. Difficulty classification.
477. Dynamic token budget.
478. Dynamic tool budget.
479. Dynamic retry budget.
480. Dynamic candidate count.
481. Dynamic verification depth.
482. Dynamic model selection.

This avoids wasting enormous compute on easy tasks while letting hard problems receive enough search.

---

# 45. Failure-aware routing

Not every failure should cause:

> "Try again."

Classify it.

483. Retrieval failure.
484. Tool selection failure.
485. Invalid arguments.
486. Missing dependency.
487. Context insufficiency.
488. Reasoning failure.
489. Test failure.
490. Permission failure.
491. Timeout.
492. Ambiguous requirement.

Each class should trigger a different recovery path.

That is much stronger than blindly rerunning the same prompt.

---

# 46. Anti-loop mechanisms

Weak agents can easily become stuck.

Harness protections:

493. Repeated-tool-call detection.
494. Repeated-query detection.
495. Identical-error detection.
496. No-progress detection.
497. Maximum iteration count.
498. Strategy-change trigger.
499. Escalation after repeated failures.
500. Backtracking.
501. Automatic summary and replanning.

---

# 47. Anti-premature-termination mechanisms

The opposite problem also happens.

Prevent:

> "Looks good, we're done."

Require:

502. Acceptance criteria satisfied.
503. Tests passed.
504. Required artifacts exist.
505. No blocking errors.
506. Verification completed.
507. Final diff inspected.
508. Outstanding tasks empty.

Only then may the harness transition into `DONE`.

---

# 48. Grounding and retrieval

A model does not need to contain all relevant knowledge in its weights.

Give it:

509. Web search.
510. Documentation search.
511. Code search.
512. Database retrieval.
513. Knowledge graph access.
514. Vector retrieval.
515. Keyword retrieval.
516. Hybrid retrieval.
517. Source ranking.
518. Citation metadata.

This can enormously reduce the intelligence advantage of huge models when the problem is primarily knowledge access rather than reasoning.

---

# 49. Source quality control

Retrieval alone is insufficient.

The harness should:

519. Prefer primary sources.
520. Rank trustworthy domains.
521. Check dates.
522. Detect duplicates.
523. Detect contradictions.
524. Fetch original sources.
525. Preserve citations.
526. Distinguish retrieved fact from model inference.

This greatly improves research quality even with modest models.

---

# 50. Prompt hierarchy and modular instructions

Avoid one gigantic prompt.

Separate:

527. Core behavior.
528. Tool rules.
529. Project instructions.
530. Task instructions.
531. Skill instructions.
532. Current context.
533. Retrieved evidence.
534. Output contract.

Load only what applies.

This is effectively **instruction-space context management**.

---

# 51. Explicit priorities

Weaker models benefit from unambiguous priorities:

```text
1. correctness
2. don't destroy existing behavior
3. complete acceptance criteria
4. minimize unnecessary changes
5. optimize elegance
```

Without priorities, the model must infer which constraints dominate.

---

# 52. Strong defaults

Every decision you remove from the model is one less decision it can get wrong.

Defaults for:

535. Search depth.
536. Pagination.
537. Test command.
538. Build command.
539. Output format.
540. Working directory.
541. Patch style.
542. Commit behavior.
543. Timeout.
544. Retry count.

---

# 53. Semantic affordances

Tool interfaces should communicate intent.

Compare:

```text
rpc_exec_7
```

versus

```text
search_customer_transactions
```

Or:

```text
arg1
```

versus:

```text
customer_id
```

The latter effectively injects world knowledge into the interface.

This is another way harness design can increase effective intelligence without changing weights.

---

# 54. Reduce choice wherever choice has no value

For a weak model:

```text
Choose among 150 tools
```

is much harder than:

```text
Choose among:
search
read
edit
test
```

Likewise:

```text
Which of 50 possible workflows?
```

can often become:

```text
Current state permits A or B.
```

Every reduction in irrelevant branching factor makes the agent smarter.

---

# 55. Increase choice where exploration has value

Interestingly, the opposite applies at the reasoning level.

You want fewer **irrelevant actions**, but sometimes more **candidate solutions**.

So a strong harness does:

```text
narrow tool/action space
+
wide solution exploration
+
strong verification
```

That combination is extremely powerful.

---

# 56. Keep the workspace as external cognition

One of the deepest agent-harness principles is:

> **The filesystem can be part of the model's brain.**

Use files for:

545. Plans.
546. Progress.
547. Notes.
548. Research.
549. Intermediate datasets.
550. Hypotheses.
551. Test results.
552. Architecture.
553. Open questions.
554. Handoff information.

Anthropic's long-running-agent work demonstrates exactly this pattern: progress files and git history effectively function as durable external memory across context windows. ([Anthropic][10])

---

# 57. Make the environment legible to the model

This may be the best single phrase for good harness design.

The system should expose its world in a way that a language model can easily understand.

Bad:

```json
{
  "a8fz19": 7,
  "e19kq": "0x12ad",
  "status_code": 4172
}
```

Better:

```text
Deployment: failed
Reason: database migration timed out
Affected service: payments-api
Previous deployment: healthy
```

Same information.

Massively different agent usability.

---

# 58. Externalize metacognition

A strong model naturally thinks:

> What do I know?
> What am I missing?
> What should I check?
> How do I know I'm done?

A weak model often does this poorly.

So make those stages explicit:

555. Known facts.
556. Unknowns.
557. Current hypothesis.
558. Evidence needed.
559. Next experiment.
560. Result.
561. Updated hypothesis.
562. Completion test.

You're essentially putting part of executive function into software.

---

# 59. Externalize executive function

More broadly, harnesses can supply the model with the cognitive functions it lacks:

| Cognitive function     | Harness replacement     |
| ---------------------- | ----------------------- |
| memory                 | persistent state        |
| attention              | context retrieval       |
| planning               | planner/workflow        |
| inhibition             | guardrails              |
| working memory         | scratchpad/files        |
| perception             | search/browser/vision   |
| calculation            | code/calculator         |
| error detection        | validators/tests        |
| learning from mistakes | retry/reflection memory |
| task switching         | orchestrator            |
| long-term organization | structured workspace    |
| self-monitoring        | progress tracker        |
| uncertainty management | confidence/escalation   |

This is why the model/harness distinction becomes blurry in practice.

---

# 60. Harnesses should turn intelligence problems into engineering problems

This is the overarching rule behind almost everything above.

Instead of asking:

> Can the model remember which files it modified?

Track the files.

Instead of:

> Can the model determine whether its JSON is valid?

Validate the JSON.

Instead of:

> Can the model remember all 600 tools?

Search tools dynamically.

Instead of:

> Can the model tell whether the website works?

Run the website and test it.

Instead of:

> Can the model remember what happened yesterday?

Write progress state.

Instead of:

> Can the model choose the right 200 lines from a million-line repository?

Build search and repository maps.

Instead of:

> Can the model recover from destroying the implementation?

Use git and checkpoints.

Instead of:

> Can the model mentally process 100,000 database rows?

Use SQL.

Instead of:

> Can the model know whether its answer is right?

Build a verifier.

That is what a serious harness does.

---

# What matters most specifically for weaker/local models

If I were building a harness specifically to make something like a Qwen-class local model punch dramatically above its raw-chat capability, I would prioritize these roughly in this order:

**Tier S — enormous impact**

1. Excellent search/read/edit/run-test tools.
2. Deterministic outer workflow.
3. Aggressive context management.
4. Persistent task/progress state.
5. Automatic verification.
6. Automatic repair loops.
7. Narrow, extremely clear tool interfaces.
8. Structured outputs.
9. Task decomposition.
10. Explicit acceptance criteria.
11. Checkpoints/rollback.
12. Code execution for deterministic processing.
13. Repository/environment maps.
14. Retrieval rather than giant contexts.
15. Model-specific harness tuning.

**Tier A — major additional capability**

16. Skills/progressive instructions.
17. Generator → critic → repair.
18. Multiple candidate generation.
19. Specialist agents.
20. Supervisor/worker architecture.
21. Dynamic tool loading.
22. Confidence-based escalation.
23. Long-running session handoffs.
24. Failure-aware recovery.
25. Anti-loop and anti-premature-completion logic.

**Tier B — production multiplier**

26. Durable execution.
27. Human approval gates.
28. Full tracing.
29. Replay.
30. Harness eval suites.
31. Source provenance.
32. Permission boundaries.
33. Sandboxing.
34. Cost/token budgeting.
35. Adaptive inference budgets.

---

# Why Claude-style harnesses can feel disproportionately strong

I would separate the factual observation from the subjective comparison.

I can't establish objectively that Claude's harness is categorically "better than everyone else's." But Anthropic's public architecture contains a striking number of the high-leverage ideas above:

**compaction + context editing + checkpoints + long-running progress state + agent skills + tool search + programmatic tool calling + MCP + filesystem/shell access + incremental workflows + explicit testing.** ([Anthropic][11])

Likewise, OpenAI's recent Codex work has moved heavily toward harness engineering: repository knowledge, strict architectural constraints, automated testing/validation, agent-accessible developer tooling, and workflows capable of taking features from reproduction through implementation and verification. ([OpenAI][4])

So I think your underlying intuition is right:

## The model determines the ceiling. The harness determines how much of that ceiling you actually reach.

And for weaker models I would modify that slightly:

## The model supplies raw cognitive primitives. The harness supplies executive function.

A weak model with:

**memory + search + tools + planning + decomposition + deterministic computation + tests + retries + rollback + verification**

is no longer meaningfully equivalent to the same model sitting behind a `/chat/completions` endpoint.

That second system is much closer to a **compound AI system**.

And that is why a properly harnessed local model can sometimes produce work that feels several model generations above what its raw chat performance would lead you to expect.

The particularly interesting research direction now is not merely **"How smart is model X?"**

It is:

> **"What is the minimum model intelligence required when the surrounding system supplies memory, executive control, search, computation, verification and recovery?"**

That question is much more important for local AI than most benchmark leaderboards make it appear.
