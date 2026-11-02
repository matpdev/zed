# Zed custom (matpdev/zed, branch `custom`)

Personal Zed build on top of the latest stable release tag, with:

- **Hover readability**: smaller, muted section headings; tighter spacing; no redundant `---`
  before headings; `hover_popover_font_size` and `hover_popover_max_width` settings.
- **Hover actions**: `hover_popover_actions` setting with documentation buttons (URL or command,
  filtered by symbol/language/hover text), web search and "Perguntar à IA"
  (`agent::AskAboutSymbol`, opens an agent thread and sends the question).
- **Doxygen cleanup** for C/C++ hovers: `@ref`, `@sa`, `%word`, HTML links, unknown commands
  (`@errors`, `@thread_safety`, …) turned into readable Markdown.

## Scripts

| Command | What it does |
|---|---|
| `script/install-custom` | Release build (official profile) and install into `~/.local/share/zed-custom`. |
| `script/install-custom --list` | List installed releases (`*` = active). |
| `script/install-custom --use <release>` | Switch to another installed release (rollback). |
| `script/build-custom --dev [--test]` | Debug build for iterating (`target/debug/zed`). |
| `script/run-dev` | Run the debug build with live hover style tuning from `hover-style.json`. |

The last 3 releases are kept (`KEEP=n` to change). Settings, extensions and sessions live in
`~/.config/zed` and `~/.local/share/zed` and are never touched.

## Updating to a new Zed release

```sh
cd ~/Projects/zed
git fetch upstream --tags
git switch custom
git rebase v1.27.0          # the new stable tag (or: git merge v1.27.0)
# fix conflicts if any, then:
script/build-custom --dev --test   # optional: check that it builds and tests pass
script/install-custom
git push --force-with-lease origin custom   # rebase rewrites history
```

If the new release breaks something, `script/install-custom --use <previous release>` brings the
old build back instantly.
