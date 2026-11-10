# Sign and notarize the macOS app with a Developer ID

The release signs pinned ad hoc (`"signingIdentity": "-"` in `tauri.conf.json`). macOS then asks
users to approve the app by hand in Privacy & Security. A Keychain item names the exact build that
made it, and every ad-hoc update gets a new build hash, so the app keeps its hub login in the
Keychain only in a signed build. Until then the login stays in `account.json`.

A Developer ID signature and Apple's notarization fix both. Gatekeeper opens the app with no
workaround. The Keychain names the app by its bundle id and team, so updates keep reading the login
with no prompt, and signing turns the Keychain on.

`.github/workflows/release.yml` already has the step that signs and notarizes. It runs only when
the secrets in step 3 exist. Without them, the release builds ad-hoc signed, as before.

## Before you start

- An Apple Developer Program membership (99 USD a year,
  <https://developer.apple.com/programs/enroll/>). Only the account holder can create a Developer ID
  certificate.
- A Mac with Keychain Access, Xcode's command line tools (`xcode-select --install`), and `gh` signed
  in with admin rights on `FelipeGazapina/pinned`.

## 1. Create the Developer ID Application certificate

1. Open Keychain Access. Select **Keychain Access > Certificate Assistant > Request a Certificate
   From a Certificate Authority**.
2. Type your email and a common name such as `pinned Developer ID`. Leave the CA email empty. Select
   **Saved to disk**, then **Continue**. This saves `CertificateSigningRequest.certSigningRequest`
   and puts its private key in your login keychain.
3. Go to <https://developer.apple.com/account/resources/certificates/add>. Select
   **Developer ID Application**, then **Continue**.
4. Select the **G2 Sub-CA** profile type. Upload the `.certSigningRequest` file. Download the
   certificate, `developerID_application.cer`.
5. Double-click the `.cer` file. Keychain Access adds it to the login keychain, next to its key.
6. Find the signing identity:

   ```sh
   security find-identity -v -p codesigning
   ```

   The line reads `"Developer ID Application: <your name> (<TEAMID>)"`. The whole quoted text is
   `APPLE_SIGNING_IDENTITY`. The 10 characters in parentheses are your team ID.
7. In Keychain Access, open **login > My Certificates**. Expand the Developer ID certificate and make
   sure a private key is under it. Right-click the certificate, select **Export**, and save
   `DeveloperID.p12` with a new password. That password is `APPLE_CERTIFICATE_PASSWORD`.
8. Encode the `.p12` on one line:

   ```sh
   openssl base64 -A -in DeveloperID.p12 -out DeveloperID.p12.txt
   ```

## 2. Create the notarization credentials

Use one of the two options. Option A is better: the key is not your Apple ID password, and you can
revoke it alone.

**Option A: an App Store Connect API key**

1. Go to <https://appstoreconnect.apple.com/access/integrations/api>. Under **Team Keys**, select
   **+**. The first time, select **Request Access** and accept the terms.
2. Name the key `pinned notarytool` and give it the **Developer** role.
3. Copy the **Issuer ID** above the table (`APPLE_API_ISSUER`) and the key's **Key ID**
   (`APPLE_API_KEY`).
4. Select **Download** on the key's row. Apple lets you download `AuthKey_<KEYID>.p8` once only. Its
   text is `APPLE_API_PRIVATE_KEY`.

**Option B: your Apple ID and an app-specific password**

1. Go to <https://account.apple.com>, then **Sign-In and Security > App-Specific Passwords**. Create
   one named `pinned notarytool`. It is `APPLE_PASSWORD`.
2. `APPLE_ID` is your Apple ID email. `APPLE_TEAM_ID` is the team ID from step 1.6.

To check the credentials before CI uses them, run one of these. An empty history means they work.

```sh
xcrun notarytool history --key AuthKey_<KEYID>.p8 --key-id <KEYID> --issuer <ISSUER ID>
xcrun notarytool history --apple-id <APPLE ID> --team-id <TEAMID> --password <APP PASSWORD>
```

## 3. Add the GitHub secrets

Run these from the repository. Each command reads the value from a file or a prompt, so the value
stays out of your shell history.

```sh
gh secret set APPLE_CERTIFICATE < DeveloperID.p12.txt
gh secret set APPLE_CERTIFICATE_PASSWORD
gh secret set APPLE_SIGNING_IDENTITY

# Option A
gh secret set APPLE_API_KEY
gh secret set APPLE_API_ISSUER
gh secret set APPLE_API_PRIVATE_KEY < AuthKey_<KEYID>.p8

# Option B
gh secret set APPLE_ID
gh secret set APPLE_PASSWORD
gh secret set APPLE_TEAM_ID
```

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | The `.p12`, base64 on one line (step 1.8) |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` export password (step 1.7) |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <your name> (<TEAMID>)` (step 1.6) |
| `APPLE_API_KEY` | The API key's Key ID (option A) |
| `APPLE_API_ISSUER` | The Issuer ID (option A) |
| `APPLE_API_PRIVATE_KEY` | The text of `AuthKey_<KEYID>.p8` (option A) |
| `APPLE_ID` | Your Apple ID email (option B) |
| `APPLE_PASSWORD` | The app-specific password (option B) |
| `APPLE_TEAM_ID` | The 10-character team ID (option B) |

tauri-action passes these names to the Tauri CLI, with one exception. Tauri reads the API key from a
file named by `APPLE_API_KEY_PATH`, so `scripts/apple-signing-env.sh` writes
`APPLE_API_PRIVATE_KEY` to `$RUNNER_TEMP/AuthKey_<KEYID>.p8` and sets that path. When both options
are set, the API key wins.

## 4. Change the app before the first signed release

Make these changes in one pull request, and test them with a signed local build:
`APPLE_SIGNING_IDENTITY="Developer ID Application: …" npx tauri build`.

1. **Ship the dictation helper as a sidecar.** Tauri signs `bundle.externalBin` sidecars and
   frameworks, and copies `bundle.resources` as they are. The 0.17.22 app's
   `Contents/Resources/binaries/dictation-helper` is only `adhoc,linker-signed`, and notarization
   rejects a binary without a Developer ID signature, a secure timestamp and the hardened runtime.
   - In `build.rs`, copy the helper to `binaries/dictation-helper-$TARGET` (the `TARGET` variable
     of the build script) instead of `binaries/dictation-helper`.
   - Add `tauri.macos.conf.json`, which Tauri merges over `tauri.conf.json` on macOS only:

     ```json
     { "bundle": { "externalBin": ["binaries/dictation-helper"], "resources": [] } }
     ```

     Tauri strips the target from the name and puts the helper at `Contents/MacOS/dictation-helper`.
     `src/dictation.rs` already looks for it next to the executable.
   - Keep the stub that `build.rs` writes for other systems, or move `resources` into the macOS file.
2. **Give the hardened runtime the microphone.** Notarization requires the hardened runtime, which
   Tauri turns on for every signed executable (`bundle.macOS.hardenedRuntime`, `true` by default).
   It blocks the microphone unless the binary has an entitlement. Add `Entitlements.plist`:

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>com.apple.security.device.audio-input</key>
     <true/>
   </dict>
   </plist>
   ```

   Set `bundle.macOS.entitlements` to `"Entitlements.plist"` in `tauri.conf.json`. This entitlement
   is not restricted, so ad-hoc builds accept it too. After the build, check that both binaries
   carry it, then hold a dictation session in the signed app:

   ```sh
   codesign -d --entitlements - pinned.app/Contents/MacOS/pinned
   codesign -d --entitlements - pinned.app/Contents/MacOS/dictation-helper
   ```
3. **Keep `"signingIdentity": "-"`.** `APPLE_SIGNING_IDENTITY` overrides it, so local builds and
   releases without the secrets stay ad-hoc signed.
4. **Leave `providerShortName` unset.** Tauri needs it, or `APPLE_PROVIDER_SHORT_NAME`, only when
   an Apple ID belongs to several teams and you notarize with option B. `APPLE_TEAM_ID` already
   names the team.

## 5. What the release does with the secrets

`scripts/apple-signing-env.sh` runs before tauri-action on the macOS legs. It writes to
`$GITHUB_ENV` only the variables whose secrets exist. Tauri reads a variable that is set but empty
as a certificate or an identity, and fails. `scripts/test-apple-signing-env.sh` checks each case.

| Secrets present | Result |
| --- | --- |
| None | Ad-hoc signed, as today |
| Certificate, password and identity | Signed with the Developer ID, not notarized. Tauri logs `skipping app notarization` |
| Those, plus option A or option B | Signed, notarized and stapled |
| A certificate with a missing password or identity, or an incomplete option | The step fails and names the missing secret |

To check a signed release, install it and run:

```sh
codesign -dv --verbose=4 /Applications/pinned.app   # Authority=Developer ID Application: …, TeamIdentifier=<TEAMID>
spctl -a -vvv -t exec /Applications/pinned.app      # accepted, source=Notarized Developer ID
xcrun stapler validate /Applications/pinned.app     # The validate action worked!
```

The updater keeps its own minisign signature (`TAURI_SIGNING_PRIVATE_KEY`). Developer ID signing
does not change it.

## 6. The Keychain after signing

Signing turns the Keychain on. The release workflow sets `PINNED_DEVELOPER_ID_BUILD=1` at compile
time when the Developer ID secrets exist (`scripts/apple-signing-env.sh`), and only then does the
app keep its hub login in the Keychain. An ad-hoc build keeps it in `account.json`, because the item
would name that build's code hash and every update would ask for the Mac password.

In a signed build, the app keeps its hub login in the login keychain as a generic password
(`crates/core/src/login_store.rs`):

- service `dev.personal-harness.pinned.hub-login`
- account `<hub URL> <account email>`
- created by the app with the default access list, which names the build that made it

Any other program reads the login only after the user types their Mac password into a system
prompt. Signing changes this in two stages.

**Signing alone.** An item that a signed build creates names the app by its designated requirement:
`identifier "dev.personal-harness.pinned"` plus Apple's Developer ID anchor plus
`certificate leaf[subject.OU] = <TEAMID>`. Every later signed update matches it, so no prompt
appears. An item an ad-hoc build made still lists only ad-hoc builds. The first signed version must
move the login once: read it from the old item (one prompt, the last), write it to a new item under
a new service, such as `dev.personal-harness.pinned.hub-login.2`, read it back, then delete the old
item. This is the same move `Credentials::load_app` makes from `account.json`, and it is safe to run
again.

**Signing with a provisioning profile.** This is the stronger end state. The data protection
keychain keeps an item in an access group, and only binaries signed by your team with that group in
their entitlements can read it. No other program can read it, prompt or not.

1. At <https://developer.apple.com/account/resources/identifiers/add/bundleId>, register an explicit
   macOS App ID for `dev.personal-harness.pinned`.
2. At <https://developer.apple.com/account/resources/profiles/add>, select **Developer ID** under
   Distribution, the App ID, then the Developer ID Application certificate. Download the
   `.provisionprofile`. Check that its entitlements list `keychain-access-groups` = `<TEAMID>.*`:
   `security cms -D -i pinned.provisionprofile`.
3. Add a config used only when the secrets exist, such as `tauri.devid.conf.json`, and pass it as
   `--config tauri.devid.conf.json` on the macOS legs:

   ```json
   {
     "bundle": {
       "macOS": {
         "entitlements": "Entitlements.devid.plist",
         "files": { "embedded.provisionprofile": "pinned.provisionprofile" }
       }
     }
   }
   ```

   `Entitlements.devid.plist` holds the audio entitlement from step 4.2, plus:

   ```xml
   <key>com.apple.application-identifier</key>
   <string><TEAMID>.dev.personal-harness.pinned</string>
   <key>com.apple.developer.team-identifier</key>
   <string><TEAMID></string>
   <key>keychain-access-groups</key>
   <array><string><TEAMID>.dev.personal-harness.pinned</string></array>
   ```

   Keep these out of `tauri.conf.json`. They are restricted entitlements, and macOS refuses to
   launch an ad-hoc build that claims them without a matching profile.
4. In `login_store.rs`, turn on the `protected` feature of `apple-native-keyring-store` and build
   the macOS entry with `protected::Cred::build(SERVICE, account,
   AccessPolicy::WhenUnlockedThisDeviceOnly, Some("<TEAMID>.dev.personal-harness.pinned".into()),
   false)`. Move an existing login from the login keychain the same way as above: write, read back,
   then delete the old item.

The Windows Credential Manager has no per-program access list, so no signature changes what can
read the login there.
