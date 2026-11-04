# Grammars

The file viewer's syntax highlighting compiles these tree-sitter grammars into the app, each from its crate in `crates/runner-app/Cargo.toml`. The list lives in three places that change together: those dependencies, `GRAMMARS` in `crates/runner-app/src/surfaces/files/highlight.rs`, and this table. Each `LICENSE-*` file is the grammar's licence at the tagged release its crate was published from; `highlight.rs` embeds every one in the app, and the Windows installer also installs them under `grammars\`.

| Grammar | Crate | Source | Licence |
| --- | --- | --- | --- |
| tree-sitter (the runtime and highlighter) | `tree-sitter`, `tree-sitter-highlight` 0.27.1 | [tree-sitter/tree-sitter](https://github.com/tree-sitter/tree-sitter) | MIT, `LICENSE-tree-sitter` |
| Bash | `tree-sitter-bash` 0.25.1 | [tree-sitter/tree-sitter-bash](https://github.com/tree-sitter/tree-sitter-bash) | MIT, `LICENSE-bash` |
| C | `tree-sitter-c` 0.24.2 | [tree-sitter/tree-sitter-c](https://github.com/tree-sitter/tree-sitter-c) | MIT, `LICENSE-c` |
| C# | `tree-sitter-c-sharp` 0.23.5 | [tree-sitter/tree-sitter-c-sharp](https://github.com/tree-sitter/tree-sitter-c-sharp) | MIT, `LICENSE-c-sharp` |
| C++ | `tree-sitter-cpp` 0.23.4 | [tree-sitter/tree-sitter-cpp](https://github.com/tree-sitter/tree-sitter-cpp) | MIT, `LICENSE-cpp` |
| CSS | `tree-sitter-css` 0.25.0 | [tree-sitter/tree-sitter-css](https://github.com/tree-sitter/tree-sitter-css) | MIT, `LICENSE-css` |
| Go | `tree-sitter-go` 0.25.0 | [tree-sitter/tree-sitter-go](https://github.com/tree-sitter/tree-sitter-go) | MIT, `LICENSE-go` |
| HTML | `tree-sitter-html` 0.23.2 | [tree-sitter/tree-sitter-html](https://github.com/tree-sitter/tree-sitter-html) | MIT, `LICENSE-html` |
| Java | `tree-sitter-java` 0.23.5 | [tree-sitter/tree-sitter-java](https://github.com/tree-sitter/tree-sitter-java) | MIT, `LICENSE-java` |
| JavaScript and JSX | `tree-sitter-javascript` 0.25.0 | [tree-sitter/tree-sitter-javascript](https://github.com/tree-sitter/tree-sitter-javascript) | MIT, `LICENSE-javascript` |
| JSON | `tree-sitter-json` 0.24.8 | [tree-sitter/tree-sitter-json](https://github.com/tree-sitter/tree-sitter-json) | MIT, `LICENSE-json` |
| Kotlin | `tree-sitter-kotlin-ng` 1.1.0 | [tree-sitter-grammars/tree-sitter-kotlin](https://github.com/tree-sitter-grammars/tree-sitter-kotlin) | MIT, `LICENSE-kotlin` |
| Lua | `tree-sitter-lua` 0.5.0 | [tree-sitter-grammars/tree-sitter-lua](https://github.com/tree-sitter-grammars/tree-sitter-lua) | MIT, `LICENSE-lua` |
| Make | `tree-sitter-make` 1.1.1 | [tree-sitter-grammars/tree-sitter-make](https://github.com/tree-sitter-grammars/tree-sitter-make) | MIT, `LICENSE-make` |
| Markdown and its inline grammar | `tree-sitter-md` 0.5.4 | [tree-sitter-grammars/tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown) | MIT, `LICENSE-markdown` |
| PHP | `tree-sitter-php` 0.25.1 | [tree-sitter/tree-sitter-php](https://github.com/tree-sitter/tree-sitter-php) | MIT, `LICENSE-php` |
| Python | `tree-sitter-python` 0.25.0 | [tree-sitter/tree-sitter-python](https://github.com/tree-sitter/tree-sitter-python) | MIT, `LICENSE-python` |
| Ruby | `tree-sitter-ruby` 0.23.1 | [tree-sitter/tree-sitter-ruby](https://github.com/tree-sitter/tree-sitter-ruby) | MIT, `LICENSE-ruby` |
| Rust | `tree-sitter-rust` 0.24.2 | [tree-sitter/tree-sitter-rust](https://github.com/tree-sitter/tree-sitter-rust) | MIT, `LICENSE-rust` |
| SQL | `tree-sitter-sequel` 0.3.11 | [derekstride/tree-sitter-sql](https://github.com/derekstride/tree-sitter-sql) | MIT, `LICENSE-sql` |
| Swift | `tree-sitter-swift` 0.7.4 | [alex-pinkus/tree-sitter-swift](https://github.com/alex-pinkus/tree-sitter-swift) | MIT, `LICENSE-swift` |
| TOML | `tree-sitter-toml-ng` 0.7.0 | [tree-sitter-grammars/tree-sitter-toml](https://github.com/tree-sitter-grammars/tree-sitter-toml) | MIT, `LICENSE-toml` |
| TypeScript and TSX | `tree-sitter-typescript` 0.23.2 | [tree-sitter/tree-sitter-typescript](https://github.com/tree-sitter/tree-sitter-typescript) | MIT, `LICENSE-typescript` |
| YAML | `tree-sitter-yaml` 0.7.2 | [tree-sitter-grammars/tree-sitter-yaml](https://github.com/tree-sitter-grammars/tree-sitter-yaml) | MIT, `LICENSE-yaml` |

`tree-sitter-kotlin-ng` ships no highlight query, so `kotlin/highlights.scm` is Runner's own, written against that grammar's node types and under Runner's MIT licence. Every other query is the one its crate ships. TypeScript, TSX and C++ extend the JavaScript and C queries, as their grammars' own configurations do.

Dockerfile is not bundled: its crate, `tree-sitter-dockerfile` 0.2.0, still builds against tree-sitter 0.20, so Dockerfiles show as plain text.
