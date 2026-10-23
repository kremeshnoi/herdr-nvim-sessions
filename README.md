# Nvim Sessions

Herdr plugin that shows every running Neovim session across all Herdr
workspaces in a sidebar pane, drawn like Herdr's own spaces and agents lists.
Jump to any editor with one key or a click.

## Showcase

<img width="1920" height="1080" alt="{BB8E2C55-4DAB-46FE-A7D9-D8F3C36948F3}" src="https://github.com/user-attachments/assets/35fe38bf-3a36-4652-8d52-a643bfc54e3b" />
<img width="1920" height="1080" alt="{0FD26C33-C2E4-4620-B561-0F8ECF87EAF7}" src="https://github.com/user-attachments/assets/1998348a-b1c3-4801-8f44-19ebde9def8b" />

## Requirements

- Herdr 0.9.3 or newer on Linux or macOS
- Rust 1.85 or newer (`cargo` on `PATH`) to build during install
- `nvim` on `PATH` at runtime

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

The toggle action opens the sidebar as a 36 column split to the right of the
focused pane and closes it when it is already open in the current tab. Toggling
from another tab moves the sidebar there.

## Usage

| Key | Action |
| --- | --- |
| `j` / `↓` | Next session |
| `k` / `↑` | Previous session |
| `Enter` / `l` / click | Focus that Neovim pane |
| `r` | Refresh now |
| `q` / `Esc` | Close the sidebar |

Each entry shows the workspace on the first row, plus the tab when the
workspace has more than one, and the current file on the second row. The
highlighted row follows the Neovim pane you last focused and moves with
`j`/`k`; it uses the same `active_row_bg`, bold `text`, and `mauve` accents as
the active workspace in Herdr's sidebar.

Status indicators use Herdr's default `dots` style and your theme colors:

| Dot | Herdr state | Neovim state |
| --- | --- | --- |
| `○` | idle | All buffers saved |
| `●` | working | Unsaved buffers, count shown as `[+N]` |
| `●` | blocked | Neovim is not responding, for example waiting on a `Press ENTER` prompt |
| `·` | unknown | No RPC socket found for that Neovim |

The pane title uses the Nerd Font Neovim glyph (`nf-linux-neovim`, U+F36F), so
it needs a Nerd Font in your terminal.

## Configuration

Set the sidebar width in columns in the plugin config file. Print its directory
with `herdr plugin config-dir kremeshnoi.nvim-sessions`:

```toml
width = 36
```

## How it works

- Subscribes to Herdr pane, tab, and workspace events and refreshes as soon as
  one arrives. A 1.5 second background poll also picks up Neovim starting,
  quitting, switching files, and saving.
- Reads `session.snapshot` and `pane.process_info` over the Herdr socket to find
  panes whose foreground process is `nvim`.
- Finds each Neovim's default RPC socket (`$TMPDIR/nvim.$USER/*/nvim.<pid>.0`
  on macOS, `$XDG_RUNTIME_DIR/nvim.<pid>.0` on Linux) and asks it for the
  current file and unsaved buffer count with `nvim --remote-expr`.
- Reads `[theme]` from Herdr's `config.toml`
  (`HERDR_CONFIG_PATH` is honored) and uses the matching built-in palette from
  `palettes.json`, generated from Herdr's own themes, plus any `[theme.custom]`
  overrides.
- Jumps with a single `pane.focus` request, which also switches workspace and
  tab.

`target/release/herdr-nvim-sessions list` prints the detected sessions as JSON
lines, which helps when debugging.

## Notes

- Neovim instances started with a custom `--listen` address are listed but show
  the `unknown` state.
- Stores the open sidebar pane id under `HERDR_PLUGIN_STATE_DIR` and only ever
  closes panes owned by this plugin.
- Herdr plugins cannot draw into the native sidebar yet, so this is a regular
  split pane.

## License

MIT
