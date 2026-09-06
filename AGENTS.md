# Agent Orchestration

## Delegation policy

When the user asks Sol to plan work and delegate implementation to Luna, treat
the delegation as a bounded handoff.

### Sol responsibilities

Use Sol only for:

- architecture and planning;
- ambiguous, high-risk, or security-sensitive decisions;
- resolving blockers that Luna cannot resolve independently;
- final integration, review, and judgment.

Before delegating, Sol must give Luna one large, well-specified assignment when
the work is low-risk and mechanically executable. The assignment must include:

- the objective;
- relevant files and scope;
- constraints and non-goals;
- the implementation plan;
- acceptance criteria;
- tests and validation commands to run;
- the expected return format, including the final diff summary, validation
  evidence, and unresolved issues.

Give Luna only the task-specific context required for the assignment. Do not
fork or reproduce Sol's full reasoning context when a narrower context is
sufficient.

### Bounded handoff behavior

After delegation, Sol must yield control. Here, yielding means waiting for completion or a blocker through the runtime, not ending the user task at dispatch. The final-review duties below still apply; no new user "continue" is required. A runtime-reported failure, cancellation, or disconnection also permits resuming to handle that blocker. Sol must not continuously monitor,
poll, inspect, or re-evaluate Luna's intermediate progress, and must not spend
reasoning tokens duplicating work already assigned to Luna.

Resume Sol reasoning only when one of these conditions occurs:

- Luna reports that the delegated task is complete;
- Luna reports a blocker requiring Sol-level judgment;
- a review checkpoint explicitly defined before delegation is reached;
- Luna's completed result fails validation and requires replanning.

Do not create implicit checkpoints. Avoid back-and-forth assignments when Luna
can complete the work independently in one handoff.

### Luna responsibilities

Luna should implement the plan, debug its own changes, run the specified tests,
and fix ordinary implementation problems without waking Sol. Luna should
resolve these independently:

- ordinary compiler and type errors;
- lint and formatting failures;
- straightforward test failures caused by its own patch;
- import, module, and path issues;
- minor implementation choices consistent with the plan;
- repetitive or mechanical edits;
- documentation updates required by the implementation;
- local refactors required to complete the implementation safely.

Luna should escalate only for:

- architecture changes;
- conflicting or materially ambiguous requirements;
- destructive migrations or irreversible operations;
- security-sensitive choices;
- major scope expansion;
- repeated failure after reasonable independent debugging attempts.

On completion, Luna must return:

- a concise summary of files and behavior changed;
- tests and validation commands run, with their results;
- any unresolved issues, risks, or assumptions requiring review.

### Final review

When Luna completes the handoff, Sol should review the final diff and Luna's
validation evidence. Sol should not reconstruct or replay Luna's entire
execution process. If correction is needed, prefer one consolidated correction
request followed by final validation.

This is an orchestration policy, not a guarantee that the runtime will consume
zero Sol tokens while Luna is working. Minimize Sol usage through immediate
yielding, coarse-grained assignments, task-specific context, and few explicit
checkpoints.
