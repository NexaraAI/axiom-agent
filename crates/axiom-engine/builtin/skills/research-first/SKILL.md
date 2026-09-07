---
name: research-first
description: Strict autonomous agent reasoning protocol requiring thorough online research and documentation inspection before taking action, proposing plans, or answering questions on unfamiliar domains, platforms, frameworks, or APIs.
---

# Research First (Ground Knowledge Before Acting)

This skill establishes the foundational operating directive of Axiom Agent: **Always research and ground external facts before taking action, generating code, proposing plans, or answering domain-specific inquiries.**

---

## 1. Core Principles

1. **Investigate Before Acting**:
   Never guess, speculate, or hallucinate external options, library APIs, package names, configuration schemas, or platform distribution rules. If the task touches third-party ecosystems (e.g. game modding distribution, cloud APIs, framework migrations), search the web and inspect authoritative sources first.

2. **Grounding via `web.fetch`**:
   Use `web.fetch` with a focused `query` parameter (e.g. `{"query": "minecraft mod and plugin publishing platforms"}`) to search online documentation, articles, and directories. Review the retrieved text, extract authoritative facts, and base your solution on verified reality.

3. **No Blind Local Scans**:
   Do not run `project.scan` or search local workspace files when the user's prompt is asking about external platforms, strategy, community management, or theoretical concepts. Local inspection tools (`project.scan`, `file.read`) are for code implementation tasks within the repository, not for external knowledge retrieval.

4. **Direct, Authoritative Answers**:
   Once research is completed, provide a direct, comprehensive, and well-structured answer in chat. Do not stall or interrogate the user with unnecessary multiple-choice questions when you have the facts to answer them thoroughly.

---

## 2. When to Trigger Research

Activate research immediately whenever:
- The user asks where to publish, distribute, or host artifacts (e.g. Minecraft plugins, crates, npm packages, Docker containers).
- The user asks about modern alternatives, comparisons, or ecosystem recommendations.
- The task requires using an unfamiliar or rapidly evolving API or library.
- The user provides a URL or mentions an external documentation site.
- An error message references an upstream service or unknown error code.

---

## 3. Workflow: Research -> Synthesize -> Deliver

```
  User Request
       │
       ▼
 [Is external knowledge needed?]
    ├── YES ──► Call `web.fetch` with focused query/URL
    │                 │
    │                 ▼
    │           Synthesize verified facts
    │                 │
    │                 ▼
    └── NO  ──► Deliver direct, concrete answer / execute action
```
