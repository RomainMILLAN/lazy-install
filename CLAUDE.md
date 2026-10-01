# lazy-install — CLAUDE.md

A TUI listing applications, telling which need an update (`needs_update`), and
running their `update` in an embedded PTY. Sibling of lazy-transfer (lazy-scp) and
lazy-aws: same loop (sync Rust, threads + mpsc, 50 ms crossterm poll), **not** the
same `App` struct.

## Commands

```sh
cargo test --locked                                  # unit + e2e (real bash, real PTY)
cargo clippy --locked --all-targets -- -D warnings
cargo fmt -- --check
cargo +1.94 check --locked --all-targets             # MSRV
```

Release: bump `version` in Cargo.toml, commit, `git tag -a vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z`.
The workflow checks the tag against Cargo.toml, builds Linux x86_64/aarch64, publishes `SHA256SUMS`.

## Architecture

```
catalog/   domain, no deps: Application, AppName, Slug, ScriptRef, Catalog (immutable,
           with_added/with_edited/with_removed -> Result), AppRuntime (CheckState ×
           UpdateState, gardes can_*, tag() priority table, sole issuer of generations),
           outcome VOs (DisplayText, Versions, ExitOutcome, CheckOutcome, Tag)
session/   application layer: Session = single entry of intentions -> Result<Vec<Effect>,
           Refusal>; apply(Fact) -> Vec<Effect>; UpdateQueue = pure state machine, sole
           writer of UpdateState. Imports nothing from ui/pty/jobs/config (tests/architecture.rs)
script/    anticorruption towards bash: contract (PRELUDE, TEMPLATE, codes, token), trust
           (judge_chain pure + TrustedScript), validate (static), command (CommandSpec), check
pty/       Screen (newtype, closure access), key_to_bytes, LogSink, PtySpawner, PtyRun
jobs/      ports (CheckRunner, UpdateSpawner, ActiveUpdate on &ScriptRef), CheckScheduler
           (pool, carries generations), UpdateRunner (active run + RunRecord per app)
config/    repository: ConfigStore (strict load, atomic save under flock, sha256
           fingerprint), DTO, paths
cli/       headless `--check`: collect() schedules every check, reads facts until the
           channel closes (or a budget from the runner's worst_case()), projects a
           CheckReport (pure; text + JSON through private DTOs). No ui/pty/script.
ui/        Tui { session, jobs, view, widgets }: routes keys to intentions, executes
           effects, feeds results back as facts; no domain decision
```

## Glossary

- **Application**: a name and a script. **AppRuntime**: what is known about it now.
- **Session**: ONLY the application layer. A **run** is one execution of `update`
  (`PtyRun`, `RunRecord`); the log holds the last run. POSIX/sudo sessions keep their
  system meaning.
- **Effect**: what the session wants done. **Fact**: what happened, never refused.
- **Contract v1**: marker `# lazy-install: v1`, `needs_update` + `update`, helpers.
- **Token**: `LI_RESULT␟nonce␟state␟installed␟latest` on fd 3. The nonce comes on fd 4.
- **Private group**: primary gid, named after the login, no other member.
- Tags: `error` = the check failed; `failed` = the update failed or never
  started; `invalid` = the script was refused.

## Invariants not to break

- **Up to date requires code 10 AND a valid token.** Never relax `contract::decide`: a
  failure must never read "OK".
- **Validation executes nothing** (user decision): static text checks + `bash -n`.
  `declare -F` is checked at run time in the prelude (exit 3 + `LI_INVALID` token).
- **`TrustedScript` is neither `Clone` nor stored**; only the adapters build it, right
  before the process, and consume it. Jobs carry `ScriptRef`.
- **Save first, apply after**: `Session` builds a candidate catalog, emits
  `Persist`, swaps on `PersistSucceeded`. `Persist` runs synchronously in the same
  tick: no intention can interleave. One pending change at a time (`Refusal::Busy`).
- **Tests never use `/tmp`** for files the trust rule judges (it refuses 1777, rightly):
  `tempdir_in(env!("CARGO_TARGET_TMPDIR"))` in `tests/`, `CARGO_MANIFEST_DIR` in unit
  tests, pure `judge_chain` for the rule itself. Never weaken the rule to pass a test.
- **Every pipe is `O_CLOEXEC`**; only the `dup2` to 3/4 in the child clears it.
- **`waitid(WNOWAIT)` → drain → `killpg` → `waitpid`**: the pgid cannot be reused
  between the observation and the kill.
- **Keys and filter/form text are never logged** (a key may be a sudo password).
- **lazy-scp's `load()` falls back to `Default`**: never copy that here; an unreadable
  config exits 2 and is never rewritten.
- **`--check` exit codes are a published contract** (0 update, 1 none found, 2 nothing
  checkable) read by the dotfiles' `notify.sh`, like the JSON field names. It runs before
  `logger::init()` (which truncates `debug.log`) and never touches the terminal.
- Only `theme.rs` names RGB values; hints derive from `KeyMap`, never written out.

## Brand

Validated in Claude Design, project **lazy-install — Brand**
(`https://claude.ai/design/p/d1fd39b5-bd95-458b-a821-8af1c875fdd4`). Any visual
change is proposed there first and integrated only once validated.

- Mark: two panes like the TUI — apps list (selected row violet) | update arrow
  on its base line. `docs/assets/logo-mark.svg` (dark), `logo-mark-light.svg`,
  `icon.svg` (filled tile). In the TUI: `ui/brand.rs`, `╭───┬───╮ │ ≡ │ ↑ │`,
  shown in the help popup and the empty list.
- Accent: violet `#A78BFA` dark / `#6D28D9` light (`theme::color_primary`).
  Tag colours carry meaning only (green OK, amber UPDATE/queued, blue
  checking/updating, red error/failed/invalid).
- Type: Space Grotesk (wordmark) + JetBrains Mono.
- Regenerate every image: `cargo run --example screenshots && bash docs/assets/src/render.sh`.
  Screens are rendered from the real widgets, never drawn by hand.
