# Microsoft Account Setup

**End users do not need this document.** ichr ships with an embedded
production AppID (Mojang-approved on 2026-05-08). Press `A` -> `a` in the
launcher and the device-code flow handles the rest.

This document is for **forks and downstream redistributions**. Per the
Minecraft Usage Guidelines each launcher distribution must use its own
approved AppID; reusing the upstream ichr AppID for a fork attributes
that fork's traffic to ichr and risks the AppID being revoked
project-wide.

## When you need your own AppID

- You're maintaining a fork that you intend to distribute.
- You're publishing a derivative product that calls Microsoft / Mojang
  auth on behalf of users.
- You want to test against a separate Azure AD tenant during development.

If you're just running ichr as a user, stop here. The default AppID
works.

## One-time Azure AD registration

1. Sign in to the [Azure Portal](https://portal.azure.com/) with any
   Microsoft account. A free personal account works; you don't need a
   paid Azure subscription.
2. Search for **App registrations** -> **New registration**.
3. Fill in:
   - **Name:** something distinctive for your fork (e.g.,
     `myfork-launcher`). Microsoft shows this on the user consent
     screen, so make it clear which app the user is authorizing.
   - **Supported account types:** choose **Personal Microsoft accounts
     only**.
   - **Redirect URI:** leave blank -- device-code flow doesn't use one.
4. Click **Register**. The overview page now shows an **Application
   (client) ID** -- a GUID like
   `11111111-2222-3333-4444-555555555555`.
5. In the left nav: **Authentication** -> scroll to **Advanced
   settings** -> set **Allow public client flows** to **Yes** ->
   **Save**.

## Apply for Mojang approval

Microsoft requires that any AppID hitting `api.minecraftservices.com` be
on Mojang's allow list. Submit your AppID through the [Microsoft
Minecraft auth approval form](https://aka.ms/mce-reviewappid). Mojang
Enforcement typically replies within 1-3 business days; their email
explicitly states that they handle AppID grants only and will not
provide API consultation.

Until your AppID is approved you'll see `XSTS` failures (typically
`XErr 2148916233`) at the Xbox -> Minecraft step of the chain.

## Point ichr at your app

Easiest -- create a `.env` file in the project root. ichr auto-loads it
at startup via `dotenvy`:

```bash
echo 'ICHR_MSA_CLIENT_ID=11111111-2222-3333-4444-555555555555' > .env
cargo run --release
```

`.env` is gitignored by default.

Alternative -- export in the shell (no `.env` file):

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

For a shipped binary fork, replace `DEFAULT_MSA_CLIENT_ID` in
`src/auth/device_code.rs` so end users don't need the env var.

## Verify

In ichr: press `A` -> `a` -> the device-code modal should now show a
`user_code` and `https://microsoft.com/link` URL. Visit the URL in a
browser, enter the code, sign in with your Minecraft-licensed Microsoft
account. The account should appear in the list when the chain
completes.

## Security notes

- The client ID is not a secret -- it identifies your app but carries
  no authority on its own. Safe to commit / share. Public-client
  AppIDs (no `client_secret`) are explicitly designed for this.
- Your refresh and access tokens live either in your OS keychain
  (libsecret / DPAPI) or in an AES-256-GCM encrypted file at
  `~/.config/ichr/accounts.enc` (Linux) or `%APPDATA%\ichr\accounts.enc`
  (Windows). The encryption key is derived from your machine identifier
  -- tokens will not decrypt on a different machine.
- ichr never logs token values. Tracing spans on auth functions use
  `skip_all` so nothing sensitive reaches the log file.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Microsoft rejected the configured client ID` | If you set `ICHR_MSA_CLIENT_ID`, verify the GUID matches the Azure AD app you registered. If you didn't override, the default AppID was revoked -- open an issue. |
| `AADSTS7000218: The request body must contain the following parameter: 'client_assertion' or 'client_secret'` | In Azure Portal -> **Authentication** -> set **Allow public client flows** to **Yes**. |
| `AADSTS50020: User account ... from identity provider 'live.com' does not exist in tenant` | In the registration step, you picked "Single tenant" or "Multi-tenant" instead of "Personal Microsoft accounts only". Create a new registration or edit **Manifest -> signInAudience -> "PersonalMicrosoftAccount"**. |
| XSTS rejects with `XErr 2148916233` | Either the signed-in MS account has no Xbox profile (create one at <https://www.xbox.com/profile>) or your AppID isn't on Mojang's allow list yet. |
| `AccountAuthFailed` with any other XErr code | Known codes (child account, country restriction, etc.) are translated into readable messages by `src/auth/xsts_errors.rs`. If you hit an unknown code, please open an issue with the raw `XErr` value. |
