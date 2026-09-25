# Password locker design

Status: draft, waiting on Daniel's sign off. Nothing in this document is built yet. This is a
design only, so this PR touches docs and nothing else.

## Decisions made by Daniel (2026-09-25)

- Auto-lock after 5 minutes of no locker activity.
- Unlock is a master passphrase plus a current code from an authenticator app (TOTP), in place of
  the earlier Windows Hello plan.
- Revealing a saved password in plain text always needs a fresh authenticator code.
- The recovery phrase is about 12 words, written on paper.

## Why

Daniel's Opera GX profile was the target of an infostealer in July 2026. The stealer read the
saved passwords and cookies straight out of Opera's SQLite files, because a browser's built in
password store is, on every mainstream browser, a database that anything running as the logged
in user can copy and decrypt. The goal here is a locker that survives that exact attack: even if
malware runs as Daniel and copies every file on disk, it should not get a single password out of
the locker without also having Daniel's passphrase and running on this specific machine.

## Goals

- The locker's data key is never stored on disk in usable form. It is wrapped under a key that
  itself depends on two things Daniel has: his passphrase, and this laptop's TPM.
- The TPM half of that wrapping is not bound to firmware or boot measurements (PCRs), because
  Daniel's laptop needs a BIOS update and a measurement bound key would be destroyed by that
  update. It is also not gated behind any user-presence prompt; see "How unlock works" for why.
- Every unlock needs Daniel's master passphrase and a current 6-digit code from his authenticator
  app, not just "the app is running as Daniel."
- A copied locker database file is useless on its own: without this laptop's TPM it cannot be
  unwrapped even with the correct passphrase, and without the correct passphrase it cannot be
  unwrapped even on this laptop.
- Autofill only ever fills into the exact origin a credential was saved for, on a real user
  gesture, so a lookalike phishing page gets nothing.
- Page scripts get no API that can read the locker, before or after a fill.
- The Claude control channel and every MCP tool it exposes can never read, list, or export locker
  entries, the TOTP secret, or any wrapped key material.
- Clipboard copies of a password clear themselves after a short time.
- A one time offline recovery phrase can re-wrap the data key after a TPM reset, a motherboard
  swap, or a reinstall, and can also re-enroll a new authenticator secret if Daniel's phone is
  lost, without which the data is unrecoverable by design.
- Export is only ever encrypted, never plaintext.
- Where a site supports passkeys, BlueFlame prefers a passkey over a stored password.
- Repeated wrong passphrases or wrong codes are met with increasing delays and an eventual
  lockout, tracked on disk so restarting the app does not reset the count.

## Threat model

| Threat | What the design does | Residual risk |
| --- | --- | --- |
| Infostealer running as Daniel (his exact July 2026 scenario), no admin rights | It can read the SQLite file, but every entry is AES-256-GCM ciphertext under a data key that is itself wrapped under a key derived from Daniel's passphrase (Argon2id) combined with a secret that is only unsealable through this laptop's TPM. Reading the file gives ciphertext and nothing else; it does not give the passphrase or a way to reach the TPM secret from a different machine. | If this same malware is running while Daniel actually unlocks the locker or reveals a password, see the row below; that is a materially different and worse case than a passive file copy. |
| Malware already running as Daniel on this machine, at the moment he unlocks the locker or reveals a password | Nothing in this design stops it. It can keylog the passphrase as Daniel types it, and because the TOTP check and the TPM unseal both run inside BlueFlame's own process under Daniel's account, malware with the same level of access can, in principle, call the same TPM APIs BlueFlame calls (the TPM key needs no user presence, so anything running as Daniel can ask it to unseal) or patch BlueFlame's own binary in memory to skip the TOTP check entirely. | This is the honest limit of the whole design: it stops someone who only has the passphrase (shoulder surfing, a reused or leaked passphrase, a phishing page that captured it) and it stops bulk offline theft of a copied file. It does not stop malware that already has arbitrary code execution as Daniel on this laptop. Against that, only not getting infected in the first place helps. |
| Someone who obtained only Daniel's passphrase (shoulder surfing, password reuse, a leaked credential dump) | This is what the second factor is actually for. Without a current code from Daniel's own authenticator app, BlueFlame's unlock screen refuses even a correct passphrase. | If the same person also has Daniel's phone, or a copy of the sealed authenticator secret plus a way to run it on this exact laptop, the second factor stops helping. Neither is true for someone who only phished or reused the passphrase. |
| Stolen disk image or backup, no access to the original machine or its TPM | The wrapped data key and every entry are ciphertext tied to a TPM that is not present. There is no way to derive the wrapping key at all without either that exact TPM or the recovery phrase. | If the recovery phrase was written down and also stolen alongside a full disk backup, the attacker can rewrap and read everything. The recovery phrase must be stored somewhere the disk thief does not also reach. |
| Phone lost, stolen, wiped, or the authenticator app is uninstalled | BlueFlame still unlocks through the recovery phrase path, which re-enrolls a brand new TOTP secret (new QR code, new confirmation) without needing the old one. | Until Daniel does that re-enrollment, he cannot unlock the ordinary way at all; that delay is intentional, the same as any lost second factor. |
| Phishing site made to look like a real one (wrong origin, same appearance) | Autofill only fires on an exact origin match (scheme, host, and port). A lookalike domain never gets a match, so nothing is offered to fill. Passkeys are phishing resistant for the same reason: WebAuthn binds the credential to the real origin at the protocol level. | A phishing page can still ask Daniel to type his passphrase and a current TOTP code directly into it. Nothing about this design changes that; it is the same social engineering risk any passphrase and TOTP combination has everywhere else on the internet. |
| Malicious script on a page Daniel is actually visiting (XSS, hostile ad, compromised dependency) | The script has no JavaScript API that returns saved entries. A fill only happens after a genuine, browser verified user gesture, checked with the same signal browsers use to distinguish a real click from a script generated one, and only into the field that gesture targeted. | Once a value lands in a form field on that page, any script already running on that page can read it back out of the field, same as with every browser's native autofill. This is not specific to BlueFlame and is not solvable without breaking normal form filling. |
| A compromised Claude session, or a malicious prompt reaching the control channel | The MCP tool set the control channel exposes has no read, list, or export tool for the locker, the TOTP secret, or any wrapped key, at the bridge layer, not just by convention. The bridge also refuses to open or read the locker's own UI route for any automated navigation or read-page call. Autofill and unlock trigger require a real OS level input event; Claude's click and type tools do not produce that signal and are rejected. | A compromised Claude session could still ask Daniel, through the chat, to type his passphrase and code and read something back to it himself, which is a social engineering risk this design cannot close by itself. |
| TPM reset (BIOS update, CMOS clear, motherboard replacement, clean reinstall) | The TPM-held secret is gone, so the data key cannot be unwrapped through the passphrase path anymore. The recovery phrase, stretched with Argon2id, derives a second key that also unwraps the data key, so Daniel can re-wrap it under a freshly provisioned TPM secret. | Without the recovery phrase, the locker is unrecoverable. That is intentional: anything that could recover it without the phrase would also be a second way to steal it. |
| Someone repeatedly guessing the passphrase or TOTP codes at the unlock screen | Failed attempts are counted in the database, not in memory, and each one after the third adds an increasing delay before the next attempt is even accepted; after ten consecutive failures the locker locks out entirely for a period that grows with every further attempt made during the lockout. | A very strong, rarely reused passphrase and treating the authenticator app itself as sensitive (screen lock on the phone) are still Daniel's job; the lockout raises the cost of guessing, it does not make a weak passphrase strong. |

### What this does not stop, said plainly

Malware running as Daniel, at the moment he actually unlocks the locker, reveals, or types a
password, can still watch it happen: a keylogger sees the passphrase and the TOTP code he types,
and a screen scraper or a malicious accessibility hook can see what gets revealed, filled, or
copied. No local software design stops an attacker who already has code running as the same user
watching the screen or the keyboard in real time. A current TOTP code makes a stronger claim than
it can actually back up here: it is a gate enforced by BlueFlame's own code at the moment of
unlock, not a second ingredient in the encryption itself (see "How unlock works" for why it is
deliberately kept out of the key derivation). That gate stops someone who has only the passphrase,
which is the realistic case after a phishing page, a reused password, or someone watching Daniel
type over his shoulder. It does not stop malware that is already running as Daniel on this
machine, because that malware can reach the same sealed TOTP secret and the same TPM calls
BlueFlame itself uses, or simply patch the check out of BlueFlame's running process. Against that
case, the passphrase and the TPM binding on the actual encryption are what is doing the real work,
and a keylogger defeats the passphrase too. The property this design buys is different and
narrower: it stops bulk, offline theft of every saved credential at once, the way Opera GX's store
was taken in July, and it stops someone from unlocking with a stolen or guessed passphrase alone.
It does not turn an already-compromised machine into a safe place to type passwords.

## What "not bound to firmware measurements" and "no user presence" mean, in plain terms

Windows can create a TPM key several ways. One seals the key to the current boot state (which
firmware, bootloader, and OS files were measured at boot), so if any of those change, such as a
BIOS update, the key becomes permanently unreadable. Another gates every use of the key behind a
Windows Hello prompt (a user-presence policy). This design uses neither. The TPM key here is
created non-exportable and marked for decrypt only, with no `NCRYPT_UI_POLICY` and no platform
attestation or PCR policy attached. Its only property is that it cannot be exported from this
TPM; using it does not require, and does not trigger, any prompt at all. Daniel can update his
BIOS and the key keeps working. This is a deliberate change from an earlier draft of this design,
which used Windows Hello for every unlock; the human gate is now the passphrase and the
authenticator app, checked by BlueFlame itself, and the TPM's only job is to make sure a copied
database file cannot be unwrapped on any machine other than this one.

## Enrollment (setup)

Enrollment happens once, when Daniel first turns the locker on.

1. BlueFlame asks Windows to create a TPM backed key through the CNG NCrypt API against the
   Microsoft Platform Crypto Provider: non-exportable, usable for decrypt, no UI policy, no PCR
   or attestation policy (see above). This is `locker_tpm_key`.
2. BlueFlame generates a random 256-bit secret `S` with `OsRng` and seals it by encrypting it with
   `locker_tpm_key`'s public half (RSA-OAEP, SHA-256), giving `wrapped_tpm_secret`. `S` itself is
   discarded from memory once sealed and only ever exists again as the output of unsealing.
3. Daniel types his master passphrase twice (entry and confirmation). BlueFlame generates a random
   salt and derives a passphrase key with Argon2id (see "How unlock works" for parameters),
   storing only the salt and the parameters, never the passphrase or the derived key.
4. BlueFlame generates a random AES-256-GCM data key with `OsRng`, derives the key encryption key
   (KEK) from the passphrase key and the unsealed `S` (see below), and wraps the data key under
   that KEK, giving `wrapped_data_key`.
5. BlueFlame generates a random 160-bit TOTP secret with `OsRng`, base32 encodes it, and builds an
   `otpauth://totp/BlueFlame:daniel?secret=<base32>&issuer=BlueFlame&algorithm=SHA1&digits=6&period=30`
   URI. It renders that URI as a QR code (for scanning into Google Authenticator, Aegis, 2FAS, or
   any standard TOTP app) and also shows the base32 secret as text for manual entry, once, on
   screen. Daniel then types back two consecutive codes from his app before enrollment completes,
   which catches a mis-scanned secret or a clock badly out of sync before it becomes a lockout
   later. Once confirmed, the TOTP secret is sealed with `locker_tpm_key` the same way `S` is
   (`wrapped_totp_secret`) and the plaintext secret is dropped from memory; the QR code and text
   are shown exactly once and are not retrievable again except by re-enrolling.
6. BlueFlame generates the recovery phrase (about 12 words) and shows it once, per the existing
   "Recovery phrase" section below, and wraps the data key a second time under a key derived from
   it, giving `wrapped_data_key_recovery`.

## How unlock works

An earlier draft of this design derived the key encryption key (the KEK) from a TPM signature
gated behind Windows Hello, on the idea that the same key signing the same fixed challenge would
always produce the same signature. That was wrong for the key type this design needs (TPM 2.0's
ECDSA signing uses a randomized per-signature nonce, so two signatures over the same challenge are
not the same bytes), and Daniel has since asked for authenticator codes instead of Windows Hello
regardless. The current design derives the KEK from two things that do not change between unlocks
(the passphrase and the TPM's non-exportable secret `S`), and checks the authenticator code as a
separate step, not as an ingredient in the key derivation at all. A TOTP code is only valid for
about 30 seconds and Daniel could type any of the two or three codes straddling that window; a key
derivation has to produce exactly one answer, so mixing the two would either force a fragile
"try the last few codes" search inside the crypto path or throw away the property that losing the
phone should only cost Daniel a re-enrollment, not the data itself. Keeping TOTP as a pure gate
avoids both problems, at the honest cost described above: it is enforced by BlueFlame's code, not
by the math.

1. Daniel opens the locker and is asked for his master passphrase and a current code from his
   authenticator app, in the same screen.
2. BlueFlame checks the persisted lockout state first (see "Brute-force limits" below). If the
   locker is currently locked out, the attempt is refused immediately with the remaining wait
   time shown; nothing below runs.
3. BlueFlame derives the passphrase key with Argon2id, using the stored salt and parameters:
   Argon2id, 64 MiB of memory, 3 iterations, 4 lanes of parallelism (RFC 9106's second recommended
   parameter set, chosen because it is strong against offline guessing while still unlocking in
   under a second on Daniel's laptop). The passphrase itself is never stored.
4. BlueFlame calls `NCryptDecrypt` against `locker_tpm_key` to unseal `S` from
   `wrapped_tpm_secret`. This needs no prompt and produces the same `S` every time on this laptop,
   and cannot be performed on any other machine because the key is non-exportable.
5. BlueFlame derives the KEK with HKDF-SHA256, using the passphrase key and `S` as input keying
   material, and attempts to unwrap `wrapped_data_key` with it under AES-256-GCM. GCM's built in
   authentication tag means a wrong passphrase (or a locker file copied to a different machine,
   where `S` would be unrecoverable) makes this unwrap fail outright rather than silently produce
   garbage.
6. In parallel, BlueFlame unseals `wrapped_totp_secret` the same way as `S`, and checks the code
   Daniel typed against the current 30-second time step and the one immediately before and after
   it (one step of allowed clock drift either way), using constant-time comparison. To stop the
   same shoulder-surfed code being replayed a second time, BlueFlame also checks the accepted
   code's time step against `last_totp_step` (see "Data format") and refuses a step it has already
   accepted.
7. Unlock only succeeds if both step 5 and step 6 succeed. If either fails, the attempt is counted
   as one failure (see "Brute-force limits"); BlueFlame does not reveal which of the two factors
   was wrong, so a partially correct guess learns nothing. If both succeed, the failure counter and
   lockout state are cleared, `last_totp_step` is updated, and the recovered data key is held in
   memory only, never written to disk unwrapped.
8. The data key decrypts locker entries on demand, one at a time, for as long as the locker stays
   unlocked.
9. A short auto-lock timer (5 minutes of no locker activity by default, configurable) zeroes the
   data key out of memory and returns to the locked state. Reaching for a saved password after
   that needs a fresh passphrase and TOTP code, exactly like the first unlock.

This still needs to be confirmed against Daniel's actual hardware in phase 1: create a
non-exportable NCrypt decrypt key with no UI policy, seal and unseal a test value across two
separate app runs, and confirm the same plaintext comes back both times with no prompt shown. If
Daniel's TPM cannot do this, the fallback is the same as before, a software key in the Microsoft
Software Key Storage Provider, which loses the non-exportable, hardware backed guarantee. That
fallback is noted here as an open gap, not a solved case: a software key does not give the
"cannot be bulk-stolen" property this whole design exists to deliver, so if Daniel's TPM turns out
not to support this, that gap needs a real answer before this feature ships as his daily locker,
not just this fallback.

## Brute-force limits

Failures are counted per locker, not per unlock screen session, and the count lives in
`locker_meta` in the SQLite database, so quitting BlueFlame, restarting Windows, or killing the
process does not reset it.

- Attempts 1 through 3 after a successful unlock (or after a clean start) are checked immediately,
  with no added delay.
- From the 4th consecutive failure onward, BlueFlame adds a delay before the next attempt is even
  read from the form: `min(2^(failures - 3), 300)` seconds, so the wait grows from 2 seconds up to
  a 5 minute cap.
- After 10 consecutive failures, the locker enters a hard lockout: no attempt is processed at all,
  correct or not, for 30 minutes. Trying again during a lockout extends it by another 30 minutes,
  so repeatedly hammering the unlock screen only makes the wait longer.
- A failure on either the passphrase or the TOTP code counts as one failure toward this same
  counter; BlueFlame does not give an attacker a separate, larger budget for one factor by telling
  them which one was wrong.
- A fully successful unlock, or a successful use of the recovery phrase, clears the failure count
  and any active lockout back to zero.

This does not make a weak passphrase strong on its own, but it makes online guessing of either
factor slow enough to be impractical long before it makes progress, and it removes the option of
just restarting the app to get a clean slate.

## Data format

Everything lives in BlueFlame's existing SQLite database (`storage.rs`), in two new tables.

`locker_meta` (one row):

- `format_version` (integer)
- `locker_tpm_key_name` (text): the name of the non-exportable NCrypt key this locker is sealed
  under (no UI policy, no PCR binding)
- `wrapped_tpm_secret` (blob): the random secret `S`, sealed under `locker_tpm_key_name` with
  RSA-OAEP, that combines with the passphrase to form the KEK
- `wrapped_data_key` (blob): the data key, wrapped under the KEK (HKDF-SHA256 of the Argon2id
  passphrase key and `S`) with AES-256-GCM and a fresh nonce
- `kek_salt`, `kek_argon2_params` (blob, text): the salt and Argon2id parameters (memory,
  iterations, parallelism) used to derive the passphrase half of the KEK
- `wrapped_totp_secret` (blob): the TOTP secret, sealed under `locker_tpm_key_name` the same way
  as `S`, so an off-machine copy of this database is useless for generating codes
- `last_totp_step` (integer): the time-step counter of the most recently accepted TOTP code, so
  the same 6-digit code cannot be replayed a second time inside its 30 second window
- `failed_attempt_count` (integer), `lockout_until` (timestamp, nullable): persisted brute-force
  state for both factors combined; see "Brute-force limits"
- `wrapped_data_key_recovery` (blob): the data key, wrapped a second time under the recovery
  phrase path KEK, so either path alone can restore access
- `recovery_salt`, `recovery_argon2_params` (blob, text): Argon2id parameters used to derive the
  recovery KEK, so the phrase alone is not enough without redoing the exact stretch
- `created_at`

`locker_entries` (one row per saved credential):

- `id` (primary key)
- `origin` (text, exact scheme plus host plus port, stored in the clear so BlueFlame can match a
  page's origin without decrypting anything first)
- `kind` (text: `password` or `passkey_meta`, for a passkey this only stores which account exists
  on which origin, never a private key, since passkey private keys live in the TPM through
  WebAuthn, not in this table)
- `encrypted_blob` (blob): AES-256-GCM ciphertext of a small JSON document holding the username,
  the password, and any notes, with a fresh random nonce per entry stored alongside the
  ciphertext
- `created_at`, `updated_at`

Only the origin is ever stored unencrypted. Username and password are always inside the
encrypted blob, so a raw copy of the database file gives an attacker a list of which sites Daniel
has saved credentials for, and nothing else.

### Encrypted export format

A single file containing a header (format version, Argon2id salt and parameters) followed by one
AES-256-GCM ciphertext of the full entry list, encrypted under a key derived from a passphrase
Daniel types at export time. There is no plaintext export option. Importing asks for the same
passphrase back.

## Autofill design

- Autofill only ever offers to fill a field on the page whose origin exactly matches a saved
  entry's `origin` column. Subdomains, different ports, and different schemes do not match.
- Nothing is offered or filled without Daniel starting it: he clicks a field, BlueFlame shows a
  small "fill saved password" affordance for that field, and only clicking it (or an explicit
  keyboard shortcut he presses himself) triggers the fill. The trigger checks that the input came
  from a real device event, not a synthetic one, so it cannot be driven by page script or by the
  Claude control channel's automation tools.
- The fill itself is done from the Rust side, through WebView2's own scripting bridge, by setting
  the field's value and dispatching the same input and change events a real keystroke would
  produce. There is no JavaScript variable or `window` property that exposes the plaintext value
  to the page before the fill happens; the page only ever sees the value land in its own field,
  the same as with any browser's native autofill.
- The data key decrypts one entry, fills it, and the decrypted plaintext is dropped immediately
  after; it is not cached for other fields or other tabs.
- Autofill rides on the locker already being unlocked; it does not ask for a fresh passphrase or
  TOTP code per fill, the same as before. Only revealing a password in plain text does (below).

## Revealing a saved password

Showing a saved password as plain text on screen always asks for a fresh, current TOTP code, even
while the locker is unlocked; a stale or already-used code is rejected the same as at unlock.
Autofill into the exact saved origin rides on the current unlock; revealing does not, because it
is the one action that puts a password where anything watching the screen can read it. Daniel's
passphrase is not asked for again here, only the code, since re-typing the full passphrase for
every reveal would push him toward writing it somewhere insecure; the honest limit in the threat
model above (a TOTP check is enforced by BlueFlame's code, not by the encryption) applies here
too.

## Clipboard

A "copy password" action puts the plaintext on the system clipboard and starts a short timer
(a few seconds). When the timer fires, BlueFlame checks whether the clipboard still holds exactly
what it put there and, if so, overwrites it with an empty string, so a value Daniel forgot about
does not sit in clipboard history indefinitely.

## Recovery phrase

At setup, after the TPM secret and the TOTP secret are both sealed, BlueFlame generates a strong,
passphrase grade secret (a long random value shown as a phrase of about 12 words, similar to a
diceware phrase, to be written on paper) and shows it to Daniel exactly once, with a clear warning
to write it down somewhere offline and separate from the laptop. It is never stored anywhere in
BlueFlame, on disk or otherwise. Losing it is fine as long as the TPM secret and the authenticator
app both keep working; it only matters on the day one of them stops.

The phrase is stretched with Argon2id (a slow, memory hard function built for exactly this:
turning a human memorable or human copyable secret into a strong key without being cheap to brute
force) using the salt and parameters stored in `locker_meta`. The result derives the recovery KEK,
which can unwrap `wrapped_data_key_recovery` at any time, and is the entry point for two separate
recovery flows:

- **TPM reset** (BIOS update gone wrong in a way that did clear the TPM anyway, CMOS clear,
  motherboard replacement, clean reinstall): the recovery KEK unwraps the data key, BlueFlame
  provisions a fresh `locker_tpm_key` and a fresh sealed secret `S`, and re-wraps the data key
  under a new passphrase-and-TPM KEK, so Daniel is not locked out permanently by a hardware
  change.
- **Lost, stolen, or wiped phone**: the recovery KEK unwraps the data key without needing any
  TOTP code at all, and BlueFlame walks Daniel through the enrollment flow again (new random TOTP
  secret, new QR code, two-consecutive-code confirmation) to replace `wrapped_totp_secret`.

Either flow also clears `failed_attempt_count` and `lockout_until` back to zero, so a lockout
never becomes permanent as long as Daniel still has the recovery phrase.

## Passkeys

Where a site offers a passkey instead of a password, BlueFlame should prefer it. This is a
separate feature from the locker's own unlock described above: WebView2 is Chromium based and
already implements WebAuthn, using whatever platform authenticator Windows itself offers (Windows
Hello, if Daniel has it set up, or a security key) as the site's authenticator, not BlueFlame's
passphrase-and-TOTP scheme. Supporting this mostly means surfacing the option clearly in the UI
(detecting that a site supports passkeys and suggesting one) rather than building new
cryptography. Passkeys are phishing resistant in the same way the locker's own autofill is: the
browser binds the credential to the real origin, so a lookalike site cannot obtain or trigger it.
The locker only stores metadata for passkeys (which account exists on which origin), never key
material, since the private key lives inside the TPM through Windows' own WebAuthn platform
authenticator, outside BlueFlame's control entirely.

## Windows APIs and Rust crates to use

- **NCrypt / CNG** (`windows::Win32::Security::Cryptography`, targeting the Microsoft Platform
  Crypto Provider): creates the one non-exportable, decrypt-capable TPM key this design needs,
  reached from Rust through the `windows` crate's Win32 bindings. It is created with no
  `NCRYPT_UI_POLICY` and no platform attestation or PCR policy, so using it triggers no prompt of
  any kind; its only job is sealing `S` and the TOTP secret so a copy of the database is useless
  off this machine. `Windows.Security.Credentials.KeyCredentialManager` (Windows Hello for Apps)
  is not used anywhere in this design; it only supports signing, not decrypt, which is what led to
  the unsound signature-derived KEK in an earlier draft of this document, and Daniel has since
  asked for authenticator codes in place of Hello regardless.
- `argon2`: stretching the master passphrase into the passphrase half of the KEK, the recovery
  phrase into the recovery KEK, and the export passphrase into an export key. Parameters: Argon2id,
  64 MiB memory, 3 iterations, 4 lanes (RFC 9106's second recommended set).
- `hkdf`: combining the Argon2id passphrase key and the TPM-unsealed secret `S` into the final KEK
  used to wrap the data key.
- `aes-gcm`: AEAD encryption for every entry's `encrypted_blob`, for the data key wrap under the
  KEK, and for the recovery path's wrap of the data key.
- `totp-rs` (or an equivalent RFC 6238 crate): generating and verifying 6-digit, SHA-1, 30-second
  step TOTP codes at enrollment and unlock, with a one-step drift window checked both directions.
- `qrcode`: rendering the enrollment `otpauth://` URI as a QR code for scanning into an
  authenticator app.
- `rand` (via `OsRng`): generating nonces, the data key, the TPM-sealed secret `S`, the TOTP
  secret's entropy, and the recovery phrase's underlying entropy.
- `zeroize`: clearing the data key, the passphrase key, the unsealed secret `S`, the TOTP secret,
  and any decrypted entry plaintext from memory as soon as each is no longer needed.
- `rusqlite`: already a BlueFlame dependency, used for the two new tables above.
- A small word list crate (for example `bip39`, used only for its word list and encoding, not its
  wallet semantics) to render the recovery phrase's random bytes as a word phrase Daniel can write
  down and type back in, rather than a raw hex string.

The same scheme works on Linux without changes to the design: a TPM-backed, non-exportable sealed
object created through `tpm2-tss` (via the `tss-esapi` crate) with no PCR policy and no
fingerprint-style authorization value plays the same role as the NCrypt key above, or, where no
TPM is present, the kernel's own "trusted key" type (`keyctl`) seals the same secret without any
user-presence prompt either. Everything above the TPM line (Argon2id, HKDF, AES-256-GCM, TOTP, the
SQLite schema) is plain Rust and is identical on both platforms. This removes the fprintd
dependency an earlier plan for Linux parity assumed, since nothing here needs a fingerprint
prompt, and simplifies that roadmap item to "point the same `LockerKeyStore` trait at tpm2-tss or
the kernel keyring instead of NCrypt."

## Fake store for tests

All TPM access goes through a small Rust trait (something like `LockerKeyStore`, with methods for
provision, wrap, unwrap, and delete), so tests run against an in-memory fake implementation
instead of touching Daniel's real TPM, his real BlueFlame CA, or any real device. Any test that
does need a real CNG or TPM backed key for an integration check creates it with a name prefixed
`blueflame-test-` and deletes it before the test ends, so nothing test related is left behind in
the TPM. TOTP verification is tested against known RFC 6238 test vectors and a fake clock, so
tests never depend on wall-clock timing or a real authenticator app.

## Phased build plan

- **Phase 1 (S): key provisioning.** The `LockerKeyStore` trait, the NCrypt backed
  implementation against the Microsoft Platform Crypto Provider (non-exportable, no UI policy, no
  PCR binding), the software key fallback for machines without a usable TPM, and the fake used in
  tests. This phase's acceptance check is the seal and unseal round trip described above, with no
  prompt shown on either call. No UI yet.
- **Phase 2 (S): passphrase KDF and data key wrap.** Argon2id for the passphrase key, HKDF to
  combine it with the TPM-unsealed secret `S` into the KEK, and AES-256-GCM wrap and unwrap of the
  data key, unit tested end to end against the fake store.
- **Phase 3 (S): TOTP enrollment and verification.** Secret generation, the `otpauth://` URI and
  QR code, the two-consecutive-code confirmation step, sealed storage of the secret via the same
  `LockerKeyStore`, and verification with the one-step drift window and replay check against
  `last_totp_step`, tested against RFC 6238 vectors and a fake clock.
- **Phase 4 (S): brute-force lockout.** The persisted failure counter and lockout timestamp in
  `locker_meta`, the increasing delay and hard lockout logic from "Brute-force limits," and the
  reset path on a successful unlock or recovery.
- **Phase 5 (M): entry management.** Tauri commands and a settings style UI to add, view (behind
  a fresh TOTP code), edit, and delete entries. No autofill yet, so this phase alone is already
  useful as a manual locker.
- **Phase 6 (M): autofill.** Exact origin matching, the user gesture gated fill trigger, and the
  privileged fill path through WebView2's scripting bridge.
- **Phase 7 (S): auto-lock and clipboard self-clear.** The idle timer that zeroes the data key,
  and the self-clearing clipboard copy action.
- **Phase 8 (S): recovery phrase.** One time generation and display, Argon2id stretching, the
  second wrap of the data key, the re-wrap flow after a detected TPM reset, and the re-enrollment
  flow after a lost phone, both of which also clear the lockout state.
- **Phase 9 (S): encrypted export and import.**
- **Phase 10 (S): passkey UI.** Detecting passkey support on a site and suggesting it over a
  stored password; storing passkey metadata rows.
- **Phase 11 (S): Claude channel hardening.** Adding the locker's routes and commands, and the
  TOTP and TPM related columns specifically, to the control channel's hard block list, and a
  focused security pass once the phases above are code complete, before anything ships as the
  default password store.

Sizes use the same S, M, L convention as the rest of `ROADMAP.md`. Phases 1 through 4 and 7
through 11 are small enough to land as one pull request each; phases 5 and 6 are medium and will
likely each split into two or three smaller pull requests as they get built.

## Open questions for Daniel

- The lockout curve above (delay from the 4th failure, hard lockout after 10) is a proposed
  default, not something Daniel has confirmed. Is that too strict or too lenient given how often
  he expects to fumble a code?
- Recovery phrase word list: Daniel already decided on a written word phrase over a QR code, but
  not which word list to draw it from. The design below assumes the standard BIP-39 English list
  (2048 words, used only for its word list and encoding, see the crates section) because it is
  the most common choice and every wallet-style app Daniel might reference already uses it. Is
  that the right list, or does Daniel want a plain diceware list instead?
- Is there a specific authenticator app Daniel already uses (Google Authenticator, Aegis, 2FAS,
  or something else), so the phase 3 acceptance check is run against that exact app rather than
  just RFC 6238 test vectors?
