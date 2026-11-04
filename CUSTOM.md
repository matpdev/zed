# Zed custom (matpdev/zed, branch `custom`)

Personal Zed build on top of upstream `main` (merged in regularly), with:

- **Hover readability**: smaller, muted section headings; tighter spacing; no redundant `---`
  before headings; `hover_popover_font_size` and `hover_popover_max_width` settings.
- **Hover actions**: `hover_popover_actions` setting with documentation buttons (URL or command,
  filtered by symbol/language/hover text), web search and "Perguntar à IA"
  (`agent::AskAboutSymbol`, opens an agent thread and sends the question).
- **`disabled_capabilities`** per language server (`"lsp": {"server": {"disabled_capabilities":
  ["completion", "hover"]}}`): Zed ignores those features of the server, at startup and on
  dynamic registration. Used to keep only the diagnostics of `shader-language-server` for GLSL.
- **Doxygen cleanup** for C/C++ hovers: `@ref`, `@sa`, `%word`, HTML links, unknown commands
  (`@errors`, `@thread_safety`, …) turned into readable Markdown.

## Scripts

| Command | What it does |
|---|---|
| `script/install-custom` | Download the latest GitHub release of the fork and install it into `~/.local/share/zed-custom`. |
| `script/install-custom --local` | Build locally with the official release profile (~12 GB in `target/release`) and install. |
| `script/install-custom --file <tar.gz>` | Install a tarball made by `script/package-custom`. |
| `script/install-custom --list` | List installed releases (`*` = active). |
| `script/install-custom --use <release>` | Switch to another installed release (rollback). |
| `script/build-custom --dev [--test]` | Debug build for iterating (`target/debug/zed`). |
| `script/run-dev` | Run the debug build with live hover style tuning from `hover-style.json`. |

Release builds (Linux x86_64 tarball, macOS Apple Silicon and Intel `.dmg`) run on GitHub Actions
(`.github/workflows/custom_release.yml`) on every push to `custom` (or manually from the Actions tab), take about 1–1.5 h, and are published as a
`custom-<version>-<commit>` release; the 5 newest are kept. Locally only the debug build is kept.
The macOS app is signed ad hoc and not notarized: after copying it to Applications, run
`xattr -dr com.apple.quarantine /Applications/Zed.app` once. A failed macOS build does not block
the release; it is published with whatever built.

The last 3 installed releases are kept (`KEEP=n` to change). Settings, extensions and sessions
live in `~/.config/zed` and `~/.local/share/zed` and are never touched.

## Updating from upstream

```sh
cd ~/Projects/zed
git switch main && git pull upstream main
git switch custom
git merge main              # fix conflicts if any
script/build-custom --dev --test             # optional: check that it builds and tests pass
git push origin custom      # starts the release build
# ~1–1.5 h later, when the "Custom release" workflow is green:
script/install-custom
```

If the new release breaks something, `script/install-custom --use <previous release>` brings the
old build back instantly.
