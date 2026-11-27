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

### 1.7 A copied image goes through `files::copy_png` on Windows
- **Rule:** to put a picture on the clipboard, call `ui::files::copy_png` (in `blocking`) on Windows; `set_clipboard_image` is for the other platforms only.
- **Why:** NSH 0.7 on Windows writes only the registered `PNG` clipboard format. "Sao chép ảnh" said it had copied, yet pasting into Paint, Zalo or Telegram gave nothing: they read the bitmap (`CF_DIB`/`CF_DIBV5`). `arboard` writes the bitmap and the PNG together.
- **Check:** after pressing the button in the real app, `[System.Windows.Forms.Clipboard]::GetDataObject().GetFormats()` in PowerShell lists `Bitmap` and `DeviceIndependentBitmap`, and Ctrl+V in Paint shows the card.

- **Also:** each `#[cfg(target_os = ...)]` branch must pass clippy on its own. The local gate only compiles the Windows one; the non-Windows `put_on_clipboard` took an `Arc` by value it did not consume and CI failed on Linux and macOS (`needless_pass_by_value`). Give both branches the same borrowed signature and read the other branch line by line before pushing.

### 1.8 A wrapping row centres its lines with `content_center`
- **Rule:** a row with `flex_wrap()` and a fixed or minimum height also gets `.content_center()`; `items_center` alone centres items inside a line, not the lines inside the row.
- **Why:** the Phân tích tables (`ui/breakdown.rs`) packed each symbol and its figures against the rule above, with the empty part of the 40 pt row below them; the user read it as a broken symbol column.

### 1.9 Focus a field the press rebuilt through an `ElementRef`, a frame later
- **Rule:** to give the keyboard to a field of a form that a button rebuilds (a blank form for "Ghi lệnh bỏ lỡ"), point an `element_ref()` taken at the page root at the input (`controls::text_field_ref`), and in the press `spawn` a task that awaits `next_frame()` and calls `focus()`.
- **Why:** pressing "Ghi lệnh bỏ lỡ" on a form that was already blank rebuilt an identical form, so nothing seemed to happen. `auto_focus(true)` on the rebuilt input did not take the focus from the button that was pressed.

### 1.10 An editor is made again only when what it is changes
- **Rule:** a `dynamic` that builds a value editor from a row (`properties.rs`, the property list) keys on what the editor *is* (the field, its kind, its name) through a `computed`, not on the whole row. Data the editor shows that its own handlers change (a select's options) is read live inside it (`Slot::options`). A handler that changes the row reads what it needs first and writes through the list, not through state of the editor it may replace.
- **Why:** adding an option in a "Chọn nhiều" property changed its definition, the whole row was read again, and the editor (with its add field) was made again under its own Enter handler: the field lost the focus and the handler went on with the old, disposed slot.

### 1.11 A file path handed to the OS is built with `Attachment::file_in`, never `join(&a.path)`
- **Rule:** the stored `Attachment.path` is `images/<hash>.png` with `/`. Turn it into a file path with `attachment.file_in(store)`, which joins segment by segment. In tests, compare paths sent to the platform with `as_os_str()`, not `==`.
- **Why:** "Hiện trong thư mục" in the image viewer failed with "An attempt was made to reference a token that does not exist". `dir.join("images/x.png")` gave `…\attachments\images/x.png`; NSH's Windows `reveal` calls `ILCreateFromPathW`, which returns null on a mixed path, and the error read was a stale `GetLastError` (1008). `ShellExecuteW` (open) accepts it, so only reveal broke. The test passed because `Path`'s `==` reads `\` and `/` alike on Windows.
- **Check:** `rg -n 'join\(&?\w+\.path\)' app/src crates` prints nothing; `a_picture_can_be_opened_with_another_program_and_shown_in_its_folder` fails on Windows if a `join(&item.path)` comes back.

### 1.12 A curve's axes fit what the trader looks at: values by `value_axis`, points one trade apart
- **Rule:** a curve's y scale gets its domain and its labels from `ui::chart::value_axis` (round ends around the values, not anchored at zero for a balance), passed to the plot's `y_scale`, the axis's `tick_values` and the grid's `y`. Points go one trade apart (`chart::TRADE_SPACING`), never by time. A point that is no state of the trading (the empty balance before an account's first deposit) is left out of the curve in `metrics::equity_curve`, not hidden by the chart.
- **Why:** on a $1,000,000 demo account funded inside the 30-day period, the curve started at the $0 before the deposit; the balance line sat flat at the top of a $0–$1,400,000 axis and the few thousand dollars it moved were invisible. Points spread by time put every trade of the last two days at the right edge, so the drawdown was one vertical drop there.
- **Check:** `a_balance_near_a_million_fills_the_plot_instead_of_starting_at_zero` (`ui/chart.rs`) and `an_account_funded_in_the_period_starts_at_its_deposit_not_at_zero` (`crates/core/tests/metrics.rs`).

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
- **The system clipboard is the developer's too:** `support::Fixture` sets `share.clipboard = memory` (`ui::files::CLIPBOARD_SETTING`), so "Sao chép ảnh" decodes the card but writes nothing to Windows' clipboard. Anything new that writes the clipboard reads that setting first.
  - Why: during a full quality-gate run, `a_trade_is_shared_as_a_picture_that_follows_the_privacy_choice` failed with "Không ghi được ảnh vào clipboard." because another process held the clipboard. Waiting longer (2be1cc4 raised it to `WAIT`) could not help, and every run overwrote what the developer had copied.
  - Check: `copies_stay_off_the_system_clipboard_only_when_the_setting_says_memory` in `ui/files.rs`; with Paint holding the clipboard open, `cargo test -p nsh-app-tradelens --test share_screens` still passes.
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
   - **Screen access to a dev build:** ask computer-use's `request_access` for `nsh-app-tradelens.exe` (the basename with `.exe`). `TradeLens` grants the user's installed release instead, and the dev build's window stays masked in every screenshot.
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
- **The data folder is asked for on the UI thread only.** `Store::database_path()` (and `mt5::folder`, built on it) asks NSH for the data directory, which panics on a `blocking` thread ("no reactive runtime: signals are used on the engine thread"). Compute the path before `spawn`/`blocking` and move it in. Why: looking for the app's own Python and preparing a real (non-demo) sign-in both called it on a blocking thread; the panic was swallowed and the card stayed on "Đang kiểm tra…" forever. Headless tests never reach those paths (no Python, demo mode), so only the real app showed it. Check: start the real app with a scratch `APPDATA`, open Kết nối MT5, read the log for `panicked`.
- **MetaQuotes' installer writes outside any scratch folder.** `mt5setup.exe /auto` (build 6327) ignores `/path:` and installs into `C:\Program Files\MetaTrader 5`, elevates itself (UAC; no prompt at all where admins auto-approve, as on the developer's machine), opens the terminal (which writes `%APPDATA%\MetaQuotes` of the real user, not the app's scratch `APPDATA`) and opens an MQL5 registration page in the default browser. A scratch `APPDATA` does not contain it, so the app copies the installed folder into `mt5\metatrader` and removes MetaQuotes' copy with its own `uninstall.exe /auto` (`installer::run`). Why: probing `/path` twice put MT5 in Program Files on a machine that had none. Check: after a real-app check of "Cài MetaTrader 5 tự động", `C:\Program Files\MetaTrader 5\terminal64.exe` must be gone (MetaQuotes' uninstaller leaves only an empty `Config` folder); otherwise run that `uninstall.exe /auto` by hand. Move `%APPDATA%\MetaQuotes` aside if the machine had none before.
- **What moves every poll is not written every poll.** Equity is stored at most every 5 minutes and the risk measured at entry is kept (`keep_measured`); otherwise every poll wrote a sync change and bumped the revision, so every page read again every 2 s.
- **The account's terminal starts with Algo Trading on.** The worker writes `tradelens-start.ini` (`[Experts] Enabled=1`, `AllowLiveTrading=1`, `Account=0`) beside the portable copy and starts it with `/config:`; before `order_send` it checks `terminal_info().trade_allowed` and says so in Vietnamese. Why: closing or moving SL/TP failed with "10027 AutoTrading disabled by client"; a fresh copy has Algo Trading off, and changing the login turns it off again unless `Account=0`. Check: `test_algo_trading_off_is_said_before_any_order_is_sent` and, on the real app, closing part of a demo position works right after the first sign-in.
- **The account's copy carries the fullest `servers.dat` on the machine.** An installed terminal keeps the broker list it downloaded in its data folder (`%APPDATA%\MetaQuotes\Terminal\<hash>\config\servers.dat`, 666 KB on the developer's machine), not beside the program (`Config\servers.dat`, 32 KB from MetaQuotes' installer). `install::portable_copy` gives the copy the largest list among the program folder's and every data folder's, on every sign-in, and never a smaller one than the copy's own. Why: the copy of the program folder alone did not know `Exness-MT5Trial7`; the terminal opened no connection at all and logged nothing, not even after "Login" with the saved password, so the Python package's `initialize` asked "ready?" (`0x6d`) until "(-10005) IPC timeout". The path-case fix below was not the cause of that timeout. Check: on the real app, the account's terminal logs `authorized on <server>` and `Get-NetTCPConnection -OwningProcess <pid>` lists the broker; `a_copy_takes_the_server_list_an_installed_terminal_downloaded`. To see what the package does over its pipe, hook its imports (`CreateFileW`, `WriteFile`, `ReadFile`, `CryptHashData`) with ctypes, with buffers declared `c_void_p`, never `LPWSTR` (ctypes copies it and the real call writes into the copy).
- **The terminal is started and `initialize`d by one and the same absolute path, letter case kept.** `mt5.initialize(path)` finds a running terminal by the exact path it was started with. Why: the worker started it `os.path.normcase`d (lowercase); `initialize` did not find it, started a second terminal that logged "terminal process already started" and exited, and every login failed with "IPC timeout" / "(-10001) IPC send failed" while the terminal never opened a connection to the broker. Check: the account terminal's `logs\<date>.log` shows its folder in the original case and no "already started" line; `test_the_login_uses_the_same_absolute_path_as_the_start`.

## 6. Git, PRs, parallel agents

- **Two sessions in one checkout build in separate target folders.** Give the second one `CARGO_TARGET_DIR=target-<name>` and `CARGO_BUILD_JOBS=4`. Why: the Stop hook's `quality.sh` and another session's `cargo clippy` wrote the shared `target/` at once; rustc ran out of memory (`STATUS_STACK_BUFFER_OVERRUN`, os error 1453) and left metadata both sides then refused (E0786 "found invalid metadata files"). Check: `Get-CimInstance Win32_Process` lists one cargo per target folder.

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
