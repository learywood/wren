# Wren

## Introduction

Wren is an agent harness for building and running software agents.

## Platform Support

Windows is Wren's only supported platform at this time.

- Design, implement, test, and document workflows for Windows.
- Do not add macOS or Linux support unless explicitly requested.
- Do not substitute macOS or Linux results or infrastructure for Windows validation.
- Agent workflows must run unattended from a standard non-elevated Windows session. Do not require UAC approval, administrator access, or other human interaction.

## Documentation

All documentation belongs in `/docs/`:

- `/docs/principles/` — guiding principles and operating conventions.
- `/docs/architecture/` — system design and architectural decisions.
- `/docs/research/` — investigations, references, and findings.

Keep documentation close to demonstrated needs; do not add speculative documents.

## Build Workflow

Always build the complete optimized Wren installation with `cargo install-wren`. This installs the release executable and bundled extensions into Cargo's binary directory on `PATH`; do not stop at a debug or intermediate build artifact.

## Performance Backpressure

For changes that may affect startup or performance-sensitive runtime paths:

- Load the `wren-performance` skill.
- Measure the real release harness before and after the change.
- Investigate meaningful regressions with the documented profiler workflow.
- Compile profiling sites completely out of normal builds.
- Use static scopes and numeric values by default; keep text and native call stacks explicitly coarse and opt-in.
- Batch profiling around operations expected to take less than roughly 10 microseconds.
- Do not merge an unexplained performance regression.

## GitHub Issues Workflow

- Use GitHub issues for feature work, bug fixes, and other substantive code changes.
- Create a new issue before starting any new feature.
- Small conversational tasks, including adding or modifying documentation, do not require an issue.
- Do not create an issue solely to track a small conversational task.
- For issue work, keep implementation scoped to the issue and link resulting work back to it.
- Open a follow-up issue rather than silently expanding scope.
- When human verification is required, stop before merging and clearly request that verification.

### Development flow

- **Understand the issue:** Read the full issue with `gh issue view <number> --json number,title,state,author,createdAt,updatedAt,body,comments,url`; review every comment in order and treat later superseding updates as authoritative. Also read the repository guidance, relevant code, and existing documentation. Prefer this command over browser retrieval or bare `--comments`, which may query deprecated GitHub Projects fields.
- **Confirm the outcome boundary:** Keep work together when later steps are required to prove it works; split only independently valuable and verifiable prerequisites. Use code boundaries and commits for intermediate stages rather than separate issues.
- **Research before designing:** For architectural or external integrations, inspect first-party documentation and relevant reference implementations, then record findings and unresolved decisions under `/docs/research/`.
- **Define acceptance evidence:** Update the issue from the research before coding, including production-path tests, evaluations, performance checks, and human verification where applicable.
- **Implement with continuous backpressure:** Make the smallest vertical change and run relevant unit, installed functional, authenticated, and behavioral checks throughout development, not only afterward.
- **Complete the issue:** Treat a request to start work as a request to run all gates, inspect evidence, commit, open and merge the pull request, attach results, and close the issue. Do not claim completion before then.

## Communication Style

This repository uses the GPT-5.6 model family. 

Communicate with brevity: be direct, lead with the outcome, omit unnecessary recaps, and ask questions only when they unblock work.

## Behavior Style

GPT-5.6 models tend toward completionism, over-engineering, and over-specification. Counteract this deliberately:

- Make the smallest change that satisfies the task.
- Prefer the narrowest reasonable interpretation; do not infer unrequested requirements.
- Avoid speculative abstractions, extensibility, documentation, and cleanup.
- Expand specifications only when required to resolve a concrete ambiguity.
- Do not consider or describe a change as complete until it is committed.
- Do not continue beyond the requested scope. For issue work, validate, merge, and close the issue once its acceptance criteria are met.
