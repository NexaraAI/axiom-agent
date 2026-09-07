---
name: humanized-codes
description: Strict engineering directives for writing clean, humanized, self-documenting code without AI boilerplate, redundant comments, or telltales, enforcing sound architecture, robust type contracts, and automatic post-change linting and formatting.
---

# Humanized Clean Codes (Engineering Standard)

This skill governs the craftsmanship, style, and architectural rigor required when authoring, modifying, or refactoring code. It ensures that all code reads as if written by a seasoned principal software engineer: clean, elegant, modern, resilient, and entirely devoid of AI telltales and trivial commentary.

---

## 1. Core Directives

1. **Zero AI-Telltale Comments**: Never write redundant, trivial, conversational, or echo comments. Code must be self-documenting.
2. **Architectural & Type Rigor**: Design strong types, enforce explicit method contracts, and make illegal states unrepresentable.
3. **Automated Linting & Formatting**: Every code edit must be automatically formatted and linted with zero warnings before completion.

---

## 2. Comment Discipline: The "Why", Never the "What"

### ❌ Prohibited Comment Types (AI Telltales)

Do **NOT** write any of the following:

- **Echo / Obvious Comments**:
  ```typescript
  // BAD: Redundant narration of what the code already clearly states
  // Initialize user id
  const userId = req.params.id;

  // Loop over users
  for (const user of users) { ... }

  // Return the result
  return result;
  ```
- **Helper / Section Markers**:
  ```rust
  // BAD: Meaningless section demarcations
  // ==========================================
  // Helper functions
  // ==========================================
  fn compute_hash() { ... }
  ```
- **Trivial Docstrings**:
  ```typescript
  // BAD: Docstrings that simply rephrase function and parameter names
  /**
   * Gets the user profile by user id.
   * @param userId The user id
   * @returns The user profile
   */
  function getUserProfile(userId: string): Promise<UserProfile>
  ```
- **Conversational / Apologetic Commentary**:
  ```python
  # BAD: AI conversationalisms
  # In this step we filter active sessions to ensure validity
  # Note: You might want to optimize this later
  ```

### ✅ Permitted Comments (The "Why" Only)

Comments are reserved exclusively for non-obvious reasoning that cannot be expressed through code and types alone:

- **Complex Business / Domain Rules**: Documenting non-intuitive domain requirements or regulatory constraints.
  ```rust
  // Transactions finalized after 17:00 UTC settle on the next banking business day (Rule SEC-402).
  ```
- **Compiler / Hardware / Upstream Workarounds**: Explaining workarounds for known upstream bugs with issue links.
  ```typescript
  // Workaround for https://github.com/nodejs/node/issues/XXXXX:
  // Stream buffer must be flushed before calling destroy() on Windows.
  ```
- **Algorithmic Invariants & Non-Trivial Concurrency**: Documenting invariants or memory barrier semantics.
  ```rust
  // Acquire-Release ordering guarantees that changes to buffer slots are visible
  // to worker threads before the ready bitmask is updated.
  ```
- **Required File Headers**: Copyright, SPDX license identifiers, and shebang lines.

### 💡 Writing Self-Documenting Code

Instead of explaining code with comments, refactor the code to explain itself:

1. **Intention-Revealing Identifiers**:
   - `is_eligible_for_instant_payout` instead of `valid` or `flag`.
   - `retry_backoff_duration` instead of `delay` or `t`.
2. **Decompose Complex Boolean Logic**:
   ```typescript
   // BAD
   if (user.age >= 18 && user.kycStatus === 'VERIFIED' && !user.isSuspended) { ... }

   // GOOD: Extract expressive predicates
   const canInitiateTransfer = user.isAdult && user.isKycVerified && !user.isSuspended;
   if (canInitiateTransfer) { ... }
   ```
3. **Small, Single-Responsibility Functions**: Keep functions focused so their purpose is immediately obvious from their signature.

---

## 3. Architectural Rigor, Type Soundness & Contracts

### 1. Make Illegal States Unrepresentable
Leverage the language's type system (sum types, tagged unions, sealed interfaces) to eliminate invalid combinations of state.

```typescript
// BAD: Impossible states can be represented (e.g. status === 'success' but error is defined)
interface ApiResponse<T> {
  status: 'loading' | 'success' | 'error';
  data?: T;
  error?: Error;
}

// GOOD: Tagged union / Sum type
type ApiResponse<T> =
  | { readonly status: 'loading' }
  | { readonly status: 'success'; readonly data: T }
  | { readonly status: 'error'; readonly error: Error };
```

```rust
// In Rust: Prefer explicit enums with payloads over structs full of Option<T>
pub enum SessionState {
    Unauthenticated { challenge: [u8; 32] },
    Authenticated { session_id: SessionId, user_id: UserId },
    Expired { expired_at: Timestamp },
}
```

### 2. Parse, Don't Validate
Do not validate data repeatedly deep in business logic. Validate and parse input once at the system boundary into rich, strongly typed domain values.

- Use Newtypes / Branded Types: `EmailAddress`, `UserId`, `PositiveInt` instead of primitive `string` or `number`.
- If an entity exists inside the domain core, its invariants are already proven by its type.

### 3. Strict Method Contracts
- **Explicit Invariants**: Clearly define pre-conditions, post-conditions, and failure modes.
- **Fail Fast & Explicitly**: Use `Result<T, E>` or typed domain errors. Never swallow exceptions or return ambiguous `null` when a specific error reason exists.
- **Pure Core, Impure Perimeter**: Keep core domain calculations pure and deterministic; push I/O, network calls, and time lookups to the perimeter.

---

## 4. Mandatory Post-Edit Linting & Formatting Pipeline

Whenever you write, edit, or refactor code, you **must** execute the appropriate formatter and linter for the target environment before presenting the solution or concluding the task.

### Ecosystem Commands Reference

| Ecosystem | Formatter Command | Linter / Static Analysis Command |
|---|---|---|
| **Rust** | `cargo fmt` | `cargo clippy --all-targets -- -D warnings` |
| **TypeScript / JS** | `npx prettier --write <files>` | `npx eslint --fix <files>` |
| **Python** | `ruff format <files>` | `ruff check --fix <files>` |
| **Go** | `gofmt -w <files>` | `golangci-lint run` |

### Zero Warning Policy
- Code must compile and pass linters with **zero warnings**.
- If a linter flags an issue, fix the root cause rather than adding ignore/suppression annotations unless an explicit upstream architectural justification exists.

---

## 5. Pre-Completion Review Checklist

Before finishing any code change, verify:
- [ ] Are all redundant, trivial, or echo comments removed?
- [ ] Are variable and function names self-describing and domain-aligned?
- [ ] Are edge cases handled with explicit types rather than defensive checks?
- [ ] Have you run the formatter and verified consistent layout?
- [ ] Have you run the linter and verified zero errors and zero warnings?
- [ ] Does the code feel natural, elegant, and maintainable by human peers?
