# Windows Wrenchmaster — working agreement

Read this before touching the UI. It exists because the interface drifted:
the same idea got built three different ways on three pages, and each new
page invented controls that already existed somewhere else.

**The rule that prevents most of it: before writing a widget, check whether
`chrome.rs` already has one. It probably does.**

---

## 1. What this app is

An offline Windows maintenance toolbox. Rust, `egui`/`eframe` 0.35, frameless
window, no network except where a feature is explicitly about the network.
Single binary, no installer, runs unelevated and elevates per action.

Hard constraints, not preferences:

- **Offline.** The app never depends on the network to do its job. Nothing
  blocks on a request, nothing waits on a timeout, nothing degrades or fails
  because a machine is disconnected, and nothing is sent anywhere on its own
  initiative — no telemetry, no update check, no license check, no avatar
  fetch. The original build phoned a license server at startup and that is
  the failure mode this rule exists to prevent.

  A URL is not a violation of this. A link the user clicks hands off to the
  browser and the app carries on; it is not in any flow and nothing waits on
  it. The Patreon button in Settings -> About is exactly that. What the rule
  forbids is the app reaching out by itself, or needing a reply to continue.
  `run_activation_scripts` genuinely does fetch and execute a remote script,
  which is why it is gated behind its own confirmation saying so.
- **Read before write.** Anything that measures runs before anything that
  changes. The cleaner scans before it deletes; diagnostics never repair.
- **Nothing destructive without a prompt.** See §6.
- **Never block the UI thread.** See §5.

---

## 2. The design system lives in two files

`src/theme.rs` holds tokens. `src/chrome.rs` holds widgets. If a value or a
control is not in one of those two, it does not exist yet — add it there
rather than inline on a page.

### Spacing — three steps, no ad-hoc numbers

| Token | Value | Use |
|---|---|---|
| `theme::GAP_TIGHT` | 4 | Inside one group, between a label and its value |
| `theme::GAP` | 8 | Between groups inside a card |
| `theme::GAP_SECTION` | 16 | Between sections of a page |

Never write `ui.add_space(12.0)`. If none of the three fit, the layout is
wrong, not the scale.

### Type — pick by role, never by pixel size

| Role | Use for |
|---|---|
| `theme::page_title()` | The page heading. `body()` already draws it — do not add a second |
| `theme::section_title()` | A section inside a page |
| `theme::card_title()` | The name of a card |
| `theme::eyebrow()` | A small grey label above a figure, and ALL-CAPS field names |
| `theme::readout()` | Any number. Monospace, so columns of figures line up |
| `theme::readout_large()` | The one big figure on a page |

### Colour

- Never hardcode a `Color32` on a page. Use `theme::good/warn/bad/text/muted/
  faint/line/track(ui)` — each resolves per theme. Hardcoded dark constants
  are what broke the light theme the first time.
- The accent is the Windows-generation colour and is **not** a fixed value.
  Always run it through `theme::accent_on(ui, accent)`.
- Chart series take `theme::series(ui, n)`, which wraps and darkens for light.
  Do not pick chart colours by hand, and do not use the semantic three
  (`good`/`warn`/`bad`) for a data series — a line must never be mistaken for
  a warning.

### Surfaces — three, and they mean different things

| Frame | Use |
|---|---|
| `theme::card(ui)` | Bordered panel for text, controls, lists |
| `theme::widget(ui)` | Flat, 14px radius. Analytics and chart cards — the things you look at first |
| `theme::row(ui)` | No fill, no border. Items in a list you scan down |

**A list of items gets ONE card containing rows, not one card per item.** Nine
bordered cards stacked down a page is thirty borders and no information. This
has been fixed twice.

---

### Page structure

`body()` already draws the page title and blurb from `Tab::title()` and
`Tab::blurb()`. Do not add a second heading or a second description at the top
of a page — that pair was duplicated on Diagnose and the description, being an
unwrapped label in a `horizontal`, pushed the run button off the edge.

Settings is a tabbed page: Appearance, AI connection, About, switched with
`chrome::segment`. Things read once belong there rather than in the sidebar.

## 3. The widget inventory

Check this list before building anything.

**Data display** — `meter` (progress bar, flat vivid fill, pill-rounded; the
value is derived from the fraction and right-aligned on the label line),
`stack` (one pill track split into butted segments), `sparkline` (trace with a
gradient wash and an end dot), `ring` (donut gauge with the figure in the
hole), `legend` (dot + name row), `sensor_chip`, `pill` (one-word status
badge).

**Controls** — `button` with `Weight::{Primary, Ghost, Danger}`, `segment`
(pill in a segmented control — use this for ALL tab switchers and mode
pickers), `toggle`, `icon_button`, `card_button` (whole card is the target,
hover lifts), `accordion` (the ONLY disclosure widget — never `ui.collapsing`
or `egui::CollapsingHeader`), and the `Pointer` trait, below.

**Structure** — `section_heading`, `icon_text`, `icon::*` (Segoe Fluent
glyphs; never type an emoji or a bare chevron — the system text fonts have no
glyph for `U+2304` and it renders as tofu).

### Every interaction ends in `.pointer()`

`chrome::Pointer` adds `.pointer()` to any `Response`. Put it before the
`.clicked()`:

```rust
if chrome::segment(ui, label, selected, accent).pointer().clicked() {
```

Windows uses the arrow for everything and leaves the visual design to say what
is clickable. That works for a button and does not work for the things this
app also makes interactive — whole cards, list rows, chart range chips,
accordion headers — so the cursor is the affordance and it is not optional.

It borrows rather than consumes, so a response can still be used afterwards,
and it is idempotent: the house widgets set it at their own return and a call
site may set it again without knowing which is which. It skips disabled
widgets, because a greyed-out control is not inviting a click.

This was applied across all 87 existing call sites at once. Do not reintroduce
the gap on a new page.

### Things that are banned because they look stock

`egui::ProgressBar`, `ui.collapsing`, `egui::CollapsingHeader`,
`ui.selectable_label` as a mode picker, `ui.separator()` where a layout should
do the work.

---

## 4. Text must never escape its container

This is the single most repeated bug in this codebase. It has been fixed in
the meter caption, the TreeSize rows, the card eyebrows, the chart header and
the reading-card caption.

- `painter.text()` **clips nothing** and does not tell the frame the text is
  there. Do not use it for anything that could be long. Lay out a galley
  against the width you actually have and paint that.
- `painter.layout(..., wrap_width)` **wraps**. If you reserved one row of
  height, a wrapped second line paints outside the rect, over whatever is
  below. For a single line use `LayoutJob::simple_singleline` with
  `wrap.max_rows = 1` and an `overflow_character`.
- Any `ui.label` holding a name, path, device, command or error that came from
  the filesystem, the registry or a subprocess gets `.truncate()` or `.wrap()`.
- A truncating label claims all remaining width, so lay out the right-aligned
  element **first**.
- In a four-column grid, assume every string is too long.

---

## 5. Threading

`egui` redraws on one thread. Anything slow on that thread freezes the window,
Windows greys it out and marks it Not Responding, and the user kills the app —
possibly mid-write.

Never call from a `fn(&mut self, ui: &mut Ui)` path: `Command::output()` or
`.status()`, `Receiver::recv()`, a filesystem walk, registry enumeration,
`thread::sleep`.

The established pattern, used by `treesize`, `diagnose`, the cleaner and the
backup jobs: spawn a worker, hold a `Receiver` in an `App` field, drain it with
`try_recv` in a `poll_*` method called from `update`, and
`ctx.request_repaint_after(...)` while it is still running.

Two refinements worth copying:

- **Report partial results.** `treesize` sends a usable tree every 600ms and
  `diagnose` reports per check, so the view fills in rather than appearing at
  the end. A long job behind a bare spinner feels broken.
- **Bound every subprocess.** Poll `try_wait` against a deadline and `kill()`
  on expiry. One hung command must not hang the whole sweep.

---

## 6. Destructive actions and subprocesses

- Every `Command` on Windows sets `CREATE_NO_WINDOW` (`0x0800_0000`) unless a
  visible console is the point — an interactive script needs a real console or
  it reaches its first prompt, finds no stdin and dies.
- Anything that deletes, rewrites the registry, resets a subsystem or runs
  third-party code is confirmed first. Mark it `destructive: true` on
  `tools::Action` and route it through `launch_action`, so a new entry cannot
  skip the prompt by being called from a different page.
- Needing administrator is a **different question** from being destructive.
  Plenty of read-only consoles need elevation.
- Degrade, never prompt: if an action needs elevation and the app is not
  elevated, report that as a result. Do not raise UAC from a background check.
- Never swallow a failure. `let _ = …` on a meaningful `Result` and a bare
  `Err(_) => {}` both mean the user is told something worked when it did not.

---

## 7. Comment style

Comments explain **why**, and what was tried before. They never restate what
the code does. Full sentences. Every public item gets a doc comment.

```rust
// A fill narrower than the bar is tall cannot be drawn as a rounded shape --
// the two end caps overlap and it renders as a dark smudge. Below that point
// the reading becomes a dot at the start of the scale: honest about being
// near zero, and visible.
```

Not: `// clamp the fill width`.

---

## 8. Before you hand work back

```
cargo fmt
cargo check
cargo clippy          # known-noisy here: collapsible_if, too_many_arguments,
                      # manual while-let, complex type. Everything else is real.
cargo test
```

The tree is rustfmt-clean and warning-free. Keep it that way.

**Compiling is not the bar for UI work.** A page can build perfectly and still
be unusable — nine stacked cards, a button pushed off the edge by an unwrapped
label, a bar that flashes before its chart arrives. All three shipped in one
day because the check was "does it build". If you changed the interface and
cannot run it, say so plainly in your hand-back instead of implying it works.

### Writing a patch as a subagent

Patches are applied by the main session, not by you. Write a Python script
that:

- uses `io.open(path, encoding='utf-8', newline='')` for every read and write,
  detecting CRLF vs LF per file and restoring it;
- asserts `old in text` before every replacement, with a message naming the
  site — and remember `'' in text` is always `True`, so guard the
  already-applied check with `assert not new or new not in text`;
- writes every file only at the very end, after all asserts pass, so a failure
  leaves the repo untouched;
- never puts a Python `\uXXXX` escape next to a Rust `\u{...}` escape. Build
  the Rust escape with `chr(92) + 'u{2026}'`. This has broken five scripts.

Capture every snippet from the current bytes on disk. Do not trust memory —
the file has probably been reformatted since you read it.
