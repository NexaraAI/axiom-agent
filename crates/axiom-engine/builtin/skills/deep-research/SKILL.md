---
name: deep-research
description: Exhaustive autonomous multi-source deep research protocol that investigates unfamiliar entities, organizations, repositories, architectures, and platforms, synthesizing findings into a comprehensive structured Markdown research dossier.
---

# Deep Research (Multi-Source Investigation & Dossier Synthesis)

This skill directs Axiom Agent to conduct rigorous, multi-step investigation across online directories, documentation, public repositories, and registries, producing an authoritative, comprehensive analysis dossier.

---

## 1. Research Protocol

1. **Multi-Angle Investigation**:
   Never rely on a single search query or snippet. Formulate targeted queries across different facets:
   - **Identity & Footprint**: Official organization, domain, handles (e.g. GitHub org, Modrinth, X, YouTube, Discord).
   - **Technical Architecture**: Repositories, open-source code, schemas, tech stack, APIs.
   - **Ecosystem & Distribution**: Plugin platforms (Modrinth, Hangar, SpigotMC, CurseForge), registries (crates.io, npm), releases.
   - **Community & Positioning**: Active partnerships, sponsorships, community feedback, server setups.

2. **Tool Coordination**:
   - Use `web.fetch` with `{"query": "..."}` to uncover web presence, articles, and documentation.
   - Use `github.search` with `{"org": "..."}`, `{"repo": "..."}`, or `{"query": "..."}` to inspect public source code, READMEs, release tags, and repo hierarchies.
   - Fetch authoritative URLs directly using `web.fetch` with `{"url": "..."}` to extract full context.

3. **Fact Corroboration**:
   - Verify versions, active maintainers, license terms, and repository activity.
   - If an entity has multiple presences (e.g. GitHub org `DemonZ-Development`, Modrinth `demonzdevelopment`), cross-reference projects to build a unified profile.

---

## 2. Dossier Generation (`research-<topic>.md`)

When executing an in-depth audit or strategic analysis, synthesize findings into a dedicated Markdown dossier named `research-<topic>.md` in the workspace, or deliver a detailed executive report following this structure:

### Report Structure
```markdown
# [Topic / Entity Name] Comprehensive Research Dossier

## 1. Executive Summary
- Core mission, identity, and current operational footprint.
- Key strengths, assets, and strategic positioning.

## 2. Technical Ecosystem & Repositories
- Breakdown of key repositories, tools, utilities, and infrastructure.
- Language/framework stack (e.g. Java 21, Paper/Folia, Rust, TypeScript).
- Architecture quality, modern patterns, and performance characteristics.

## 3. Distribution, Platforms & Ecosystem
- Active distribution channels (Modrinth, Hangar, GitHub Releases, NPM).
- Community channels (Discord, social media, sponsor partnerships).

## 4. Competitive Analysis & Opportunities
- Market gaps and areas where modern infrastructure beats legacy bloat.
- High-leverage product opportunities (e.g. AI-driven companion networks, serverless deployers).

## 5. Strategic Roadmap & Recommended Next Steps
- Immediate high-priority actions (1-4 weeks).
- Product hardening and distribution acceleration.
```

---

## 3. Direct Execution Rules

- **Do Not Prevaricate**: When asked to research or audit an entity, immediately run research tools and synthesize real data.
- **Precision Grounding**: Quote actual repository names, plugin features, and architecture details discovered during research.
