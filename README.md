# Editor

A lightweight code editor for macOS, built with [GPUI](https://www.gpui.rs).

## Features

- Syntax highlighting (tree-sitter), including Vue, Svelte and Astro with embedded JavaScript/TypeScript and CSS
- File tree with file-type icons and git status, following `.gitignore` and changes on disk
- Preview tabs, and the open tabs of each folder restored when you return to it
- Project-wide search and find/replace in a file
- Git: stage with checkboxes, commit and amend, fetch, pull, push, create and switch branches, stash, discard changes, conflicts, diffs, history, and who last changed the current line
- Dependency licenses on manifest entries and import lines, with conflicts against your own license marked, and a dependency graph that opens onto the problems: Cargo; npm, pnpm, Yarn, Bun and Deno; pip, uv, Poetry, Pipenv, PDM, Hatch, conda and pixi; Go; Bundler; Composer; NuGet; Maven and Gradle; SwiftPM, CocoaPods and Carthage; LuaRocks; vcpkg and Conan
- Markdown and SVG preview, with TeX mathematics, Mermaid diagrams and vectorized box drawings
- Images and video in tabs
- Recent folders, command palette, dark themes
- Opens source files from Finder

## Markdown preview

- Mathematics: `$…$`, `$$…$$`, `\(…\)`, `\[…\]`, GitHub's dollar/backtick notation, and `math`, `tex` or `latex` fences. Rendering uses RaTeX with embedded fonts: LaTeX/AMS environments, local macros and mhchem (`\ce`, `\pu`). Macros and equation counters in Markdown remain local to each formula; complete `.tex` documents use the TeX preview below.
- Mermaid fences render locally with `mermaid-rs-renderer`, including flowcharts, sequences, classes, states, ER, pie, Gantt, Git graphs, mindmaps, timelines, journeys, XY and Sankey charts. `init` applies themes, theme variables, flowchart spacing and the renderer's per-diagram settings. Unsupported settings produce an error instead of being silently ignored. This native implementation is not fully compatible with Mermaid.js.
- Multi-line Unicode box drawings and directory trees become SVG strokes, arrows and labels, rather than monospaced characters. They are recognized in ordinary paragraphs and plain/text/plaintext/ascii/tree/diagram fences. The original grid is preserved, not semantically rearranged; ordinary source-code fences are left alone.

Graphics render in the background without a browser, external commands or network requests. Invalid or oversized input keeps its source visible. Preview examples: [`tests/fixtures/markdown.md`](tests/fixtures/markdown.md).

## TeX documents

Open a `.tex` or `.ltx` file and use `Cmd+Shift+V` to open its preview in a separate tab. Select **LuaLaTeX**, **XeLaTeX** or **pLaTeX** in the preview toolbar; pLaTeX uses dvipdfmx. `% !TeX program = lualatex` (or `xelatex` / `platex`) selects the initial engine; without a directive the default is LuaLaTeX.

Install MacTeX/TeX Live locally, including `latexmk`, the selected engine, converters, and the packages/fonts your document uses. Editor does not install TeX or fetch missing packages. Compilation uses the current main-file buffer and saved included files. `latexmk` resolves reruns for references; **Build** rebuilds after edits or included-file changes. PDF pages render natively inside Editor with previous/next controls.

Opening a source file does not compile it. Preview builds run in a macOS sandbox: project and system/TeX resources are readable, writes stay in a private temporary directory, network and shell escapes are disabled, and user/project `.latexmkrc` files are not executed. Generated files never overwrite project files. Builds can be cancelled and time out after 120 seconds; source/PDF limits are 4/64 MiB. Packages requiring shell escape or custom build hooks are intentionally not executed.

Examples: [`tests/fixtures/tex`](tests/fixtures/tex). With a complete TeX installation, run the real-engine and sandbox checks with `cargo test tex::tests::real_ -- --ignored`.

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
| `Cmd+Shift+V` | Switch between source and its separate Markdown, SVG or TeX preview tab |
| `Cmd+Enter` | Commit, in the Git panel |
| `Cmd+K Cmd+T` | Theme |
| `Space` | Play or pause video |

## Build

Requires macOS 13 or later, Rust, and Xcode 26 (for the app icon).

```sh
script/icons.sh
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
- `vendor/gpui-base`: [gpui-base](https://github.com/longbridge/gpui-kit), Apache-2.0, with small changes marked `editor patch:`
- `assets/themes`: from gpui-component, Apache-2.0
- `assets/icons`: [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme), MIT
- `assets/ui`: [Lucide](https://lucide.dev), ISC
