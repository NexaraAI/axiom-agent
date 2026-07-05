# Providers

Axiom supports non-streaming chat completions for:

- OpenAI-compatible endpoints.
- Cloudflare AI Gateway.

## OpenAI-Compatible

Point Axiom at any endpoint that accepts OpenAI-style `/chat/completions` requests.

```toml
[llm]
active_provider = "local"
active_model = "llama-3.1-8b-instruct"
stream = true

[providers.local]
type = "openai_compatible"
base_url = "http://localhost:8000/v1"
api_key_env = "LOCAL_LLM_API_KEY"
```

Windows PowerShell:

```powershell
$env:LOCAL_LLM_API_KEY = "your-api-key"
axiom chat
```

## Cloudflare AI Gateway

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

Windows PowerShell:

```powershell
$env:CLOUDFLARE_API_TOKEN = "your-token"
axiom chat
```

## Notes

- Axiom reads API keys and tokens from environment variables.
- Axiom does not store tokens in `config.toml`.
- Streaming is not implemented yet; chat uses non-streaming responses.
- CI tests cover request construction and response parsing without making real API calls.
