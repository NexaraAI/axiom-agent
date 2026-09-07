---
name: github-research
description: Deep GitHub inspection protocol to search repositories, inspect organizations, extract release histories, and analyze code architectures via GitHub API.
---

# GitHub Research & Inspection Protocol

This skill directs Axiom Agent to inspect public GitHub organizations, repositories, architectures, releases, and documentation to understand any open-source or community software portfolio.

---

## 1. Capabilities

1. **Organization Audit**:
   - Inspect public repositories of an organization or user (`{"org": "<name>", "type": "org_repos"}`).
   - Discover active projects, languages, stargazers, forks, and recent updates.

2. **Repository Architecture Inspection**:
   - Query repository details (`{"repo": "<owner/repo>", "type": "repo_detail"}`).
   - Retrieve full README documentation (`{"repo": "<owner/repo>", "type": "readme"}`).
   - Inspect releases and changelogs (`{"repo": "<owner/repo>", "type": "releases"}`).

3. **Global Repository Discovery**:
   - Search across GitHub by keyword (`{"query": "<keyword>", "type": "repos"}`).

---

## 2. Research Best Practices

- Always start by listing the repositories of the target organization or user to understand their actual projects before making assumptions.
- Look at project descriptions, primary languages, and dependencies.
- Cross-reference GitHub findings with other distribution hubs (Modrinth, Hangar, Maven, Crates.io, NPM) to see the full operational picture.
