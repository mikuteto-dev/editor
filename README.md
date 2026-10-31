# Editor

A lightweight code editor for macOS, built with [GPUI](https://www.gpui.rs).

## Features

- Syntax highlighting for mainstream languages (tree-sitter)
- File tree with file-type icons and git status, following `.gitignore` and changes on disk
- Preview tabs, and the open tabs of each folder restored when you return to it
- Project-wide search and find/replace in a file
- Git: changed lines in the editor, changed files with diffs, commit history, file history
- Markdown and SVG preview
- Images and video in tabs
- Recent folders, command palette, dark themes
- Opens source files from Finder

## Shortcuts

| Key | Action |
|---|---|
| `Cmd+Shift+P` | Command palette |
| `Cmd+O` / `Ctrl+R` | Open folder / recent folder |
| `Cmd+S` / `Cmd+W` | Save / close tab |
| `Cmd+Shift+]` / `Cmd+Shift+[` | Next / previous tab |
| `Cmd+B` | Toggle sidebar |
| `Cmd+Shift+E` / `Cmd+Shift+F` / `Ctrl+Shift+G` | Explorer / search / git |
| `Cmd+F` | Find in file |
| `Cmd+Shift+V` | Toggle Markdown or SVG preview |
| `Cmd+K Cmd+T` | Theme |
| `Space` | Play or pause video |

## Build

Requires macOS 13 or later, Rust, and Xcode 26 (for the app icon).

```sh
cargo run --release -- path/to/folder
```

To install `~/Applications/Editor.app` and make it the default app for source files:

```sh
script/bundle.sh
swift script/set-default.swift ~/Applications/Editor.app
```

## License

GPL-3.0-or-later. Bundled third-party code and assets keep their own licenses:

- `vendor/gpui-component`: [gpui-component](https://github.com/longbridge/gpui-component), Apache-2.0, with small changes marked `editor patch:`
- `assets/themes`: from gpui-component, Apache-2.0
- `assets/icons`: [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme), MIT
- `assets/ui`: [Lucide](https://lucide.dev), ISC
