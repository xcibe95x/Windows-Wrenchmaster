//! The MCP server lists that Claude Desktop and the OpenAI Codex CLI already
//! keep on this machine.
//!
//! "Automatic for Claude and OpenAI" means this and nothing more: both
//! products record their local stdio MCP servers in a file under the user
//! profile, so the servers someone has already set up can be offered as a
//! list instead of being retyped into two text fields. Nothing here reaches a
//! network service and nothing here wants an API key -- the app has no HTTP
//! client, and the connection this feeds is still a local process over stdio.
//!
//! The TOML side is hand-parsed rather than pulled in with the `toml` crate.
//! Taking that dependency would add serde derive, toml_edit and winnow to a
//! build that currently carries `serde_json` alone, all to read two keys out
//! of one table. The rest of this codebase already hand-parses its own ini in
//! `appearance` and picks JSON fields apart by hand in `malware_lookup`, so
//! this matches what is here. The parser deliberately understands only the
//! subset Codex writes: a `[mcp_servers.<name>]` header, a quoted `command`,
//! and an `args` array of quoted strings. Anything it does not recognise is
//! skipped rather than guessed at.
//!
//! Every function returns a list, never an error. These are other programs'
//! files: absent on most machines, reshaped between releases, and briefly
//! half-written while the other program saves. A settings page must not fail
//! over any of that, so a missing, unreadable or malformed file yields an
//! empty list, and one malformed entry is dropped on its own.

use std::path::PathBuf;

/// Which other program a discovered server was read out of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    ClaudeDesktop,
    OpenAiCodex,
}

impl Source {
    /// The word on the segmented control. Deliberately the vendor rather than
    /// the product, because the control answers "whose servers?" and nobody
    /// thinks of these as the "Claude Desktop" and "Codex" lists.
    pub fn label(self) -> &'static str {
        match self {
            Source::ClaudeDesktop => "Claude",
            Source::OpenAiCodex => "OpenAI",
        }
    }

    /// Where the list came from, for the caption under an empty result.
    /// Naming the file is the fastest way for someone to work out why their
    /// server is missing, which is the only question an empty list provokes.
    pub fn origin(self) -> &'static str {
        match self {
            Source::ClaudeDesktop => r"%APPDATA%\Claude\claude_desktop_config.json",
            Source::OpenAiCodex => r"%USERPROFILE%\.codex\config.toml",
        }
    }
}

/// One stdio MCP server found in another program's configuration.
#[derive(Clone, Debug)]
pub struct Discovered {
    pub source: Source,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

impl Discovered {
    /// The command and its arguments as one line, for the caption under the
    /// server's name. Arguments are joined unquoted on purpose: this is a
    /// caption to recognise a server by, not something to paste into a shell,
    /// and quoting rules would only add noise to the usual `npx -y ...` case.
    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            return self.command.clone();
        }
        format!("{} {}", self.command, self.args.join(" "))
    }
}

/// Everything configured on this machine, Claude Desktop first.
///
/// This touches the disk, so callers are expected to run it once and keep the
/// result rather than calling it while painting a frame.
pub fn discover() -> Vec<Discovered> {
    let mut found = claude_desktop();
    found.extend(openai_codex());
    found
}

/// Servers from `%APPDATA%\Claude\claude_desktop_config.json`.
///
/// The shape is `{"mcpServers": {"<name>": {"command": ..., "args": [...]}}}`.
/// Entries without a `command` are skipped rather than shown as a row that
/// cannot be connected: Claude Desktop also accepts remote server entries,
/// which this app has no transport for.
pub fn claude_desktop() -> Vec<Discovered> {
    let Some(path) = crate::util::expand(r"%APPDATA%\Claude\claude_desktop_config.json") else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(servers) = root.get("mcpServers").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (name, entry) in servers {
        let Some(command) = entry.get("command").and_then(|value| value.as_str()) else {
            continue;
        };
        if command.trim().is_empty() {
            continue;
        }
        let args = entry
            .get("args")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        found.push(Discovered {
            source: Source::ClaudeDesktop,
            name: name.clone(),
            command: command.trim().to_string(),
            args,
        });
    }
    found
}

/// Servers from `%USERPROFILE%\.codex\config.toml`.
///
/// Codex spells the table `mcp_servers`, in snake_case, which is the one
/// detail most likely to be mistyped when copying from the Claude Desktop
/// shape. Nested tables such as `[mcp_servers.<name>.env]` are ignored, and
/// so is any server declared with a `url` instead of a `command`, for the
/// same reason as above.
pub fn openai_codex() -> Vec<Discovered> {
    let Some(path) = codex_config_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_codex(&text)
}

/// Codex consults `CODEX_HOME` before falling back to the profile directory,
/// and honouring that is nearly free, so a relocated install is still found.
fn codex_config_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if !home.trim().is_empty() {
            return Some(PathBuf::from(home).join("config.toml"));
        }
    }
    crate::util::expand(r"%USERPROFILE%\.codex\config.toml")
}

/// The hand-written TOML subset parser described in the module comment.
///
/// It is separate from [`openai_codex`] so it can be reasoned about without a
/// file on disk: everything it does is a pure transform of the text, which is
/// also what makes the cases below testable.
fn parse_codex(text: &str) -> Vec<Discovered> {
    let mut found: Vec<Discovered> = Vec::new();
    let mut current: Option<String> = None;
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = strip_comment(line).trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            // An array-of-tables header starts with "[[" and never names a
            // server here, so it simply fails to match and clears `current`.
            let header = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
            current = table_server_name(header);
            if let Some(name) = &current {
                if !found.iter().any(|entry| &entry.name == name) {
                    found.push(Discovered {
                        source: Source::OpenAiCodex,
                        name: name.clone(),
                        command: String::new(),
                        args: Vec::new(),
                    });
                }
            }
            continue;
        }
        let Some(name) = current.clone() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = unquote(key.trim());
        let mut value = value.trim().to_string();
        if key == "args" {
            // An args array is routinely written across several lines, so
            // keep pulling lines in until the brackets balance. The only
            // thing that can run out is the file, which ends the loop.
            while value.matches('[').count() > value.matches(']').count() {
                let Some(next) = lines.next() else { break };
                value.push(' ');
                value.push_str(strip_comment(next).trim());
            }
        }
        let Some(target) = found.iter_mut().find(|entry| entry.name == name) else {
            continue;
        };
        match key.as_str() {
            "command" => target.command = unquote(&value),
            "args" => target.args = parse_array(&value),
            _ => {}
        }
    }
    found.retain(|entry| !entry.command.trim().is_empty());
    found
}

/// `mcp_servers.<name>` and nothing else: a deeper path is a subtable such as
/// `env`, and any other table belongs to a Codex feature this app never reads.
fn table_server_name(header: &str) -> Option<String> {
    let parts = split_dotted(header);
    if parts.len() != 2 || parts[0] != "mcp_servers" {
        return None;
    }
    let name = parts[1].clone();
    if name.is_empty() { None } else { Some(name) }
}

/// Split a dotted TOML key, leaving dots inside a quoted segment alone.
fn split_dotted(header: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in header.chars() {
        match quote {
            Some(open) => {
                current.push(ch);
                if ch == open {
                    quote = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            None if ch == '.' => {
                parts.push(unquote(current.trim()));
                current.clear();
            }
            None => current.push(ch),
        }
    }
    parts.push(unquote(current.trim()));
    parts
}

/// Everything before an unquoted `#`. Windows commands are full of paths and
/// the occasional `#`, and splitting on the first one truncated real values.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some('"') if ch == '\\' => escaped = true,
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

/// A quoted TOML scalar as its text. A basic string gets the handful of
/// escapes that turn up in a Windows command path; a literal string is taken
/// as written, which is what single quotes mean in TOML. An unquoted value
/// comes back unchanged so a bare key still compares correctly.
fn unquote(value: &str) -> String {
    let value = value.trim();
    let Some(first) = value.chars().next() else {
        return String::new();
    };
    let Some(last) = value.chars().last() else {
        return String::new();
    };
    if value.len() < 2 || first != last {
        return value.to_string();
    }
    let inner = &value[1..value.len() - 1];
    if first == '\'' {
        return inner.to_string();
    }
    if first != '"' {
        return value.to_string();
    }
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            out.push(match ch {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        out.push(ch);
    }
    out
}

/// The quoted strings inside a TOML array, in order. Anything that is not a
/// string -- a number, a table, a nested array -- is dropped, because an
/// argument that is not a string is not something this app could hand to a
/// process anyway.
fn parse_array(value: &str) -> Vec<String> {
    let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match quote {
            Some('"') if ch == '\\' => {
                current.push(ch);
                escaped = true;
            }
            Some(open) if ch == open => {
                current.push(ch);
                quote = None;
            }
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => {
                current.push(ch);
                quote = Some(ch);
            }
            None if ch == ',' => {
                push_array_item(&mut items, &current);
                current.clear();
            }
            None => current.push(ch),
        }
    }
    push_array_item(&mut items, &current);
    items
}

fn push_array_item(items: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.len() < 2 {
        return;
    }
    if !trimmed.starts_with('"') && !trimmed.starts_with('\'') {
        return;
    }
    items.push(unquote(trimmed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_codex_table() {
        let text = "[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"server\"]\n";
        let found = parse_codex(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "docs");
        assert_eq!(found[0].command, "npx");
        assert_eq!(found[0].args, vec!["-y".to_string(), "server".to_string()]);
    }

    #[test]
    fn ignores_subtables_and_commandless_servers() {
        let text = concat!(
            "[mcp_servers.remote]\n",
            "url = \"https://example.invalid\"\n",
            "[mcp_servers.local]\n",
            "command = \"a.exe\"\n",
            "[mcp_servers.local.env]\n",
            "K = \"v\"\n",
        );
        let found = parse_codex(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "local");
    }

    #[test]
    fn keeps_a_hash_inside_a_quoted_command() {
        let text = "[mcp_servers.odd]\ncommand = \"C:\\\\tools\\\\a#b.exe\" # note\n";
        let found = parse_codex(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, r"C:\tools\a#b.exe");
    }

    #[test]
    fn survives_rubbish() {
        assert!(parse_codex("not toml at all ][").is_empty());
        assert!(parse_codex("").is_empty());
    }
}
