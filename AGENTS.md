# Agent Directives for handlr-gui

## Code Philosophy

Write straightforward, simple, short code. Prefer obvious solutions over clever ones. Do not write overly defensive code focused on unlikely scenarios that will never happen in practice. Trust internal code and library guarantees. Validate only at real system boundaries (user input, file I/O, IPC). Three clear lines beat a premature abstraction.

## Subagent Policy

The main agent **must** launch subagents for:
- Any research task that requires reading more than a handful of files
- Any implementation task that is non-trivial (more than ~20 lines of new code)
- Any debugging session that requires systematic investigation

The main agent's context must stay clean. Delegate aggressively. Subagents are cheap; polluted context is expensive.

Subagent types to use:
- `Explore` — codebase research, file reading, pattern finding
- `general-purpose` — implementation tasks with write access
- `superpowers:code-reviewer` — code review

## Review Process

After each larger implementation event (a feature, a module, a significant refactor):

1. Launch a **reviewer subagent** (`superpowers:code-reviewer`) with full context of what was implemented and the relevant plan section.
2. The reviewer returns findings (issues, improvements, anything off-spec).
3. Launch an **implementation subagent** that receives both the original code and the reviewer's findings, and applies any necessary fixes.
4. Repeat this review loop **twice** per implementation event before moving on.

This loop is mandatory for any implementation larger than a single small function.
