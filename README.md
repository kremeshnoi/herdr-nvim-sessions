# Nvim Sessions

Herdr plugin that shows every running Neovim session across all Herdr
workspaces in a sidebar pane, styled like Herdr's own agent list. Jump to any
editor with one key or a click.

```text
 ○ api · server
   src/handlers/auth.rs
 ● web · ui
   src/components/Header.tsx [+2]
 ● dotfiles · nvim
   init.lua
```

## Install

Install from GitHub. Herdr builds the plugin with Cargo during install:

```sh
herdr plugin install kremeshnoi/herdr-nvim-sessions
```

Or clone, build, and link locally while developing:

```sh
git clone https://github.com/kremeshnoi/herdr-nvim-sessions.git
cd herdr-nvim-sessions
cargo build --release
herdr plugin link .
```

`herdr plugin link` does not run build commands, so rebuild with
`cargo build --release` after local changes.

## Toggle with a keybind

Add this keybind to Herdr's `config.toml`, then run `herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+shift+v"
type = "plugin_action"
command = "kremeshnoi.nvim-sessions.toggle"
description = "nvim sessions"
```

Manual controls:

```sh
herdr plugin action invoke kremeshnoi.nvim-sessions.toggle
herdr plugin pane open --plugin kremeshnoi.nvim-sessions --entrypoint sidebar --placement split --direction right
```

The toggle action opens the sidebar as a narrow split to the right of the
focused pane, and closes it when it is already open.

## Usage

| Key | Action |
| --- | --- |
| `j` / `↓` | Next session |
| `k` / `↑` | Previous session |
| `Enter` / `l` / click | Focus that Neovim pane |
| `r` | Refresh now |
| `q` / `Esc` | Close the sidebar |

Each entry shows the workspace and tab on the first row and the current file
on the second row. The status dot uses Herdr's default `dots` indicators and
your theme colors:

| Dot | Herdr state | Neovim state |
| --- | --- | --- |
| `○` | idle | All buffers saved |
| `●` | working | Unsaved buffers, count shown as `[+N]` |
| `●` | blocked | Neovim is not responding, for example waiting on a `Press ENTER` prompt |
| `·` | unknown | No RPC socket found for that Neovim |

## How it works

- Subscribes to Herdr pane, tab, and workspace events and refreshes as soon as
  one arrives. A 1.5 second background poll also picks up Neovim starting,
  quitting, switching files, and saving.
- Reads `session.snapshot` and `pane.process_info` over the Herdr socket to find
  panes whose foreground process is `nvim`.
- Finds each Neovim's default RPC socket (`$TMPDIR/nvim.$USER/*/nvim.<pid>.0`
  on macOS, `$XDG_RUNTIME_DIR/nvim.<pid>.0` on Linux) and asks it for the
  current file and unsaved buffer count with `nvim --remote-expr`.
- Reads `[theme]` from Herdr's `config.toml` (`HERDR_CONFIG_PATH` is honored)
  and uses the matching built-in palette from `palettes.json` plus any
  `[theme.custom]` overrides.
- Jumps with a single `pane.focus` request, which also switches workspace and
  tab.

`target/release/herdr-nvim-sessions list` prints the detected sessions as JSON
lines, which helps when debugging.

## Notes

- Requires a Rust toolchain with edition 2024 support (Rust 1.85 or newer) to
  build, and `nvim` on `PATH` at runtime.
- Neovim instances started with a custom `--listen` address are listed but show
  the `unknown` state.
- Stores the open sidebar pane id under `HERDR_PLUGIN_STATE_DIR`.
- Herdr plugins cannot draw into the native sidebar yet, so this is a regular
  split pane that stays in the tab where it was opened.

## License

MIT
