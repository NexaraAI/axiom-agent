# Providers

Axiom streams OpenAI-compatible chat completions and supports tool calling through hosted and local providers. The onboarding command includes first-class presets:

| Preset | Endpoint | API key environment variable | Default model |
| --- | --- | --- | --- |
| `groq` | `https://api.groq.com/openai/v1` | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| `openrouter` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` | `openrouter/free` |
| `gemini` | `https://generativelanguage.googleapis.com/v1beta/openai` | `GEMINI_API_KEY` | `gemini-2.5-flash` |
| `github-models` | `https://models.github.ai/inference` | `GITHUB_TOKEN` | `openai/gpt-4.1` |
| `nvidia` | `https://integrate.api.nvidia.com/v1` | `NVIDIA_API_KEY` | `meta/llama-3.3-70b-instruct` |
| `openai` | `https://api.openai.com/v1` | `OPENAI_API_KEY` | required with `--model` |
| `ollama` | `http://localhost:11434/v1` | none | `llama3.2` |
| `lm-studio` | `http://localhost:1234/v1` | none by default | required with `--model` |

`openai-compatible` remains an alias for `openai`, `lmstudio` remains an alias for `lm-studio`, and `nvidia-nim` remains an alias for `nvidia`.

## Quick setup

For the friendliest setup, run `axiom onboarding`. Choose one provider or two
comma-separated providers. Axiom accepts credentials through hidden input,
stores them in the Windows Credential Manager, macOS Keychain, or Linux Secret
Service, fetches the provider's model catalog when that provider exposes a
documented catalog endpoint, and lets you search or select a model. Environment
variables remain supported for servers, CI, and systems without a desktop
credential store.

Model discovery makes a catalog `GET` request only; it does not send a prompt
or create a chat/completion request. Provider API rate limits can still apply.

Groq:

```powershell
$env:GROQ_API_KEY = "your-key"
axiom onboarding --non-interactive --provider groq --workspace . --yes
axiom chat
```

OpenRouter's rotating free-model router:

```powershell
$env:OPENROUTER_API_KEY = "your-key"
axiom onboarding --non-interactive --provider openrouter --workspace . --yes
axiom chat
```

NVIDIA NIM's hosted API:

```powershell
$env:NVIDIA_API_KEY = "your-key"
axiom onboarding --non-interactive --provider nvidia --workspace . --yes
axiom chat
```

Ollama, running locally without an API key:

```powershell
ollama pull llama3.2
axiom onboarding --non-interactive --provider ollama --workspace . --yes
axiom chat
```

LM Studio, after starting its local server and loading a model:

```powershell
axiom onboarding --non-interactive --provider lm-studio --model your-loaded-model-id --workspace . --yes
axiom chat
```

Pass `--model <id>` to override any preset default. Provider catalogs and
free-tier limits change over time, so use a model ID available to your account.
“Free” hosted options can still require an account and API key and are subject
to rate, daily-use, regional, and availability limits. Catalog output is capped
at 100 matching IDs; use `axiom model list --filter <text>` or
`!model list <text>` to narrow it.

Run `axiom doctor` (or `axiom doctor --json`) to verify the active provider,
model, endpoint configuration, and whether the expected credential is available
from the environment or native credential manager. Diagnostics report only the
variable name and source, never its value.

Change providers and models without rerunning onboarding:

```powershell
axiom provider list
axiom provider use openrouter
axiom model list --filter free
axiom model use openrouter/free
axiom model list --provider ollama
```

GitHub Models uses a personal access token with `models:read` permission. Every account currently receives included rate-limited usage; paid use is a separate opt-in and the service remains subject to GitHub's preview terms.

## Custom OpenAI-compatible endpoint

Point Axiom at a service accepting OpenAI-style `/chat/completions` requests.
Remote completion and catalog URLs must use HTTPS. Plain HTTP is accepted only
for `localhost` or a literal loopback address, which keeps Ollama, LM Studio,
and other trusted local development servers convenient. `api_key_env` is
optional for local servers that do not require authentication.

```toml
[llm]
active_provider = "local"
active_model = "your-model-id"
stream = true

[providers.local]
type = "openai_compatible"
base_url = "http://localhost:8000/v1"
models_url = "http://localhost:8000/v1/models" # optional override
```

For an authenticated endpoint, add only the environment variable name to config:

```toml
api_key_env = "LOCAL_LLM_API_KEY"
```

```powershell
$env:LOCAL_LLM_API_KEY = "your-api-key"
axiom chat
```

## Cloudflare AI Gateway

Cloudflare's unified REST API currently documents chat inference and a web
model catalog, but not an authenticated account-wide `GET /models` contract
covering every third-party model. Axiom therefore treats Cloudflare as an
explicit-model provider: onboarding asks for a model ID and does not issue an
inference request while setting it up.

```toml
[llm]
active_provider = "cloudflare"
active_model = "openai/gpt-4.1-mini"
stream = true

[providers.cloudflare]
type = "cloudflare_ai_gateway"
account_id = "YOUR_ACCOUNT_ID"
gateway_id = "default"
api_token_env = "CLOUDFLARE_API_TOKEN"
base_url = "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1"
```

```powershell
$env:CLOUDFLARE_API_TOKEN = "your-token"
axiom chat
```

## Security notes

- Axiom stores environment variable names, never provider tokens, in `config.toml`.
- Axiom resolves an environment or native-keyring credential directly into the
  selected provider's HTTP client. It does not hydrate the process environment
  with keyring values.
- Every configured provider credential variable is removed from test,
  diagnostic, and Git child-process environments, including credentials for
  inactive providers. Git diff also disables external diff and textconv drivers.
- Custom credential-variable names must use normal identifier syntax and cannot
  replace process-control variables such as `PATH`, proxy variables,
  `LD_PRELOAD`, or `AXIOM_HOME`.
- Do not expose an unauthenticated local model server to an untrusted network.
- Provider URLs reject embedded credentials, query strings, and fragments.
  Provider HTTP clients do not follow redirects, so a configured bearer token
  or prompt cannot be forwarded to an unexpected endpoint.
- Provider completion/catalog endpoints are separate from `[network]`
  `web.fetch` controls. This permits explicitly configured local Ollama or LM
  Studio endpoints without allowing the model-invoked web tool to access local
  or private networks.
- Provider HTTP clients ignore system proxy settings, preventing explicit
  provider bearer credentials and prompts from being forwarded through an
  ambient proxy.
- Streaming and request parsing are covered without making live provider calls in CI.

Provider references: [Groq OpenAI compatibility](https://console.groq.com/docs/openai), [OpenRouter quickstart](https://openrouter.ai/docs/quickstart), [OpenRouter free router](https://openrouter.ai/docs/guides/routing/routers/free-router), [Gemini OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai), [GitHub Models quickstart](https://docs.github.com/en/github-models/quickstart), [NVIDIA NIM LLM APIs](https://docs.api.nvidia.com/nim/reference/llm-apis), [NVIDIA Llama 3.3 endpoint](https://build.nvidia.com/meta/llama-3_3-70b-instruct), [Cloudflare AI Gateway REST API](https://developers.cloudflare.com/ai-gateway/usage/rest-api/), [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility), and [LM Studio local server](https://lmstudio.ai/docs/developer/core/server).
