# Password locker design

Status: draft, waiting on Daniel's sign off. Nothing in this document is built yet. This is a
design only, so this PR touches docs and nothing else.

## Why

Daniel's Opera GX profile was the target of an infostealer in July 2026. The stealer read the
saved passwords and cookies straight out of Opera's SQLite files, because a browser's built in
password store is, on every mainstream browser, a database that anything running as the logged
in user can copy and decrypt. The goal here is a locker that survives that exact attack: even if
malware runs as Daniel and copies every file on disk, it should not get a single password out of
the locker without also getting a live Windows Hello gesture from Daniel on that specific
machine.

## Goals

- The locker's master key never exists on disk in a usable form. It is generated inside the TPM
  and never leaves it.
- The TPM key is not bound to firmware or boot measurements (PCRs), because Daniel's laptop needs
  a BIOS update and a measurement bound key would be destroyed by that update.
- Every unlock needs a live Windows Hello gesture (face, fingerprint, or PIN), not just "the app
  is running as Daniel."
- Autofill only ever fills into the exact origin a credential was saved for, on a real user
  gesture, so a lookalike phishing page gets nothing.
- Page scripts get no API that can read the locker, before or after a fill.
- The Claude control channel and every MCP tool it exposes can never read, list, or export locker
  entries.
- Clipboard copies of a password clear themselves after a short time.
- A one time offline recovery code can re-wrap the data key after a TPM reset, a motherboard
  swap, or a reinstall, without which the data is unrecoverable by design.
- Export is only ever encrypted, never plaintext.
- Where a site supports passkeys, BlueFlame prefers a passkey over a stored password.

## Threat model

| Threat | What the design does | Residual risk |
| --- | --- | --- |
| Infostealer running as Daniel (his exact July 2026 scenario), no admin rights | It can read the SQLite file, but every entry is AES-256-GCM ciphertext under a data key that is itself wrapped by a TPM backed key. The TPM key cannot be exported, so the ciphertext is useless without a live Hello gesture on this machine. Decrypt commands are only reachable from BlueFlame's own frontend, not from another process. | If the malware runs while the locker is unlocked and Daniel is actively using it, it can potentially observe the same decrypted values BlueFlame's own UI process holds in memory for that short window (see "what this does not stop" below). |
| Stolen disk image or backup, no access to the original machine or its TPM | The wrapped data key and every entry are ciphertext tied to a TPM that is not present. There is no way to unwrap the data key at all without either that exact TPM or the offline recovery code. | If the recovery code was written down and also stolen alongside a full disk backup, the attacker can rewrap and read everything. The recovery code must be stored somewhere the disk thief does not also reach. |
| Phishing site made to look like a real one (wrong origin, same appearance) | Autofill only fires on an exact origin match (scheme, host, and port). A lookalike domain never gets a match, so nothing is offered to fill. Passkeys are phishing resistant for the same reason: WebAuthn binds the credential to the real origin at the protocol level. | None specific to this design; general phishing risk (Daniel typing a password by hand into the fake site) is unchanged. |
| Malicious script on a page Daniel is actually visiting (XSS, hostile ad, compromised dependency) | The script has no JavaScript API that returns saved entries. A fill only happens after a genuine, browser verified user gesture, checked with the same signal browsers use to distinguish a real click from a script generated one, and only into the field that gesture targeted. | Once a value lands in a form field on that page, any script already running on that page can read it back out of the field, same as with every browser's native autofill. This is not specific to BlueFlame and is not solvable without breaking normal form filling. |
| A compromised Claude session, or a malicious prompt reaching the control channel | The MCP tool set the control channel exposes has no read, list, or export tool for the locker, at the bridge layer, not just by convention. The bridge also refuses to open or read the locker's own UI route for any automated navigation or read-page call. Autofill triggers require a real OS level input event; Claude's click and type tools do not produce that signal and are rejected by the fill trigger. | A compromised Claude session could still ask Daniel, through the chat, to fill and read back a field himself, which is a social engineering risk this design cannot close by itself. |
| TPM reset (BIOS update, CMOS clear, motherboard replacement, clean reinstall) | The TPM backed key is gone, so the data key cannot be unwrapped that way anymore. The offline recovery code, stretched with Argon2id, can derive a second key that also unwraps the data key, so Daniel can re-wrap it under a freshly provisioned TPM key. | Without the recovery code, the locker is unrecoverable. That is intentional: anything that could recover it without the code would also be a second way to steal it. |

### What this does not stop, said plainly

Malware running as Daniel, at the moment he actually unlocks the locker and fills or types a
password, can still watch it happen: a keylogger sees what he types, and a screen scraper or a
malicious accessibility hook can see what gets filled or copied. No local software design stops
an attacker who already has code running as the same user watching the screen or the keyboard in
real time. The property this design buys is different and narrower: it stops bulk, offline theft
of every saved credential at once, the way Opera GX's store was taken in July. It does not turn
an already-compromised machine into a safe place to type passwords.

## What "not bound to firmware measurements" means, in plain terms

Windows can create a TPM key two ways. One seals the key to the current boot state (which
firmware, bootloader, and OS files were measured at boot), so if any of those change, such as a
BIOS update, the key becomes permanently unreadable. The other just asks the TPM to hold a key
and gate its use behind a policy such as "the user must present Windows Hello," with no
dependency on boot measurements. This design uses the second kind only. Daniel can update his
BIOS and the key keeps working, because the CNG NCrypt APIs used here (see below) create policy
bound keys, not measurement bound ones, by default; the design deliberately avoids the platform
attestation properties that would add measurement binding.

## How unlock works

An earlier draft of this design derived the key encryption key (the KEK) from a TPM signature,
on the idea that the same key signing the same fixed challenge would always produce the same
signature. That is wrong for the key type this design needs. Windows Hello for Apps
(`KeyCredentialManager`) creates ECDSA P-256 keys on a TPM 2.0 machine, and TPM 2.0's ECDSA
signing operation uses a randomized per-signature nonce, not the deterministic nonce scheme from
RFC 6979. Two signatures over the same challenge with the same key are both valid but are not
the same bytes. Deriving the KEK from the signature would therefore produce a different KEK on
most unlocks, and the wrapped data key would fail to unwrap starting with the second one. The
design below replaces that step with direct TPM wrap and unwrap, which has no such problem.

1. At setup, BlueFlame asks Windows to create a TPM backed RSA key through the CNG NCrypt API
   against the Microsoft Platform Crypto Provider (not through `KeyCredentialManager`, which
   only supports signing and cannot decrypt). The key is created non-exportable, marked for
   decrypt, and given an `NCRYPT_UI_POLICY` with the "force high protection" flag, so Windows
   itself gates every private key use behind a Windows Hello prompt.
2. BlueFlame generates a random AES-256-GCM data key with `OsRng` and wraps it by encrypting it
   directly with the TPM key's RSA public half, using RSA-OAEP (SHA-256). This wrapped blob is
   `wrapped_data_key_tpm`; it needs no separate KEK derivation step, because the TPM key itself
   is the wrapping key.
3. On every later unlock, BlueFlame calls `NCryptDecrypt` with that same TPM key and RSA-OAEP
   padding to unwrap `wrapped_data_key_tpm` back into the data key. Because of the key's UI
   policy, Windows pops the Hello prompt (face, fingerprint, or PIN) before it will perform the
   decrypt. This is a Windows system prompt, not something BlueFlame draws itself, so it cannot
   be faked by a webpage or by page script.
4. Decryption is deterministic: the same ciphertext under the same key always unwraps to the
   same data key, on the first unlock and on every one after it. The recovered data key is held
   in memory only, never written to disk unwrapped.
5. The data key decrypts locker entries on demand, one at a time, for as long as the locker
   stays unlocked.
6. A short auto-lock timer (a few minutes of no locker activity, configurable) zeroes the data
   key out of memory and returns to the locked state. Reaching for a saved password after that
   needs a fresh Windows Hello gesture, which means a fresh `NCryptDecrypt` call.

This still needs to be confirmed against Daniel's actual hardware in phase 1, because not every
TPM 2.0 implementation is guaranteed to allow an NCrypt RSA key marked for decrypt behind a
Windows Hello UI policy. The phase 1 acceptance check is exactly that: create such a key, wrap a
test value, unwrap it across two separate Hello prompts on two separate app runs, and confirm the
same plaintext comes back both times. If Daniel's TPM cannot do this, the fallback is a software
key in the Microsoft Software Key Storage Provider with the same UI policy, which loses the
non-exportable, hardware backed guarantee but keeps the same Hello gated flow. That fallback is
noted here as an open gap, not a solved case: a software key does not give the "cannot be
bulk-stolen" property this whole design exists to deliver, so if Daniel's TPM turns out not to
support this, that gap needs a real answer before this feature ships as his daily locker, not
just this fallback.

## Data format

Everything lives in BlueFlame's existing SQLite database (`storage.rs`), in two new tables.

`locker_meta` (one row):

- `format_version` (integer)
- `key_credential_name` (text): the name of the NCrypt key this locker is tied to
- `wrapped_data_key_tpm` (blob): the data key, encrypted directly with the TPM key's RSA public
  half (RSA-OAEP), with no separate KEK derivation step
- `wrapped_data_key_recovery` (blob): the data key, wrapped a second time under the recovery code
  path KEK, so either path alone can restore access
- `recovery_salt`, `recovery_argon2_params` (blob, text): Argon2id parameters used to derive the
  recovery KEK, so the code alone is not enough without redoing the exact stretch
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

## Clipboard

A "copy password" action puts the plaintext on the system clipboard and starts a short timer
(a few seconds). When the timer fires, BlueFlame checks whether the clipboard still holds exactly
what it put there and, if so, overwrites it with an empty string, so a value Daniel forgot about
does not sit in clipboard history indefinitely.

## Recovery code

At setup, after the TPM key is created, BlueFlame generates a strong, passphrase grade secret
(a long random value shown as a word list, similar to a diceware phrase) and shows it to Daniel
exactly once, with a clear warning to write it down somewhere offline and separate from the
laptop. It is never stored anywhere in BlueFlame, on disk or otherwise. Losing it is fine as long
as the TPM key keeps working; it only matters on the day the TPM key stops working.

The code is stretched with Argon2id (a slow, memory hard function built for exactly this: turning
a human memorable or human copyable secret into a strong key without being cheap to brute force)
using the salt and parameters stored in `locker_meta`. The result derives the recovery KEK, which
can unwrap `wrapped_data_key_recovery` at any time, and can also be used to re-wrap the data key
under a freshly provisioned TPM key after a TPM reset, so Daniel is not locked out permanently by
a hardware change.

## Passkeys

Where a site offers a passkey instead of a password, BlueFlame should prefer it. WebView2 is
Chromium based and already implements WebAuthn, with Windows Hello as the platform authenticator,
so this mostly means surfacing the option clearly in the UI (detecting that a site supports
passkeys and suggesting one) rather than building new cryptography. Passkeys are phishing
resistant in the same way the locker's own autofill is: the browser binds the credential to the
real origin, so a lookalike site cannot obtain or trigger it. The locker only stores metadata for
passkeys (which account exists on which origin), never key material, since the private key lives
inside the TPM through Windows' own WebAuthn platform authenticator, outside BlueFlame's control
entirely.

## Windows APIs and Rust crates to use

- **NCrypt / CNG** (`windows::Win32::Security::Cryptography`, targeting the Microsoft Platform
  Crypto Provider): the mechanism this design uses to create and use the TPM backed key,
  reached from Rust through the `windows` crate's Win32 bindings. This is the primary path, not
  a fallback: the key must support decrypt so BlueFlame can wrap and unwrap the data key
  directly, and only the raw NCrypt surface, not `KeyCredentialManager`, exposes that. The key
  is created non-exportable with an `NCRYPT_UI_POLICY` set to "force high protection," so
  Windows itself pops the Hello prompt on every private key operation, without BlueFlame drawing
  its own prompt UI (which page script could otherwise try to spoof).
- **Windows Hello for Apps** (`Windows.Security.Credentials.KeyCredentialManager`, a WinRT API)
  is deliberately not used for the data key path: it only supports signing, not decrypt, which
  is what led to the unsound signature-derived KEK in an earlier draft of this document. It is
  not part of the current design; if a future need for a sign-only, Hello gated key comes up
  (proving to a remote party that Daniel unlocked, for example), it can be added separately
  without touching the data key path above.
- `aes-gcm`: AEAD encryption for every entry's `encrypted_blob`, and for the recovery path's
  wrap of the data key (see "Recovery code" below).
- `argon2`: stretching the recovery code and the export passphrase into keys.
- `rand` (via `OsRng`): generating nonces, the data key, and the recovery code's underlying
  entropy.
- `zeroize`: clearing the data key and any decrypted entry plaintext from memory as soon as each
  is no longer needed.
- `rusqlite`: already a BlueFlame dependency, used for the two new tables above.
- A small word list crate (for example `bip39`, used only for its word list and encoding, not its
  wallet semantics) to render the recovery code's random bytes as a word phrase Daniel can write
  down and type back in, rather than a raw hex string.

## Fake store for tests

All TPM and Windows Hello access goes through a small Rust trait (something like
`LockerKeyStore`, with methods for provision, wrap, unwrap, and delete), so tests run against an
in-memory fake implementation instead of touching Daniel's real TPM, his real BlueFlame CA, or
his real Windows Hello enrollment. Any test that does need a real CNG or TPM backed key for an
integration check creates it with a name prefixed `blueflame-test-` and deletes it before the
test ends, so nothing test related is left behind in the TPM.

## Phased build plan

- **Phase 1 (S): key provisioning.** The `LockerKeyStore` trait, the NCrypt backed
  implementation against the Microsoft Platform Crypto Provider (the primary path; see "How
  unlock works" for why `KeyCredentialManager` is not used here), the software key fallback for
  machines without a usable TPM, and the fake used in tests. This phase's acceptance check is the
  wrap and unwrap round trip described above, across two separate Hello prompts. No UI yet.
- **Phase 2 (S): storage and wrap or unwrap.** The two new SQLite tables and the data key wrap
  and unwrap logic (direct RSA-OAEP through the TPM key, no signature or HKDF step), unit tested
  end to end against the fake store.
- **Phase 3 (M): entry management.** Tauri commands and a settings style UI to add, view (behind
  another Hello prompt), edit, and delete entries. No autofill yet, so this phase alone is
  already useful as a manual locker.
- **Phase 4 (M): autofill.** Exact origin matching, the user gesture gated fill trigger, and the
  privileged fill path through WebView2's scripting bridge.
- **Phase 5 (S): auto-lock and clipboard self-clear.** The idle timer that zeroes the data key,
  and the self-clearing clipboard copy action.
- **Phase 6 (S): recovery code.** One time generation and display, Argon2id stretching, the
  second wrap of the data key, and the re-wrap flow after a detected TPM reset.
- **Phase 7 (S): encrypted export and import.**
- **Phase 8 (S): passkey UI.** Detecting passkey support on a site and suggesting it over a
  stored password; storing passkey metadata rows.
- **Phase 9 (S): Claude channel hardening.** Adding the locker's routes and commands to the
  control channel's hard block list, and a focused security pass once the phases above are code
  complete, before anything ships as the default password store.

Sizes use the same S, M, L convention as the rest of `ROADMAP.md`. Phases 1, 2, and 5 through 9
are small enough to land as one pull request each; phases 3 and 4 are medium and will likely each
split into two or three smaller pull requests as they get built.

## Open questions for Daniel

- Auto-lock timeout: is a few minutes the right default, or should it be shorter given the July
  incident?
- Should viewing an existing password (not just filling it) always require a fresh Hello prompt,
  even mid session, rather than only the once-per-unlock gate?
- Recovery code format: a word list phrase, or would Daniel rather have a printable QR code to
  store instead?
