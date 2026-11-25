---
name: tradelens-lessons
description: >-
  Lessons from bugs already fixed in TradeLens: NSH pitfalls (dialogs, owners, theme colours,
  text inputs, key bindings), typing lost when data is read again, headless tests that flake on
  CI, migrations, imports, checking on the real app, and the git/PR workflow. Load this before
  building or changing any screen, component or test in `app/`, before writing a migration or an
  import in `crates/core`, and before checking a change on the real app or pushing to a PR, even
  when the task looks small.
---

# Lessons from fixed bugs

Every item below is a bug that really happened here, with the rule that avoids it and how to check.
Read the relevant section before writing code. When you fix a bug whose lesson is not here yet, add
an item in the same shape: **Rule**, **Why** (the bug), **Check**.

## 1. NSH pitfalls

### 1.1 Never `dialog()`; use `ui::modal` or `confirm_then`
- **Rule:**
  - In-window dialogs (plan, share…) use `ui::modal`: `Modal::new(id, title, body, on_dismiss)`, then `modal::open` / `modal::close`. `modal::host()` is already mounted in `shell.rs`.
  - Confirming a destructive action uses `controls::confirm_then(owner, title, description, confirm_label, then)`, which is a system alert.
- **Why:** on Windows the native window layer of `dialog()` is broken. Confirmations auto-accepted, and modals showed only the dark scrim with no content.
- **Check:** `app/tests/no_native_dialog.rs` forbids `dialog()`.

### 1.2 Work that outlives a button runs under an owner that is not rebuilt
- **Rule:** a confirmation, background task or callback that runs after a press must run under the root owner or another stable one, not under the owner of a card or button that can be rebuilt. Take `Owner::current()` at the page root or parent component and pass it down, as `confirm_then` does.
- **Why:** "Đăng xuất OneDrive" did nothing. The cloud card rebuilt when the sign-in state changed, which cancelled the confirmation task running under its owner. `gallery` likewise runs `on_change` under the gallery's owner.
- **Check:** a test that presses the button, makes its container rebuild, then answers the question. See `a_question_outlives_the_button_that_asked_it` in `ui/controls.rs`.

### 1.3 Pass theme colours as a function or closure, never as a value
- **Rule:**
  - Write `.text_color(tokens::loss)`, `.bg(move || variant.fill())`, `.border_color(move || edge(&day))`.
  - Never `.text_color(tokens::loss())` or `.bg(theme().card)` in a builder.
  - When the colour depends on data, capture the data and pick the colour inside the closure.
- **Why:** reading `theme()` while building makes any `dynamic`, list or calendar holding the element track the theme, so switching the colour mode rebuilds the whole block. Within the same update the old component (a button, say) is also disposed before it is drawn, and NSH logs `a prop read or written after its owner was disposed` on the `nsh-engine` thread.
- **Check:**
  - `rg -n -e '\.(bg|text_color|border_color|outline_color)\([a-z_:.]+\([^)|]*\)(\.[a-z_]+)?\)' app/src` must print nothing. It found the 30 sites fixed in 130f355 and 10b731c.
  - Regression tests: `a_button_does_not_tie_the_dynamic_holding_it_to_the_theme` and `an_error_line_does_not_tie_the_dynamic_holding_it_to_the_theme`.

### 1.4 "Add" fields use `add_field`; fields that edit a value use `commit_field`
- **Rule:** a field typed into to add an item (checklist, reason, question, option) is `controls::add_field`. Enter adds, empties the field and keeps the focus; leaving the field adds too. Don't build a `text_field` next to an "Add" button.
- **Why:**
  - NSH's `text_input` defaults to `SubmitBehavior::BlurAndSubmit`, so Enter drops the focus. The checklist field also had no `on_submit`, so Enter did nothing at all.
  - A `commit_field` over an always-empty value keeps the text it just added, then adds it again when the field is left.
- **Check:** a test that types, presses Enter, types again with `press_key` without clicking the field, presses Enter, then leaves the field; each item must be added exactly once.

### 1.5 Keys that must work while typing need `When::Always`
- **Rule:** register a key that must fire with the focus in a text input (Escape closing a modal…) with `on_key_with(.., KeyOptions { when: When::Always, .. })`.
- **Why:** with `When::Auto`, NSH skips a binding for a key the focused input uses, so Escape did not close the modal.

### 1.6 Press targets only from `ui/controls`
- **Rule:** use `action`, `action_when`, `icon_action`, `link` and their siblings, with `Variant::Destructive` for deletes.
- **Why:**
  - Hand-made press targets lacked `cursor_pointer`; the user reported no hand cursor on hover.
  - One agent removed `Variant::Destructive` while another was using it.
- **Check:** before removing or changing a `pub(crate)` item in `ui/`, `rg` its uses on the integration branch.

## 2. Typing lost when data is read again

- **Rule:**
  - `Store::snapshot` is a `SignalEquals::Never` signal: every database re-read announces a change, even when nothing changed. Read the part you need through `computed(...)` (a memo compared with `==`), and build elements inside `untracked`, so the field being typed in is not rebuilt.
  - Keep drafts in a local signal. A form loaded from the database takes new data only while it is not `dirty`. `commit_field` follows the read value only while the user hasn't written over it.
- **Why:** the R field in manual entry, the day journal while it autosaves, renaming checklist items and renaming options in the form editor all lost keystrokes whenever a re-read landed between keys. The same bug made tests flaky on CI.
- **Check:** add a case to `app/tests/typing_survives_reloads.rs` using `type_through_reloads`, which types in pieces and forces a re-read between them.

## 3. Headless tests

- **Waiting:**
  - nsh-testing's `wait_for` has a fixed 5 s. Anything shown after a database read is waited for with `wait_for_within(sel, WAIT)` (`WAIT` = 30 s, in `tests/support`).
  - Conditions outside the app (database, files) are waited for with `wait_until`.
  - Never `sleep` for a guessed time.
  - Why: the CI runners are slow, so tests passed locally and failed on CI.
- **Lost presses:** a page redraws once its data is read, and a press that lands then hits a control being replaced.
  - Turn tags and options on with `common::toggle_on`, which presses again when a press is lost.
  - Before pressing, wait for a data-driven element (e.g. `role:button[Sửa]`) to appear.
  - After Escape, wait for the screen to reach a known state before pressing again.
- **Keyboard:** a harness that uses `press_key` must set `config.environment.os = Os::Linux`; without it `press_key` types nothing.
- **Swallowed panics:** NSH catches panics on the `nsh-engine` thread, so the test still passes.
  - After changing UI code, run `cargo test -p nsh-app-tradelens --lib -- --nocapture --test-threads=1 2>&1 | grep -c "panicked"`. It must print 0.
  - Such a panic is a real bug to find the cause of, not teardown noise.
  - To narrow it down fast, split the harness into small probe components.
- **Temp dirs:** every test database lives in a `tempfile::TempDir` through `Fixture`, so it is removed afterwards. Leaked `.tmp*` folders once filled the disk.
- **Regression tests:** before trusting a new test, revert the fix, confirm the test fails, then restore the fix.

## 4. `crates/core`: data and migrations

- **Migrations** change structure only; they never guess what old data means.
  - Example: don't infer "date-only trade" from a 00:00 time. Migration 0013 did that and was removed.
  - To mark missing data, record it at import time, like the `gaps` column (0012).
- **The Notion import is dynamic:**
  - Column roles are inferred from content: no column-mapping screen, no hardcoded column names.
  - It reads the `.zip` directly (`*_all.csv`; file names with broken encoding are ignored).
  - The user rejected both of the alternatives.
- **The legacy data folder** `%APPDATA%\default` is only copied (`core::legacy::adopt`), never modified or deleted.
- **The user's private data** (Notion export, real database) is for local checks only. Never commit it, and never copy its content into fixtures or docs.

## 5. Checking on the real app

The user is explicit: "kiểm tra = app thật" (check on the real app). Passing headless tests is not enough. Many UI bugs only show to the eye: the search box wrapping, narrow table columns, charts drawn wrong, empty modals.

1. **Build:** use a separate `CARGO_TARGET_DIR` per worktree, with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`.
   - Full debug info filled the disk.
   - A target dir shared across worktrees served stale builds.
   - The app package is `nsh-app-tradelens`.
2. **Back up:** copy `%APPDATA%\vn.dat.tradelens` somewhere else before anything that writes data.
3. **Launch:** start the exe from the shell (`(exe &)`). computer-use's `open_application` starts yet another instance. Maximise by double-clicking the title bar.
4. **Compare with the design:** compare each screen with `docs/design/screens/<screen>.html` and `screenshots/<screen>.png` at full resolution, zooming into each region. Check hover, the cursor, the date-picker popup, the enlarged image viewer, and both light and dark modes.
5. **Restore:** close the app, rename the tested data folder to `.after-test-*`, and copy the backup back in place.

## 6. Git, PRs, parallel agents

- **Before pushing to a PR,** check that it is still open: `gh pr view <n> --json state`. It may have been merged while you worked; then open a new PR instead of pushing to a merged branch.
- **While waiting for CI to merge a PR,** don't push more commits to its branch. Commit locally and push to a new PR once it has merged.
- **To merge,** use `gh pr merge --match-head-commit <sha>` so only the commit CI checked gets merged.
- **Merge conflicts:**
  - For `Cargo.lock`, take ours, then run `cargo metadata` to regenerate it.
  - For `mod` lists or `shell.rs`, keep both sides.
- **Don't switch branches in the main checkout** (another session uses it); work in a worktree.
- **No bare `git stash`;** the stash is shared by all worktrees.
