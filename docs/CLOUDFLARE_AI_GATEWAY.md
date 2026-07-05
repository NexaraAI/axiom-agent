# Cloudflare AI Gateway

The Cloudflare provider uses:

- Base URL: `https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1`
- Chat endpoint: `/chat/completions`
- Token source: environment variable configured by `api_token_env`
- Optional gateway header: `cf-aig-gateway-id`

Do not store tokens in `config.toml`.

## Example Config

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

## PowerShell Example

```powershell
$env:CLOUDFLARE_API_TOKEN = "your-token"
axiom chat
```

Inside chat:

```text
!provider current
!model current
hello
```

If the token is missing, Axiom prints the environment variable name you need to set. It does not print the token value.

## Troubleshooting

- `API key/token environment variable is not set`: set the env var named by `api_token_env`.
- `missing provider field: account_id`: rerun `axiom onboarding` and update Cloudflare setup.
- Axiom summarizes HTTP errors without printing request headers or tokens.
