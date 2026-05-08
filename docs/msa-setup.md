# Microsoft Account Setup

ichr authenticates Minecraft accounts through the Microsoft device-code OAuth flow. The legacy shared client ID (`00000000402b5328`) that several older third-party launchers used is no longer accepted by Microsoft's consumer tenant (`AADSTS700016: Application ... was not found in the directory 'Microsoft Accounts'`).

You need to register your own Azure AD app and point ichr at its client ID via the `ICHR_MSA_CLIENT_ID` environment variable.

## One-time Azure AD registration

1. Sign in to the [Azure Portal](https://portal.azure.com/) with any Microsoft account. A free personal account works; you don't need a paid Azure subscription.
2. Search for **App registrations** → **New registration**.
3. Fill in:
   - **Name:** anything (e.g., `ichr-<yourhandle>`).
   - **Supported account types:** choose **Personal Microsoft accounts only**.
   - **Redirect URI:** leave blank — device-code flow doesn't use one.
4. Click **Register**. The overview page now shows an **Application (client) ID** — a GUID like `11111111-2222-3333-4444-555555555555`.
5. In the left nav: **Authentication** → scroll to **Advanced settings** → set **Allow public client flows** to **Yes** → **Save**.

## Point ichr at your app

Easiest — create a `.env` file in the project root. ichr auto-loads it at startup via `dotenvy`:

```bash
echo 'ICHR_MSA_CLIENT_ID=11111111-2222-3333-4444-555555555555' > .env
cargo run --release
```

`.env` is gitignored by default.

Alternative — export in the shell (no `.env` file):

```bash
# Linux / macOS
export ICHR_MSA_CLIENT_ID=11111111-2222-3333-4444-555555555555
cargo run --release
```

```powershell
# Windows PowerShell
$env:ICHR_MSA_CLIENT_ID = "11111111-2222-3333-4444-555555555555"
cargo run --release
```

Persist the shell export in your rc file (`~/.bashrc`, `~/.zshrc`, etc.) or Windows user environment if you want it across sessions without `.env`.

## Verify

In ichr: press `A` → `a` → the device-code modal should now show a `user_code` and `https://microsoft.com/link` URL. Visit the URL in a browser, enter the code, sign in with your Minecraft-licensed Microsoft account. The account should appear in the list when the chain completes.

## Security notes

- The client ID is not a secret — it identifies your app but carries no authority on its own. Safe to commit / share.
- Your refresh and access tokens live either in your OS keychain (libsecret / DPAPI) or in an AES-256-GCM encrypted file at `~/.config/ichr/accounts.enc` (Linux) or `%APPDATA%\ichr\accounts.enc` (Windows). The encryption key is derived from your machine identifier — tokens will not decrypt on a different machine.
- ichr never logs token values. Tracing spans on auth functions use `skip_all` so nothing sensitive reaches the log file.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Microsoft rejected the default client ID` | You haven't set `ICHR_MSA_CLIENT_ID` yet, or the variable isn't visible to the launcher. Verify with `echo $ICHR_MSA_CLIENT_ID` (Linux) / `$env:ICHR_MSA_CLIENT_ID` (PowerShell). |
| `AADSTS7000218: The request body must contain the following parameter: 'client_assertion' or 'client_secret'` | In Azure Portal → **Authentication** → set **Allow public client flows** to **Yes**. |
| `AADSTS50020: User account ... from identity provider 'live.com' does not exist in tenant` | In the registration step, you picked "Single tenant" or "Multi-tenant" instead of "Personal Microsoft accounts only". Create a new registration or edit **Manifest → signInAudience → "PersonalMicrosoftAccount"**. |
| XSTS rejects with `XErr 2148916233` | The signed-in MS account has no Xbox profile. Create one at <https://www.xbox.com/profile> and retry. |
| `AccountAuthFailed` with any other XErr code | Known codes (child account, country restriction, etc.) are translated into readable messages by `src/auth/xsts_errors.rs`. If you hit an unknown code, please open an issue with the raw `XErr` value. |
