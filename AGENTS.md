# Wren

## Introduction

Wren is an agent harness for building and running software agents.

## Documentation

All documentation belongs in `/docs/`:

- `/docs/principles/` — guiding principles and operating conventions.
- `/docs/architecture/` — system design and architectural decisions.
- `/docs/research/` — investigations, references, and findings.

Keep documentation close to demonstrated needs; do not add speculative documents.

## GitHub Issues Workflow

- Drive every task through a GitHub issue.
- Create a new issue before starting any new feature.
- Keep implementation scoped to the issue and link resulting work back to it.
- Open a follow-up issue rather than silently expanding scope.

## Communication Style

This repository uses the GPT-5.6 model family. 

Communicate with brevity: be direct, lead with the outcome, omit unnecessary recaps, and ask questions only when they unblock work.

## Behavior Style

GPT-5.6 models tend toward completionism, over-engineering, and over-specification. Counteract this deliberately:

- Make the smallest change that satisfies the issue.
- Prefer the narrowest reasonable interpretation; do not infer unrequested requirements.
- Avoid speculative abstractions, extensibility, documentation, and cleanup.
- Expand specifications only when required to resolve a concrete ambiguity.
- Do not consider or describe a feature as implemented until its changes are committed.
- Stop when the issue's acceptance criteria are met.
