# AI Capsule Engineer Checklist

Use this checklist before shipping any AI-assisted capsule inspection feature.

## Architecture

- [ ] Feature is implemented outside Core.
- [ ] Core does not depend on AI provider code, prompts, or plugin package.
- [ ] WASM/plugin layer does not receive direct network, keychain, or ledger
      write access.
- [ ] AI output is advisory only and cannot create ledger truth.
- [ ] User-facing Capsule Doctor and Developer Hivra Engineer are separate
      modes.
- [ ] Plugin Auditor and Plugin Scaffolder modes are separate from capsule
      diagnostics.
- [ ] Plugin Scaffolder writes only draft files until human approval.

## Secret Safety

- [ ] Provider API key is stored only in platform secure storage.
- [ ] No plaintext fallback file exists.
- [ ] Snapshot builder redacts seeds, private keys, exchange credentials, and
      keychain/keystore material.
- [ ] Denylist includes `.env`, `*.pem`, `*.key`, `capsule_seeds.json`, and
      exchange credential files.
- [ ] Denylist regression tests include realistic Hivra paths.
- [ ] AI provider key, GitHub token, and plugin credentials are separately
      scoped and cannot be reused across scopes.
- [ ] User can clear stored AI credentials.

## Snapshot Determinism

- [ ] Capsule snapshot has a schema version.
- [ ] Snapshot canonical JSON is stable.
- [ ] Snapshot hash is shown in diagnostics.
- [ ] Same input capsule state produces the same snapshot hash.
- [ ] Wall-clock time is either excluded or carried only as metadata outside
      deterministic diagnosis inputs.
- [ ] Snapshot size is bounded.
- [ ] Large ledgers/logs are summarized by facts and hashes.
- [ ] Binary files are never sent to the provider.
- [ ] Report records included/excluded section counts.

## Transport Diagnostics

- [ ] Snapshot includes delivery outbox summary.
- [ ] Snapshot includes transport receipts.
- [ ] Snapshot includes pending invitations and terminal lifecycle status.
- [ ] Diagnosis distinguishes relay acceptance from peer ledger confirmation.
- [ ] Timeout, relay reject, auth failure, and missing peer fetch are separate
      findings.

## Consensus Diagnostics

- [ ] Snapshot includes pair consensus status and hash.
- [ ] Snapshot includes relationship facts by peer/starter.
- [ ] Strict conflict facts can be supplied when lifecycle histories diverge.
- [ ] AI report cannot mark consensus as stronger than supplied evidence.

## Developer Mode

- [ ] Developer Mode is explicit and disabled by default.
- [ ] Repository path is user-provided.
- [ ] Repo scanner uses allowlist paths only.
- [ ] Repo scanner refuses denylisted paths.
- [ ] Selected file snippets include paths and hashes.
- [ ] Full repository dumps are forbidden.
- [ ] Prompt injection from files/logs/manifests is treated as untrusted data.
- [ ] Remote repository links are explicit allowlist entries.
- [ ] Public remote repositories can be read without GitHub token, but only
      through host-selected files.
- [ ] Private remote repositories require a secure-storage GitHub token.
- [ ] Remote repository context is pinned to commit/tag or marked mutable.
- [ ] Developer workspace bootstrap supports app/plugin repo links without
      maintainer private keys.
- [ ] Repository cache clone does not execute scripts or hooks.
- [ ] Repository cache can be cleared by the user.
- [ ] Cloned repositories are treated as hostile input.
- [ ] Symlinks escaping repo cache are ignored.
- [ ] Submodules/nested repositories are not fetched unless explicitly
      allowlisted.
- [ ] Large files are skipped by default.

## Plugin Auditor

- [ ] Plugin manifest is inspected without granting new capabilities.
- [ ] Package/catalog SHA-256 and signature status are included when available.
- [ ] Capability mismatch findings are advisory and fail closed.
- [ ] Runtime invocation evidence is linked to plugin id, method, and response
      hash.
- [ ] Auditor cannot mutate registry, catalog, package files, or ledger.

## Plugin Scaffolder

- [ ] Scaffolder is available only in Developer Mode.
- [ ] Target plugin id and capability list are explicit.
- [ ] Generated files are placed under a draft path.
- [ ] Generated code contains no secrets or embedded credentials.
- [ ] Generated plugin has deterministic tests and at least one golden vector.
- [ ] Catalog update, package build, signing, commit, push, and release remain
      separate human-confirmed steps.
- [ ] Generated code is not built, executed, installed, or published
      automatically.

## UX

- [ ] User can inspect what data will be sent before sending.
- [ ] User can clear stored AI credentials.
- [ ] Report separates user actions from developer findings.
- [ ] Errors are actionable when secure storage or network provider calls fail.
- [ ] Cost/rate-limit failures do not affect capsule state.
- [ ] No automatic background uploads.
- [ ] User can delete local reports.
- [ ] User can clear repository cache.
- [ ] Capsule Doctor shows a local deterministic diagnostic summary when AI is
      unavailable.

## Failure Handling

- [ ] Invalid AI key leaves capsule state unchanged.
- [ ] Provider timeout/rate-limit leaves capsule state unchanged.
- [ ] Repository clone failure leaves capsule state unchanged.
- [ ] Redaction failure blocks provider upload.
- [ ] Oversized snapshot is rejected or summarized before upload.
- [ ] Malformed model response is rejected.
- [ ] Model-suggested forbidden actions are displayed as blocked, not executed.

## Release Gate

- [ ] Unit tests cover redaction.
- [ ] Unit tests cover denylist enforcement.
- [ ] Unit tests cover deterministic snapshot hashing.
- [ ] Unit tests cover prompt-injection treatment for logs, manifests, source
      comments, and chat messages.
- [ ] Unit tests cover remote repo cache path traversal/symlink rejection.
- [ ] Unit tests cover malformed AI response handling.
- [ ] Manual smoke covers Capsule Doctor without repo access.
- [ ] Manual smoke covers Developer Mode with repo allowlist.
- [ ] Manual smoke covers Plugin Auditor on an installed plugin.
- [ ] Manual smoke covers Plugin Scaffolder draft generation without catalog
      mutation.
- [ ] No new review gate violations.
