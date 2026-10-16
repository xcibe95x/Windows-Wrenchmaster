# Windows Wrenchmaster

An offline Windows maintenance toolbox. Version 2.0.0, rewritten in Rust as a
single binary with no installer, no network and no telemetry.

It replaces a 2017 Visual Basic WinForms utility of the same name — one that
phoned a license server at startup, filled most of its tabs with buttons that
did nothing, and shipped a Clean button that walked `C:\` deleting every `.log`
file it could reach.

## Support

**Weekly support through the end of 2027.** Updates ship weekly for the whole
of 2027 — fixes, Windows compatibility work, and the occasional feature.

## What it does

- **Overview** — live processor, graphics, memory and drive readings with
  recent-history charts, and a health score with the reasons behind it.
- **Diagnose** — nine read-only checks: uptime, pending restart, Windows event
  log, device problems, virus protection, memory-diagnostic history, drive
  hardware health and endurance, filesystem dirty flags, and the Windows
  component store. Nothing here repairs, schedules or restarts anything.
- **Storage** — drive summary, a preview-first cleaner that measures before it
  deletes, a folder Size Explorer that fills in as it scans, and leftover-app
  detection.
- **Memory** — live use, what is holding it, and how to free it.
- **Tweaks and Privacy** — registry switches, each reversible from the row that
  set it, each showing the value actually read from the registry.
- **Troubleshoot, Repair and Tools** — Windows diagnostics and repair commands.
  Anything that changes the system says what it does and asks first.
- **Backup** — registry export and System Restore points, both on a worker so
  the window stays responsive.
- **Settings** — appearance with a Custom theme, a local MCP server connection
  discovered from Claude Desktop or the OpenAI Codex CLI, and build details.

## Support the project

Wrenchmaster is free and offline, and it stays that way — there is nothing to
unlock and nothing phoning home. If it saves you an afternoon, you can back the
weekly development on Patreon:

**[Support Wrenchmaster on Patreon](https://www.patreon.com/aboveconstraints)**

## Principles

- **Offline.** No telemetry, no update check, no license server. The single
  networked action downloads a third-party script and is gated behind a
  confirmation that says exactly that.
- **Read before write.** Anything that measures runs before anything that
  changes.
- **Nothing destructive without a prompt**, and every failure is surfaced
  rather than swallowed.
- **Runs unelevated.** Administrator rights are requested per action, not at
  startup. Actions that need them say so and stay disabled without them.

## Build

Requires a Rust toolchain (built against 1.94) and the MSVC target.

```
cargo build --release
```

The binary lands at `target/release/wrenchmaster.exe`. The release profile sets
`windows_subsystem = "windows"`, so it starts without a console; debug builds
keep one. `build.rs` embeds the icon, version resource and manifest, and warns
rather than failing if the resource compiler is unavailable.

```
cargo fmt
cargo clippy
cargo test
```

## Contributing

Read [AGENTS.md](AGENTS.md) first. It is the design system and working
agreement — spacing and type tokens, the widget inventory, the threading
pattern, and the rules for destructive actions.

## License

MIT.
