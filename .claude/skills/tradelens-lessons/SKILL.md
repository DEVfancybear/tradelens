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

### 2.1 A rule about stored data is checked inside the write, not only on the snapshot
- **Rule:** a check that depends on what is stored (a name already in use, say) runs again inside the `writer().run` closure, against `db`. The check on `store.snapshot_now()` before it is only quick feedback. Return the user's message as an inner `Result` (`Ok(Err(message))`) and `flatten()` it in `done`, so it is shown as written rather than as "Không ghi được dữ liệu: …".
- **Why:** Playbook's Save checked for a duplicate name only against the snapshot. When Save was pressed before the list had been read (a loaded machine), the snapshot was empty and a second "breakout" was stored next to "Breakout". `a_second_playbook_cannot_take_a_name_in_use` failed under load on the quality gate.
- **Check:** a test that stores the conflicting row through `fixture.db()` after the page has read its list (the screen doesn't see it), then saves through the form. See `a_name_stored_after_the_list_was_read_is_still_refused`.

### 2.2 A question to the database is a `Store::read`, never a `Writer::run`
- **Rule:** a read that only informs a decision (how many values a type change would lose) goes through `store().read(op, done)`. `Writer::run` calls `data_changed()` whenever its closure succeeds, so every page reads again for nothing, and in the middle of a flow (a confirmation waiting for its answer).
- **Why:** the form editor asked "what would be lost" through `writer().run`; the changing-a-type test failed twice under load in full runs (the stored values never followed the new type) and not once in 40 runs alone. The extra re-read is the suspect; the test now also chooses the kind again until it is stored.
- **Check:** `rg -n "writer\(\)|write\.run" app/src` and look at each closure: does it store anything?

### 2.3 A detail built from a stored row follows the row; a form opens from the row as it is stored when pressed
- **Rule:**
  - A `when(cond, || untracked(|| build(current.get())))` builds its branch again only when `cond` flips, not when what it read changes. For a detail of a stored row, use `dynamic` over a `computed` of the row (a memo is compared with `==`, so a re-read that changed nothing builds nothing), and leave out of that memo what a child follows on its own (a gallery's pictures), so adding a picture does not close its viewer.
  - A button that opens a form or writes a change reads the row when it is pressed (`current.get_untracked()`), never a clone made when the row was built.
- **Why:** on Playbook, Sửa then Lưu left the old name and description on screen, and the next Sửa opened the form with the previous data. The detail was built once inside `when(current.is_some, untracked(..))`; the save closed the form before the snapshot was read again, so it was built from the old row and never again, and "Sửa" held a clone of that old row.
- **Check:** `an_edited_playbook_shows_its_new_data_at_once_and_opens_from_it` edits twice without leaving the page and asserts the detail, the form's fields and the database each time. Reverting `detail` to the `when` form makes it fail.

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
- **"invalid metadata" / "can't find crate" on Windows:** read the `note:` lines before blaming the cache.
  - `os error 1455` ("The paging file is too small") means rustc ran out of virtual memory: twelve parallel jobs each linking a test binary with full debug info.
  - The cache is fine, so deleting `target` only costs a full rebuild. Build with `CARGO_BUILD_JOBS=4 cargo test --workspace --no-run` first, then run `scripts/quality.sh`, which reuses that build.
  - Also don't run `quality.sh` in the background as a turn ends. The `Stop` hook runs it at the same time over the same `target`.
- **The OS credential store is the developer's, not the test's:** `support::Fixture` sets `cloud.credentials = memory` (`CREDENTIALS_SETTING`), so sessions and secrets stay in memory. Anything new in `app/src/cloud.rs` that keeps a token goes through `Env::token_store()` / `Env::{load,save,clear}_secret`, never `KeyringStore::new()` directly.
  - Why: after the user signed in to Google in the real app, four screen tests failed ("nobody is signed in" was false). They read the real Windows Credential Manager, and a test pressing "Đăng xuất" would have deleted the user's real token.
  - Check: `credentials_kept_in_memory_never_reach_the_operating_system_store` in `cloud.rs`; with a real sign-in on the machine, `cargo test -p nsh-app-tradelens --test cloud_screens` still passes.
- **Temp dirs:** every test database lives in a `tempfile::TempDir` through `Fixture`, so it is removed afterwards. Leaked `.tmp*` folders once filled the disk.
- **Regression tests:** before trusting a new test, revert the fix, confirm the test fails, then restore the fix.

### 3.1 A loopback HTTP listener reads the whole request head, then closes gracefully
- **Rule:** a hand-made listener (the OAuth redirect in `crates/cloud/src/auth.rs`) reads until the blank line (`\r\n\r\n`), with the total time and size bounded, before it replies. After writing it calls `shutdown(Shutdown::Write)` and drains the input for a moment before the stream is dropped. Never reply after reading only the request line.
- **Why:** closing a socket that still holds unread input makes the OS send RST instead of FIN. On macOS CI the reset beat the reply, so the client got `ConnectionReset` on its first request and the sign-in test timed out; a real browser could likewise miss the "Đăng nhập thành công" page.
- **Check:** the test client sends the request line, pauses, sends the headers, pauses, and only then reads the reply (`visit` in `crates/cloud/tests/cloud/auth.rs`). Reverting the fix makes it fail every time, on Windows too.

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
   - **Never call `open_application` on the app**, even to bring it forward: it started a second instance on the user's real data folder while their own build was running on it. Bring the window forward with PowerShell `(New-Object -ComObject WScript.Shell).AppActivate(<pid>)`.
   - To check without touching the real data at all, start the exe with `APPDATA` pointing to a scratch folder (NSH takes the data folder from it), plus `TRADELENS_CLOUD=memory` (no real sign-in, secrets in memory) and `TRADELENS_MT5=memory` (the broker in memory; any login opens a sample account).
4. **Compare with the design:** compare each screen with `docs/design/screens/<screen>.html` and `screenshots/<screen>.png` at full resolution, zooming into each region. Check hover, the cursor, the date-picker popup, the enlarged image viewer, and both light and dark modes.
5. **Restore:** close the app, rename the tested data folder to `.after-test-*`, and copy the backup back in place.
   - **Rule:** wait until `tasklist` no longer lists `nsh-app-tradelens.exe`, rename, then check that
     `vn.dat.tradelens` is gone before copying. Copy only to a path that does not exist (`[ ! -e dst ] && cp -rp backup dst`),
     then `diff -rq backup vn.dat.tradelens` must print nothing.
   - **Why:** right after `taskkill /F`, Windows still held the database files. Git Bash's `mv` fell back to copy and
     delete, and left the test session's `tradelens.db-wal` in `vn.dat.tradelens`. `cp -r backup vn.dat.tradelens`
     then copied the backup *into* that folder as a subfolder. The app opened on the test data (a fake feed, dark mode)
     while the report said the data was restored.
   - **Check:** after restoring, `ls -la vn.dat.tradelens` shows no subfolder named after the backup, and the diff is empty.

## 5.1 The MT5 connection (`crates/mt5`, `app/src/mt5.rs`)

- **An order the app gave up on is never sent later.** Each order carries a claim the link's thread and the asker race for (`Claim` in `link.rs`): not taken within `order_queue`, it is dropped and reported "chưa được gửi"; taken but unanswered, it is `Mt5Error::Uncertain` ("không rõ kết quả"), never a plain failure. Why: the thread reads orders only between polls, so an order sent while it logged in ran minutes later, and a second press closed twice the volume. Check: `an_order_given_up_while_the_link_logs_in_is_never_sent_later`.
- **Events are tagged with the link they come from.** Each start gets a new link id; what an older link of the account still says is ignored, and a new link starts only after the old one has stopped (`Link::stop_and_wait`), so two workers never drive one portable terminal. Why: a late `Refused`/`Connected` of the old link dropped or saved the wrong password.
- **A failed read is an error, never "nothing".** The worker raises when `positions_get()` returns `None`; read as "no positions", every open position came back as new at the next poll and put a plan prompt on screen.
- **What moves every poll is not written every poll.** Equity is stored at most every 5 minutes and the risk measured at entry is kept (`keep_measured`); otherwise every poll wrote a sync change and bumped the revision, so every page read again every 2 s.

## 6. Git, PRs, parallel agents

- **Before pushing to a PR,** check that it is still open: `gh pr view <n> --json state`. It may have been merged while you worked; then open a new PR instead of pushing to a merged branch.
- **While waiting for CI to merge a PR,** don't push more commits to its branch. Commit locally and push to a new PR once it has merged.
- **To merge,** use `gh pr merge --match-head-commit <sha>` so only the commit CI checked gets merged.
- **Merge conflicts:**
  - For `Cargo.lock`, take ours, then run `cargo metadata` to regenerate it.
  - For `mod` lists or `shell.rs`, keep both sides.
- **Don't switch branches in the main checkout** (another session uses it); work in a worktree.
- **No bare `git stash`;** the stash is shared by all worktrees.

## 7. Dates and "today"

### 7.1 "Now" is on the same scale as the stored times
- **Rule:** a stored trade time is the server's wall clock when the account's server offset is known, else the trader's own wall clock (manual entry and Notion import keep what was typed). Anything that means "now" next to stored times (today, range cutoffs, the planned trade of the entry dialog, `written_at`) goes through `zone::stored_now`. Never add `server_offset_min.unwrap_or(0)` to a UTC "now", and never derive "today" from `Utc::now().date_naive()`.
- **Why:** with no offset, "today" was the UTC date. At UTC+7 the Nhật ký ngày page opened on yesterday between 00:00 and 07:00, "Hôm nay" went to yesterday and a note typed then was saved under yesterday. CI runs in UTC, so it never showed.
- **Check:** `zone::tests` assert the day for a late-UTC instant with injected offsets, so they do not depend on the machine's zone. To reproduce an integration test's failure on old code, run it between 00:00 and 07:00 at UTC+7 (or temporarily make `stored_now` ignore the local offset).

### 7.2 A test that seeds "now" puts its account on a UTC server
- **Rule:**
  - A headless test that stores trades at `now_ms()` or `Utc::now()` and then expects them on today, this week or this month calls `server_clock::on_utc(db)` (`app/tests/server_clock`) after the batch is stored. The app then reads them as UTC and shows them on the machine's clock, so they are today in any zone.
  - Expected dates in such a test come from `Local::now()`. A test with no account at all also counts on the machine's clock (`Local`); never `Utc::now().date_naive()` for a day, week or month the page computes.
  - A test seeding only 1 to 3 days back and checking the 7 or 30 day range needs nothing.
- **Why:** the overview test seeded trades at the UTC clock on an account with no server offset. Between local midnight and UTC midnight (00:00 to 07:00 at UTC+7) the page's "today" was a day later than the trades', so the discipline card read `0 / 5` instead of `2 / 5`. It passed on CI (UTC) and on a developer's machine by day. The week review tests had the opposite slip: they computed the week from UTC while the page used the machine's clock, which fails on Monday mornings at UTC+7.
- **Check:** run the screen tests between 00:00 and 07:00 at UTC+7 (or on the first hours of a Monday, or of a month), and `rg -n 'Utc::now\(\)\.date_naive' app/tests` must print nothing.
